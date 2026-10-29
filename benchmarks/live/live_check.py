#!/usr/bin/env python3
"""Check uc against a live Starknet network, block by block, with the chain as the oracle.

    python3 benchmarks/live/live_check.py --uc target/release/uc [--backfill 50] [--port 8790]
        [--rpc https://api.cartridge.gg/x/starknet/mainnet] [--max-blocks 0] [--out <dir>]

For every confirmed block:
- every v3 transaction: `uc utils tx-hash` must reproduce the transaction hash;
- every v3 declare: `uc class-hash --casm` must reproduce the class hash and the compiled class
  hash the sequencer accepted, and `uc audit` must finish on the new class;
- one sampled invoke: `uc trace` must report the same call tree as `starknet_traceTransaction`,
  and every function name it decodes must hash to the call's selector;
- the sampled sender: `uc call balance_of` on STRK at that block must equal `starknet_call`.

Every disagreement or uc failure is written to <out>/repro/ with the inputs. Results stream to
<out>/checks.jsonl and, with --port, to a page at http://127.0.0.1:<port>/. With --max-blocks N
the run stops after N blocks and exits 1 if uc disagreed with the chain or failed.
"""
import argparse
import json
import socket
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
from collections import Counter, deque
from concurrent.futures import ThreadPoolExecutor
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

# IPv6 is broken on some hosts and urllib has no Happy Eyeballs: resolve IPv4 only.
_getaddrinfo = socket.getaddrinfo
socket.getaddrinfo = lambda host, port, family=0, *a, **k: _getaddrinfo(host, port, socket.AF_INET, *a, **k)

STRK = 0x04718F5A0FC34CC1AF16A1CDEE98FFB20C31F5CD61D6AB07201858F4287C938D
CHECKED_TYPES = {"INVOKE", "DECLARE", "DEPLOY_ACCOUNT"}

# Keccak-256 (the pre-SHA3 padding Starknet uses), so selectors are checked without uc's code.
_RC = [0x1, 0x8082, 0x800000000000808A, 0x8000000080008000, 0x808B, 0x80000001, 0x8000000080008081,
       0x8000000000008009, 0x8A, 0x88, 0x80008009, 0x8000000A, 0x8000808B, 0x800000000000008B,
       0x8000000000008089, 0x8000000000008003, 0x8000000000008002, 0x8000000000000080, 0x800A,
       0x800000008000000A, 0x8000000080008081, 0x8000000000008080, 0x80000001, 0x8000000080008008]
_ROT = [[0, 36, 3, 41, 18], [1, 44, 10, 45, 2], [62, 6, 43, 15, 61], [28, 55, 25, 21, 56], [27, 20, 39, 8, 14]]
_M = (1 << 64) - 1


def _rol(v, n):
    return ((v << n) | (v >> (64 - n))) & _M if n else v


def keccak256(data: bytes) -> bytes:
    rate = 136
    msg = bytearray(data)
    pad = rate - len(msg) % rate
    msg += bytes(pad)
    msg[len(data)] ^= 0x01
    msg[-1] ^= 0x80
    a = [[0] * 5 for _ in range(5)]
    for off in range(0, len(msg), rate):
        for i in range(rate // 8):
            a[i % 5][i // 5] ^= int.from_bytes(msg[off + 8 * i: off + 8 * i + 8], "little")
        for rc in _RC:
            c = [a[x][0] ^ a[x][1] ^ a[x][2] ^ a[x][3] ^ a[x][4] for x in range(5)]
            d = [c[(x - 1) % 5] ^ _rol(c[(x + 1) % 5], 1) for x in range(5)]
            b = [[0] * 5 for _ in range(5)]
            for x in range(5):
                for y in range(5):
                    b[y][(2 * x + 3 * y) % 5] = _rol(a[x][y] ^ d[x], _ROT[x][y])
            a = [[b[x][y] ^ (~b[(x + 1) % 5][y] & b[(x + 2) % 5][y]) for y in range(5)] for x in range(5)]
            a[0][0] ^= rc
    return b"".join(a[i % 5][i // 5].to_bytes(8, "little") for i in range(4))


def sn_keccak(name: str) -> int:
    return int.from_bytes(keccak256(name.encode()), "big") & ((1 << 250) - 1)


def felt(value) -> int:
    return int(value, 16) if isinstance(value, str) else int(value)


class Rpc:
    def __init__(self, url):
        self.url = url
        self.retries = 0

    def __call__(self, method, params, attempts=8):
        body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
        for attempt in range(attempts):
            try:
                req = urllib.request.Request(self.url, data=body, headers={"content-type": "application/json"})
                reply = json.loads(urllib.request.urlopen(req, timeout=60).read())
                break
            except (urllib.error.URLError, TimeoutError, ConnectionError) as err:
                if attempt == attempts - 1 or (isinstance(err, urllib.error.HTTPError) and err.code not in (429, 500, 502, 503, 504)):
                    raise
                self.retries += 1
                time.sleep(min(2 ** attempt, 20))
        if "error" in reply:
            raise RuntimeError(f"{method}: {reply['error']}")
        return reply["result"]


class Checker:
    def __init__(self, args):
        self.args = args
        self.rpc = Rpc(args.rpc)
        self.uc = str(Path(args.uc).resolve())
        self.out = Path(args.out).resolve()
        (self.out / "repro").mkdir(parents=True, exist_ok=True)
        (self.out / "classes").mkdir(exist_ok=True)
        self.log = (self.out / "checks.jsonl").open("a")
        self.chain_id = self.rpc("starknet_chainId", [])
        # Client-side proven invokes hash their proof facts, which spec 0.10 returns only on request.
        self.block_flags = {"response_flags": ["INCLUDE_PROOF_FACTS"]}
        try:
            self.rpc("starknet_getBlockWithTxs", {"block_id": "latest", **self.block_flags}, attempts=2)
        except Exception:  # noqa: BLE001 - older specs reject the flag
            self.block_flags = {}
        self.lock = threading.Lock()
        self.counts = {k: Counter() for k in ("tx_hash", "class_hash", "casm_hash", "audit", "trace", "selector", "call")}
        self.blocks = deque(maxlen=240)
        self.disagreements = deque(maxlen=200)
        self.declares = deque(maxlen=100)
        self.head = self.last_block = None
        self.started = time.time()
        version = subprocess.run([self.uc, "--version"], capture_output=True, text=True).stdout.strip()
        self.uc_version = version or "uc"

    # -- bookkeeping -------------------------------------------------------------------------
    def record(self, kind, block, item, outcome, detail=None, repro=None, ms=None):
        """outcome: agree | disagree | skip | uc_error."""
        entry = {"t": round(time.time(), 3), "kind": kind, "block": block, "id": item, "outcome": outcome}
        if detail is not None:
            entry["detail"] = detail
        if ms is not None:
            entry["ms"] = round(ms, 1)
        if outcome in ("disagree", "uc_error") and repro is not None:
            path = self.out / "repro" / f"{kind}-{str(item)[:18]}-{block}.json"
            path.write_text(json.dumps(repro, indent=1))
            entry["repro"] = path.name
        with self.lock:
            self.counts[kind][outcome] += 1
            self.log.write(json.dumps(entry) + "\n")
            self.log.flush()
            if outcome in ("disagree", "uc_error"):
                self.disagreements.appendleft(entry)
        return outcome

    def run_uc(self, argv, stdin=None, timeout=180):
        started = time.time()
        try:
            p = subprocess.run([self.uc, *argv], input=stdin, capture_output=True, text=True, timeout=timeout)
        except subprocess.TimeoutExpired:
            return None, f"timed out after {timeout} s", (time.time() - started) * 1000
        ms = (time.time() - started) * 1000
        if p.returncode != 0:
            return None, (p.stderr or p.stdout)[-1200:], ms
        text = p.stdout
        try:
            return json.loads(text[text.index("{"):]), None, ms
        except ValueError:
            return None, f"no JSON on stdout: {text[-400:]}", ms

    # -- checks ------------------------------------------------------------------------------
    def check_tx_hash(self, block, tx):
        tx_hash = tx["transaction_hash"]
        if tx.get("type") not in CHECKED_TYPES or tx.get("version") != "0x3":
            return self.record("tx_hash", block, tx_hash, "skip", f"{tx.get('type')} {tx.get('version')}")
        got, err, ms = self.run_uc(["utils", "tx-hash", "--tx", "-", "--chain-id", self.chain_id, "--json"], json.dumps(tx))
        if err:
            return self.record("tx_hash", block, tx_hash, "uc_error", err, {"tx": tx, "error": err}, ms)
        ok = felt(got["transaction_hash"]) == felt(tx_hash)
        return self.record("tx_hash", block, tx_hash, "agree" if ok else "disagree",
                           None if ok else f"uc {got['transaction_hash']}", {"tx": tx, "uc": got}, ms)

    def check_declare(self, block, tx):
        class_hash, casm_hash = tx["class_hash"], tx["compiled_class_hash"]
        cls = self.rpc("starknet_getClass", {"block_id": {"block_number": block}, "class_hash": class_hash})
        path = self.out / "classes" / f"{class_hash}.json"
        path.write_text(json.dumps(cls))
        info = {"t": time.time(), "block": block, "class_hash": class_hash, "sierra": cls.get("sierra_program", ["", "", ""])[:3]}
        got, err, ms = self.run_uc(["class-hash", str(path), "--casm", "--json"])
        if err:
            self.record("class_hash", block, class_hash, "uc_error", err, {"class_file": path.name, "error": err}, ms)
            info["class_hash_ok"] = info["casm_hash_ok"] = "uc error"
        else:
            for kind, want, key in (("class_hash", class_hash, "class_hash"), ("casm_hash", casm_hash, "compiled_class_hash")):
                ok = got.get(key) is not None and felt(got[key]) == felt(want)
                self.record(kind, block, class_hash, "agree" if ok else "disagree", None if ok else f"uc {got.get(key)} chain {want}",
                            {"class_file": path.name, "uc": got, "chain": {"class_hash": class_hash, "compiled_class_hash": casm_hash}}, ms)
                info[f"{kind}_ok"] = ok
        if not self.args.no_audit:
            report, err, ms = self.run_uc(["audit", class_hash, "--url", self.args.rpc, "--json"], timeout=300)
            if err:
                self.record("audit", block, class_hash, "uc_error", err, {"class_hash": class_hash, "error": err}, ms)
                info["audit"] = "uc error"
            else:
                severities = Counter(f.get("severity") for f in report.get("caracal_findings") or [])
                self.record("audit", block, class_hash, "agree", dict(severities), ms=ms)
                info["audit"] = {"findings": dict(severities), "ms": round(ms), "functions": len(report.get("symbolic_proofs") or []),
                                 "counterexamples": report.get("counterexample_count")}
        with self.lock:
            self.declares.appendleft(info)

    def check_trace(self, block, tx):
        tx_hash = tx["transaction_hash"]
        raw = self.rpc("starknet_traceTransaction", {"transaction_hash": tx_hash})
        got, err, ms = self.run_uc(["trace", tx_hash, "--url", self.args.rpc, "--json"])
        if err:
            return self.record("trace", block, tx_hash, "uc_error", err, {"tx_hash": tx_hash, "error": err}, ms)
        execute = raw.get("execute_invocation") or {}
        want = []

        def walk(inv, depth):
            want.append((felt(inv["contract_address"]), felt(inv["entry_point_selector"]), felt(inv.get("caller_address", "0x0")),
                         felt(inv.get("class_hash", "0x0")), depth, len(inv.get("events") or []), len(inv.get("calldata") or [])))
            for sub in inv.get("calls") or []:
                walk(sub, depth + 1)

        if "contract_address" in execute:
            walk(execute, 0)
        have = [(felt(c.get("contract_address") or "0x0"), felt(c.get("selector") or "0x0"), felt(c.get("caller_address") or "0x0"),
                 felt(c.get("class_hash") or "0x0"), c.get("depth"), c.get("events"), c.get("calldata_len")) for c in got.get("calls") or []]
        reverted_chain = "revert_reason" in execute
        ok = have == want and bool(got.get("reverted")) == reverted_chain
        detail = None
        if not ok:
            first = next((i for i, (a, b) in enumerate(zip(have, want)) if a != b), min(len(have), len(want)))
            detail = f"{len(have)} calls vs {len(want)} on chain; first difference at call {first}"
        self.record("trace", block, tx_hash, "agree" if ok else "disagree", detail, {"tx_hash": tx_hash, "uc": got.get("calls"), "chain_trace": raw}, ms)
        for c in got.get("calls") or []:
            name = c.get("function")
            if not name or name.startswith("0x"):
                continue
            ok = sn_keccak(name.split("::")[-1]) == felt(c["selector"])
            self.record("selector", block, f"{tx_hash}:{name}", "agree" if ok else "disagree",
                        None if ok else f"`{name}` does not hash to {c['selector']}", {"call": c})

    def check_call(self, block, sender):
        selector = sn_keccak("balance_of")
        want = self.rpc("starknet_call", {"request": {"contract_address": hex(STRK), "entry_point_selector": hex(selector), "calldata": [sender]},
                                          "block_id": {"block_number": block}})
        got, err, ms = self.run_uc(["call", "--contract-address", hex(STRK), "--function", "balance_of", "--calldata", sender,
                                    "--block-id", str(block), "--url", self.args.rpc, "--json"])
        if err:
            return self.record("call", block, sender, "uc_error", err, {"sender": sender, "error": err}, ms)
        ok = [felt(v) for v in got.get("response") or []] == [felt(v) for v in want] and felt(got.get("entry_point_selector", "0x0")) == selector
        self.record("call", block, sender, "agree" if ok else "disagree", None if ok else f"uc {got.get('response')} chain {want}",
                    {"sender": sender, "uc": got, "chain": want}, ms)

    def check_block(self, number, block):
        txs = block["transactions"]
        before = sum(sum(c[o] for o in ("disagree", "uc_error")) for c in self.counts.values())
        for tx in txs:
            self.check_tx_hash(number, tx)
            if tx.get("type") == "DECLARE" and tx.get("version") == "0x3":
                try:
                    self.check_declare(number, tx)
                except Exception as err:  # noqa: BLE001 - an RPC failure must not stop the run
                    self.record("class_hash", number, tx["transaction_hash"], "skip", f"rpc: {err}")
        invokes = [t for t in txs if t.get("type") == "INVOKE" and t.get("version") == "0x3"]
        if invokes and number % self.args.sample_every == 0:
            pick = invokes[number % len(invokes)]
            for check, arg in ((self.check_trace, pick), (self.check_call, pick["sender_address"])):
                try:
                    check(number, arg)
                except Exception as err:  # noqa: BLE001
                    self.record(check.__name__[6:], number, pick["transaction_hash"], "skip", f"rpc: {err}")
        after = sum(sum(c[o] for o in ("disagree", "uc_error")) for c in self.counts.values())
        with self.lock:
            self.blocks.append({"n": number, "txs": len(txs), "bad": after - before, "t": time.time()})
            self.last_block = number

    # -- loop --------------------------------------------------------------------------------
    def run(self):
        self.head = self.rpc("starknet_blockNumber", [])
        start = self.head - self.args.backfill
        fetched = 0
        with ThreadPoolExecutor(2) as pool:
            fetch = lambda n: self.rpc("starknet_getBlockWithTxs", {"block_id": {"block_number": n}, **self.block_flags})
            nxt = start
            while True:
                self.head = self.rpc("starknet_blockNumber", [])
                if nxt > self.head:
                    time.sleep(1.5)
                    continue
                batch = list(range(nxt, min(self.head, nxt + 8) + 1))
                for number, block in zip(batch, pool.map(fetch, batch)):
                    self.check_block(number, block)
                    fetched += 1
                    if self.args.max_blocks and fetched >= self.args.max_blocks:
                        return
                nxt = batch[-1] + 1

    def state(self):
        with self.lock:
            return {"network": self.chain_id, "rpc": self.args.rpc, "uc": self.uc_version, "head": self.head,
                    "last_block": self.last_block, "uptime_s": round(time.time() - self.started), "rpc_retries": self.rpc.retries,
                    "proof_facts": bool(self.block_flags), "counts": {k: dict(v) for k, v in self.counts.items()}, "blocks": list(self.blocks),
                    "disagreements": list(self.disagreements)[:50], "declares": list(self.declares)[:30],
                    "log": str(self.out / "checks.jsonl")}

    def failed(self):
        return any(c["disagree"] or c["uc_error"] for c in self.counts.values())


def serve(checker, port):
    page = Path(__file__).parent / "live_check.html"

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):  # noqa: N802
            if self.path.startswith("/state.json"):
                body, kind = json.dumps(checker.state()).encode(), "application/json"
            elif self.path.startswith("/repro/"):
                name = Path(self.path.split("?")[0]).name
                path = checker.out / "repro" / name
                if not path.is_file():
                    self.send_error(404)
                    return
                body, kind = path.read_bytes(), "application/json"
            else:
                body, kind = page.read_bytes(), "text/html; charset=utf-8"
            self.send_response(200)
            self.send_header("content-type", kind)
            self.send_header("cache-control", "no-store")
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *_):
            pass

    server = ThreadingHTTPServer(("127.0.0.1", port), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    print(f"live page: http://127.0.0.1:{port}/", flush=True)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--uc", required=True)
    ap.add_argument("--rpc", default="https://api.cartridge.gg/x/starknet/mainnet/rpc/v0_10")
    ap.add_argument("--out", default=f"benchmarks/results/live-{time.strftime('%Y%m%d-%H%M%S')}")
    ap.add_argument("--backfill", type=int, default=50, help="start this many blocks behind the head")
    ap.add_argument("--max-blocks", type=int, default=0, help="stop after this many blocks (0: run until stopped)")
    ap.add_argument("--sample-every", type=int, default=1, help="trace and call one invoke every N blocks")
    ap.add_argument("--no-audit", action="store_true", help="skip `uc audit` on declared classes")
    ap.add_argument("--port", type=int, default=0, help="serve the live page on 127.0.0.1:<port>")
    args = ap.parse_args()
    checker = Checker(args)
    if args.port:
        serve(checker, args.port)
    try:
        checker.run()
    except KeyboardInterrupt:
        pass
    summary = checker.state()
    (checker.out / "summary.json").write_text(json.dumps(summary, indent=1))
    print(json.dumps({"last_block": summary["last_block"], "counts": summary["counts"]}))
    return 1 if checker.failed() else 0


if __name__ == "__main__":
    sys.exit(main())

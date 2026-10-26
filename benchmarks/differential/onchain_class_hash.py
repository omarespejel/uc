#!/usr/bin/env python3
"""Class hashes against the chain: `uc class-hash` on classes fetched from a live network.

    benchmarks/differential/onchain_class_hash.py --uc target/release/uc --out <dir> \
        [--rpc https://api.cartridge.gg/x/starknet/mainnet] [--blocks 30] [--max-classes 200]

Collects contract addresses from recent blocks (senders and called contracts), resolves each
one's class hash with `starknet_getClassHashAt`, fetches the class with `starknet_getClass`,
and checks that `uc class-hash` on the fetched JSON reproduces the chain's hash. Legacy Cairo 0
classes are skipped (uc hashes Sierra classes). Writes <out>/summary.json; exit 1 on a mismatch.
"""
import argparse
import json
import subprocess
import sys
import time
import urllib.error
import urllib.request
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path


def rpc(url, method, params, attempts=8):
    """One JSON-RPC call; public endpoints rate-limit (HTTP 429), so back off and retry."""
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
    for attempt in range(attempts):
        try:
            req = urllib.request.Request(url, data=body, headers={"content-type": "application/json"})
            reply = json.loads(urllib.request.urlopen(req, timeout=120).read())
            break
        except urllib.error.HTTPError as err:
            if err.code != 429 or attempt == attempts - 1:
                raise
            time.sleep(min(2 ** attempt, 30))
    if "error" in reply:
        raise RuntimeError(reply["error"])
    return reply["result"]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--uc", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--rpc", default="https://api.cartridge.gg/x/starknet/mainnet")
    ap.add_argument("--blocks", type=int, default=30)
    ap.add_argument("--max-classes", type=int, default=200)
    args = ap.parse_args()
    uc = str(Path(args.uc).resolve())
    out = Path(args.out).resolve()
    out.mkdir(parents=True, exist_ok=False)
    latest = rpc(args.rpc, "starknet_blockNumber", [])
    addresses = set()
    for b in range(latest - args.blocks, latest):
        for tx in rpc(args.rpc, "starknet_getBlockWithTxs", {"block_id": {"block_number": b}})["transactions"]:
            if tx.get("sender_address"):
                addresses.add(tx["sender_address"])
            calldata = tx.get("calldata") or []
            # Account `__execute__` calldata: [n_calls, to, selector, len, ...]; take the targets.
            i, n = 1, int(calldata[0], 16) if calldata else 0
            for _ in range(min(n, 8)):
                if i + 2 >= len(calldata):
                    break
                addresses.add(calldata[i])
                i += 3 + int(calldata[i + 2], 16)

    def class_of(addr):
        try:
            return addr, rpc(args.rpc, "starknet_getClassHashAt", {"block_id": "latest", "contract_address": addr})
        except Exception:  # noqa: BLE001
            return addr, None

    with ThreadPoolExecutor(4) as pool:
        by_class = {}
        for addr, ch in pool.map(class_of, sorted(addresses)):
            if ch:
                by_class.setdefault(int(ch, 16), addr)
    classes = sorted(by_class)[: args.max_classes]

    def check(ch):
        cls = rpc(args.rpc, "starknet_getClass", {"block_id": "latest", "class_hash": hex(ch)})
        if "sierra_program" not in cls:
            return {"class_hash": hex(ch), "skipped": "legacy Cairo 0 class"}
        path = out / f"{hex(ch)}.json"
        path.write_text(json.dumps(cls))
        p = subprocess.run([uc, "class-hash", str(path), "--json"], capture_output=True, text=True)
        try:
            got = json.loads(p.stdout[p.stdout.index("{"):]).get("class_hash")
        except ValueError:
            got = None
        ok = got is not None and int(got, 16) == ch
        return {"class_hash": hex(ch), "address": by_class[ch], "uc": got, "ok": ok, "sierra_version": cls["sierra_program"][:3], "err": None if ok else (p.stderr or p.stdout)[-300:]}

    with ThreadPoolExecutor(3) as pool:
        results = list(pool.map(check, classes))
    checked = [r for r in results if "ok" in r]
    summary = {"rpc": args.rpc, "latest_block": latest, "addresses": len(addresses), "classes": len(classes), "checked": len(checked), "skipped_legacy": sum(1 for r in results if "skipped" in r), "mismatches": [r for r in checked if not r["ok"]], "results": results}
    (out / "summary.json").write_text(json.dumps(summary, indent=2))
    print(json.dumps({k: (len(v) if isinstance(v, list) else v) for k, v in summary.items() if k != "results"}))
    return 1 if summary["mismatches"] else 0


if __name__ == "__main__":
    sys.exit(main())

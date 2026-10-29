#!/usr/bin/env python3
"""Devnet differential: one scenario against `uc devnet` and starknet-devnet.

    benchmarks/differential/devnet_diff.py --uc target/release/uc --out <dir> \
        --uc-devnet http://127.0.0.1:5062/rpc --reference http://127.0.0.1:5061/rpc

Declares a fresh Counter class with `uc declare`, deploys it with `uc deploy`, invokes and calls
it, then probes the nodes directly over JSON-RPC: receipts, blocks, events (paged), storage,
nonces, state updates, traces, fee estimates, simulation, and the edge cases a client meets:
a reverted call, a stale nonce, a bad signature, resource bounds too low, a fee the account
cannot pay, unknown transactions, contracts, classes and blocks, and a failing call. Raw
transactions are hashed with `uc utils tx-hash` (checked against mainnet and Sepolia by
`benchmarks/live`) and signed by this script's own STARK-curve ECDSA, so the reference node
independently checks every signature. Each probe is reported as `same`, `same_error` (both
nodes return the same error code), `same_shape` (equal keys, values differ, as with fees),
`differ`, or `unsupported` on one side. Writes <out>/summary.json; exit 1 on `differ`.
"""
import argparse
import hashlib
import json
import random
import subprocess
import sys
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "live"))
from live_check import sn_keccak  # noqa: E402

# STARK curve (y^2 = x^3 + x + BETA over P) and its ECDSA, as starknet-crypto signs.
P = 2**251 + 17 * 2**192 + 1
BETA = 0x6F21413EFBE40DE150E596D72F7A8C5609AD26C15C915C1F4CDFCB99CEE9E89
N = 0x800000000000010FFFFFFFFFFFFFFFFB781126DCAE7B2321E66A241ADC64D2F
G = (0x1EF15C18599971B7BECED415A40F0C7DEACFD9B0D1819E03D723D8BC943CFCA,
     0x5668060AA49730B7BE4801DF46EC62DE53ECD11ABE43A32873000C36E8DC1F)


def ec_add(a, b):
    if a is None or b is None:
        return a or b
    if a[0] == b[0]:
        if (a[1] + b[1]) % P == 0:
            return None
        m = (3 * a[0] * a[0] + 1) * pow(2 * a[1], -1, P) % P
    else:
        m = (b[1] - a[1]) * pow(b[0] - a[0], -1, P) % P
    x = (m * m - a[0] - b[0]) % P
    return x, (m * (a[0] - x) - a[1]) % P


def ec_mul(k, point):
    acc = None
    while k:
        if k & 1:
            acc = ec_add(acc, point)
        point, k = ec_add(point, point), k >> 1
    return acc


def sign(msg_hash, private_key):
    k = int.from_bytes(hashlib.sha256(f"{private_key}:{msg_hash}".encode()).digest(), "big")
    while True:
        k = k % (N - 1) + 1
        r = ec_mul(k, G)[0] % N
        s = pow(k, -1, N) * (msg_hash + r * private_key) % N
        if 0 < r < 2**251 and 0 < s < N and pow(s, -1, N) < 2**251:
            return r, s
        k += 1


def verify(msg_hash, public_key_x, r, s):
    y2 = (pow(public_key_x, 3, P) + public_key_x + BETA) % P
    if pow(y2, (P - 1) // 2, P) != 1:
        return False
    y = tonelli(y2)
    w = pow(s, -1, N)
    for public in ((public_key_x, y), (public_key_x, P - y)):
        point = ec_add(ec_mul(msg_hash * w % N, G), ec_mul(r * w % N, public))
        if point and point[0] % N == r:
            return True
    return False


def tonelli(n):
    """Square root modulo P (P - 1 is divisible by a large power of two)."""
    q, e = P - 1, 0
    while q % 2 == 0:
        q, e = q // 2, e + 1
    z = 3
    while pow(z, (P - 1) // 2, P) != P - 1:
        z += 1
    m, c, t, r = e, pow(z, q, P), pow(n, q, P), pow(n, (q + 1) // 2, P)
    while t != 1:
        i, t2 = 0, t
        while t2 != 1:
            t2, i = t2 * t2 % P, i + 1
        b = pow(c, 1 << (m - i - 1), P)
        m, c, t, r = i, b * b % P, t * b * b % P, r * b % P
    return r


def get_selector_from_name(name):
    return sn_keccak(name)

COUNTER = """#[starknet::interface]
pub trait ICounter<T> { fn increase(ref self: T, by: u64); fn get(self: @T) -> u64; fn tag(self: @T) -> felt252; }
#[starknet::contract]
pub mod Counter {
    use starknet::storage::{StoragePointerReadAccess, StoragePointerWriteAccess};
    #[storage]
    struct Storage { value: u64 }
    #[event]
    #[derive(Drop, starknet::Event)]
    pub enum Event { Increased: Increased }
    #[derive(Drop, starknet::Event)]
    pub struct Increased { pub by: u64, pub value: u64 }
    #[constructor]
    fn constructor(ref self: ContractState, start: u64) { self.value.write(start); }
    #[abi(embed_v0)]
    impl CounterImpl of super::ICounter<ContractState> {
        fn increase(ref self: ContractState, by: u64) { let v = self.value.read() + by; self.value.write(v); self.emit(Increased { by, value: v }); }
        fn get(self: @ContractState) -> u64 { self.value.read() }
        fn tag(self: @ContractState) -> felt252 { TAG }
    }
}
"""


def rpc(url, method, params):
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
    req = urllib.request.Request(url, data=body, headers={"content-type": "application/json"})
    reply = json.loads(urllib.request.urlopen(req, timeout=120).read())
    return ("err", reply["error"]) if "error" in reply else ("ok", reply["result"])


def uc_json(uc, args, cwd):
    p = subprocess.run([uc, *args, "--json"], capture_output=True, text=True, cwd=cwd)
    try:
        return json.loads(p.stdout[p.stdout.index("{"):])
    except ValueError:
        return {"error": (p.stderr or p.stdout)[-600:]}


UC = None


def raw_invoke(url, account, calls, nonce, signature_override=None, bounds_override=None):
    """An INVOKE V3 with bounds from the node's estimate, hashed by uc and signed here."""
    calldata = [len(calls)]
    for to, selector, args in calls:
        calldata += [to, selector, len(args), *args]
    sender = int(account["address"], 16)
    unsigned = {"type": "INVOKE", "version": "0x3", "sender_address": hex(sender), "calldata": [hex(x) for x in calldata], "nonce": hex(nonce),
                "signature": [], "tip": "0x0", "paymaster_data": [], "account_deployment_data": [], "nonce_data_availability_mode": "L1", "fee_data_availability_mode": "L1",
                "resource_bounds": {k: {"max_amount": "0x0", "max_price_per_unit": "0x0"} for k in ("l1_gas", "l2_gas", "l1_data_gas")}}
    kind, est = rpc(url, "starknet_estimateFee", {"request": [unsigned], "simulation_flags": ["SKIP_VALIDATE"], "block_id": "latest"})
    est = est[0] if kind == "ok" else {}

    def bound(consumed, price, floor):
        return {"max_amount": hex(max(int(est.get(consumed, "0x0"), 16) * 2, floor)),
                "max_price_per_unit": hex(max(int(est.get(price, "0x0"), 16) * 2, 1))}

    unsigned["resource_bounds"] = bounds_override or {"l1_gas": bound("l1_gas_consumed", "l1_gas_price", 0),
                                                      "l2_gas": bound("l2_gas_consumed", "l2_gas_price", 2_000_000),
                                                      "l1_data_gas": bound("l1_data_gas_consumed", "l1_data_gas_price", 1000)}
    chain = rpc(url, "starknet_chainId", [])[1]
    p = subprocess.run([UC, "utils", "tx-hash", "--tx", "-", "--chain-id", chain, "--json"], input=json.dumps(unsigned), capture_output=True, text=True)
    tx_hash = int(json.loads(p.stdout)["transaction_hash"], 16)
    r, s = sign(tx_hash, int(account["private_key"], 16))
    assert verify(tx_hash, int(account["public_key"], 16), r, s), "the script's own signature does not verify"
    tx = dict(unsigned, signature=[hex(x) for x in (signature_override or [r, s])])
    return rpc(url, "starknet_addInvokeTransaction", {"invoke_transaction": tx}), tx_hash


def scenario(uc, url, pkg):
    out = {}
    accounts = rpc(url, "devnet_getPredeployedAccounts", {})[1]
    acc = accounts[0]
    auth = ["--url", url, "--account-address", acc["address"], "--private-key", acc["private_key"]]
    d = uc_json(uc, ["declare", "-c", "Counter", *auth, "--wait"], pkg)
    out["declare"] = {"class_hash": d.get("class_hash"), "error": d.get("error")}
    ch = d.get("class_hash")
    dp = uc_json(uc, ["deploy", "-g", ch or "0x0", "-s", "0x7", "--constructor-calldata", "40", *auth, "--wait"], pkg)
    addr = dp.get("contract_address")
    out["deploy"] = {"contract_address": addr, "error": dp.get("error")}
    inv = uc_json(uc, ["invoke", "-a", addr or "0x0", "-f", "increase", "-c", "2", *auth, "--wait"], pkg)
    txh = inv.get("transaction_hash")
    out["invoke"] = {"ok": bool(txh), "error": inv.get("error")}
    call = uc_json(uc, ["call", "-a", addr or "0x0", "-f", "get", "--url", url], pkg)
    out["call_get"] = call.get("result") or call.get("response") or call.get("error")
    out["receipt"] = rpc(url, "starknet_getTransactionReceipt", {"transaction_hash": txh or "0x0"})
    out["tx"] = rpc(url, "starknet_getTransactionByHash", {"transaction_hash": txh or "0x0"})
    out["status"] = rpc(url, "starknet_getTransactionStatus", {"transaction_hash": txh or "0x0"})
    out["trace"] = rpc(url, "starknet_traceTransaction", {"transaction_hash": txh or "0x0"})
    out["block"] = rpc(url, "starknet_getBlockWithTxHashes", {"block_id": "latest"})
    out["state_update"] = rpc(url, "starknet_getStateUpdate", {"block_id": "latest"})
    out["storage_value"] = rpc(url, "starknet_getStorageAt", {"contract_address": addr or "0x0", "key": hex(get_selector_from_name("value")), "block_id": "latest"})
    out["events"] = rpc(url, "starknet_getEvents", {"filter": {"address": addr or "0x0", "chunk_size": 10, "from_block": {"block_number": 0}, "to_block": "latest"}})
    out["nonce"] = rpc(url, "starknet_getNonce", {"block_id": "latest", "contract_address": acc["address"]})
    out["class_at"] = rpc(url, "starknet_getClassHashAt", {"block_id": "latest", "contract_address": addr or "0x0"})
    nonce = int(out["nonce"][1], 16) if out["nonce"][0] == "ok" else 0
    inc = [(int(addr or "0x0", 16), get_selector_from_name("increase"), [5])]
    probe_tx = {"type": "INVOKE", "version": "0x3", "sender_address": acc["address"], "calldata": [hex(x) for x in [1, int(addr or "0x0", 16), get_selector_from_name("increase"), 1, 5]], "nonce": hex(nonce), "signature": [], "tip": "0x0", "paymaster_data": [], "account_deployment_data": [], "nonce_data_availability_mode": "L1", "fee_data_availability_mode": "L1", "resource_bounds": {k: {"max_amount": "0x0", "max_price_per_unit": "0x0"} for k in ("l1_gas", "l2_gas", "l1_data_gas")}}
    out["estimate_fee"] = rpc(url, "starknet_estimateFee", {"request": [probe_tx], "simulation_flags": ["SKIP_VALIDATE"], "block_id": "latest"})
    out["simulate"] = rpc(url, "starknet_simulateTransactions", {"block_id": "latest", "transactions": [probe_tx], "simulation_flags": ["SKIP_VALIDATE", "SKIP_FEE_CHARGE"]})
    out["stale_nonce"] = raw_invoke(url, acc, inc, max(nonce - 1, 0))[0]
    out["bad_signature"] = raw_invoke(url, acc, inc, nonce, signature_override=[1, 2])[0]
    good, _ = raw_invoke(url, acc, inc, nonce)
    out["raw_invoke"] = good
    if good[0] == "ok":
        out["raw_invoke_receipt"] = rpc(url, "starknet_getTransactionReceipt", {"transaction_hash": good[1]["transaction_hash"]})
    reverted, _ = raw_invoke(url, acc, [(int(addr or "0x0", 16), get_selector_from_name("no_such_entry_point"), [])], nonce + 1)
    out["reverted"] = reverted
    if reverted[0] == "ok":
        out["reverted_receipt"] = rpc(url, "starknet_getTransactionReceipt", {"transaction_hash": reverted[1]["transaction_hash"]})
    nonce = int(rpc(url, "starknet_getNonce", {"block_id": "latest", "contract_address": acc["address"]})[1], 16)
    tiny = {k: {"max_amount": "0x1", "max_price_per_unit": "0x1"} for k in ("l1_gas", "l2_gas", "l1_data_gas")}
    out["bounds_too_low"] = raw_invoke(url, acc, inc, nonce, bounds_override=tiny)[0]
    huge = dict(tiny, l2_gas={"max_amount": hex(10**9), "max_price_per_unit": hex(10**30)})
    out["fee_unaffordable"] = raw_invoke(url, acc, inc, nonce, bounds_override=huge)[0]
    unknown = "0x123456789abcdef"
    out["unknown_receipt"] = rpc(url, "starknet_getTransactionReceipt", {"transaction_hash": unknown})
    out["unknown_status"] = rpc(url, "starknet_getTransactionStatus", {"transaction_hash": unknown})
    out["unknown_class_at"] = rpc(url, "starknet_getClassHashAt", {"block_id": "latest", "contract_address": unknown})
    out["unknown_storage"] = rpc(url, "starknet_getStorageAt", {"contract_address": unknown, "key": "0x1", "block_id": "latest"})
    out["unknown_nonce"] = rpc(url, "starknet_getNonce", {"block_id": "latest", "contract_address": unknown})
    out["unknown_class"] = rpc(url, "starknet_getClass", {"block_id": "latest", "class_hash": unknown})
    out["future_block"] = rpc(url, "starknet_getBlockWithTxHashes", {"block_id": {"block_number": 10**9}})
    out["call_missing_entry"] = rpc(url, "starknet_call", {"request": {"contract_address": addr or "0x0", "entry_point_selector": hex(get_selector_from_name("no_such_entry_point")), "calldata": []}, "block_id": "latest"})
    out["estimate_stale_nonce"] = rpc(url, "starknet_estimateFee", {"request": [dict(probe_tx, nonce="0x0")], "simulation_flags": [], "block_id": "latest"})
    page1 = rpc(url, "starknet_getEvents", {"filter": {"from_block": {"block_number": 0}, "to_block": "latest", "chunk_size": 1}})
    out["events_page_1"] = page1
    token = page1[1].get("continuation_token") if page1[0] == "ok" else None
    if token:
        out["events_page_2"] = rpc(url, "starknet_getEvents", {"filter": {"from_block": {"block_number": 0}, "to_block": "latest", "chunk_size": 1, "continuation_token": token}})
    out["block_hash_and_number"] = rpc(url, "starknet_blockHashAndNumber", [])
    out["block_with_receipts"] = rpc(url, "starknet_getBlockWithReceipts", {"block_id": "latest"})
    return out


def shape(v):
    if isinstance(v, dict):
        return {k: shape(x) for k, x in sorted(v.items())}
    if isinstance(v, list):
        return [shape(v[0])] if v else []
    return type(v).__name__


def classify(a, b):
    if a == b:
        return "same"
    if all(isinstance(x, (list, tuple)) and len(x) == 2 and x[0] == "err" for x in (a, b)):
        return "same_error" if a[1].get("code") == b[1].get("code") else "differ"
    if isinstance(a, (list, tuple)) and isinstance(b, (list, tuple)) and len(a) == 2 and len(b) == 2 and a[0] != b[0]:
        return "unsupported_uc" if a[0] == "err" else "unsupported_reference"
    return "same_shape" if shape(a) == shape(b) else "differ"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--uc", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--uc-devnet", required=True)
    ap.add_argument("--reference", required=True)
    args = ap.parse_args()
    global UC
    uc = UC = str(Path(args.uc).resolve())
    out = Path(args.out).resolve()
    out.mkdir(parents=True, exist_ok=False)
    pkg = out / "pkg"
    (pkg / "src").mkdir(parents=True)
    (pkg / "Scarb.toml").write_text('[package]\nname = "counter"\nversion = "0.1.0"\nedition = "2024_07"\n\n[dependencies]\nstarknet = "2.18.0"\n\n[[target.starknet-contract]]\nsierra = true\ncasm = true\n')
    (pkg / "src" / "lib.cairo").write_text(COUNTER.replace("TAG", str(random.SystemRandom().randrange(1, 2**120))))
    results = {"uc": scenario(uc, args.uc_devnet, pkg), "reference": scenario(uc, args.reference, pkg)}
    verdicts = {k: classify(results["uc"].get(k), results["reference"].get(k)) for k in results["reference"]}
    (out / "summary.json").write_text(json.dumps({"verdicts": verdicts, "results": results}, indent=2, default=str))
    print(json.dumps(verdicts, indent=1))
    return 1 if "differ" in verdicts.values() else 0


if __name__ == "__main__":
    sys.exit(main())

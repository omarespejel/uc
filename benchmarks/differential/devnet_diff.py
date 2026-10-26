#!/usr/bin/env python3
"""Devnet differential: one scenario against `uc devnet` and starknet-devnet.

    benchmarks/differential/devnet_diff.py --uc target/release/uc --out <dir> \
        --uc-devnet http://127.0.0.1:5062/rpc --reference http://127.0.0.1:5061/rpc

Declares a fresh Counter class with `uc declare`, deploys it with `uc deploy`, invokes and calls
it, then probes the nodes directly over JSON-RPC: receipts, blocks, events, storage, nonces,
state updates, traces, fee estimates, simulation, a reverted call, a stale nonce and a bad
signature (raw transactions signed with starknet-py, independent of uc). Each probe is reported
as `same` (equal values), `same_shape` (equal keys, values differ, as with fees), `differ`, or
`unsupported` on one side. Writes <out>/summary.json.
"""
import argparse
import json
import random
import subprocess
import sys
import urllib.request
from pathlib import Path

from starknet_py.hash.selector import get_selector_from_name
from starknet_py.hash.transaction import CommonTransactionV3Fields, TransactionHashPrefix, compute_invoke_v3_transaction_hash
from starknet_py.hash.utils import message_signature
from starknet_py.net.client_models import DAMode, ResourceBounds, ResourceBoundsMapping

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


def raw_invoke(url, account, calls, nonce, signature_override=None):
    """An INVOKE V3 built and signed with starknet-py; resource bounds from the node's estimate."""
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
        return ResourceBounds(max_amount=max(int(est.get(consumed, "0x0"), 16) * 2, floor), max_price_per_unit=max(int(est.get(price, "0x0"), 16) * 2, 1))
    rb = ResourceBoundsMapping(l1_gas=bound("l1_gas_consumed", "l1_gas_price", 0), l2_gas=bound("l2_gas_consumed", "l2_gas_price", 2_000_000), l1_data_gas=bound("l1_data_gas_consumed", "l1_data_gas_price", 1000))
    chain = int(rpc(url, "starknet_chainId", [])[1], 16)
    common = CommonTransactionV3Fields(tx_prefix=TransactionHashPrefix.INVOKE, version=3, address=sender, tip=0, resource_bounds=rb, paymaster_data=[], nonce_data_availability_mode=DAMode.L1, fee_data_availability_mode=DAMode.L1, chain_id=chain, nonce=nonce)
    tx_hash = compute_invoke_v3_transaction_hash(account_deployment_data=[], calldata=calldata, common_fields=common)
    r, s = message_signature(tx_hash, int(account["private_key"], 16))
    tx = dict(unsigned, signature=[hex(x) for x in (signature_override or [r, s])],
              resource_bounds={k: {"max_amount": hex(getattr(rb, k).max_amount), "max_price_per_unit": hex(getattr(rb, k).max_price_per_unit)} for k in ("l1_gas", "l2_gas", "l1_data_gas")})
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
    uc = str(Path(args.uc).resolve())
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
    return 0


if __name__ == "__main__":
    sys.exit(main())

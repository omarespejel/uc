#!/usr/bin/env python3
"""Audit well-known deployed Starknet mainnet contracts straight from chain with `uc audit`.

Usage:
    benchmarks/scripts/run_mainnet_audit.py --uc target/release/uc [--network mainnet] [--rpc URL]

For each address the deployed Sierra class is fetched over JSON-RPC, decompiled, run through the
symbolic prover and the Caracal-style detectors, and summarised. Output goes to
benchmarks/results/mainnet-audit-<stamp>.json plus a Markdown table on stdout. Nothing is
written to chain. Findings are reported as-is: this is a measurement of the prover on real
bytecode, not a security review of the listed protocols.
"""
import argparse
import json
import subprocess
import sys
import time
from pathlib import Path

# (label, contract address) - public, widely used mainnet contracts.
CORPUS = [
    ("STRK token", "0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d"),
    ("ETH token", "0x049d36570d4e46f48e99674bd3fcc84644ddd6b96f7c741b1562b82f9e004dc7"),
    ("USDC token", "0x053c91253bc9682c04929ca02ed00b3e423f6710d2ee7e0d5ebb06f3ecf368a8"),
    ("USDT token", "0x068f5c6a61780768455de69077e07e89787839bf8166decfbf92b645209c0fb8"),
    ("DAI v2 token", "0x05574eb6b8789a91466f902c380d978e472db68170ff82a5b650b95a58ddf4ad"),
    ("wstETH token", "0x042b8f0484674ca266ac5d08e4ac6a3fe65bd3129795def2dca5c34ecc5f96d2"),
    ("Ekubo core", "0x00000005dd3d2f4429af886cd1a3b08289dbcea99a294197e9eb43b0e0325b4b"),
    ("Ekubo positions", "0x02e0af29598b407c8716b17f6d2795eca1b471413fa03fb145a5e33722184067"),
    ("AVNU exchange", "0x04270219d365d6b017231b52e92b3fb5d7c8378b05e9abc97724537a80e93b0f"),
    ("JediSwap v1 router", "0x041fd22b238fa21cfcf5dd45a8548974d8263b3a531a60388411c5e230f97023"),
    ("MySwap AMM", "0x010884171baf1914edc28d7afb619b40a4051cfae78a094a55d230f19e944a28"),
    ("zkLend market", "0x04c0a5193d58f74fbace4b74dcf65481e734ed1714121bdc571da345540efa05"),
    ("Nostra ETH interest token", "0x057146f6409deb4c9fa12866915dd952aa07c1eb2752e451d7f3b042086bdeb"),
    ("StarkGate ETH bridge", "0x073314940630fd6dcda0d772d4c972c4e0a9946bef9dabf4ef84eda8ef542b82"),
    ("Starknet ID naming", "0x06ac597f8116f886fa1c97a23fa4e08299975ecaf6b598873ca6792b9bbfb678"),
    ("Starknet ID identity", "0x05dbdedc203e92749e2e746e2d40a768d966bd243df04a6b712e222bc040a9af"),
    ("Universal Deployer", "0x041a78e741e5af2fec34b695679bc6891742439f7afb8484ecd7766661ad02bf"),
    ("Starknet staking", "0x00ca1702e64c81d9a07b86bd2c540188d92a2c73cf5cc0e508d4e85d3e9e5c6a"),
    ("Vesu singleton", "0x02545b2e5d519fc230e9cd781046d3a64e092114f07e44771e0d719d148725ef"),
    ("Pragma oracle", "0x02a85bd616f912537c50a49a4076db02c00b29b2cdc8a197ce92ed1837fa875b"),
]


def run_audit(uc, address, network, rpc, timeout):
    cmd = [uc, "audit", address, "--json"]
    cmd += ["--rpc-url", rpc] if rpc else ["--network", network]
    t0 = time.perf_counter()
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return {"ok": False, "error": f"timeout after {timeout}s", "ms": timeout * 1000}
    ms = (time.perf_counter() - t0) * 1000
    if proc.returncode != 0:
        return {"ok": False, "error": (proc.stderr or proc.stdout).strip()[-400:], "ms": round(ms)}
    out = proc.stdout
    try:
        rep = json.loads(out[out.index("{"):])
    except (ValueError, json.JSONDecodeError):
        return {"ok": False, "error": "non-JSON output", "ms": round(ms)}
    findings = rep.get("caracal_findings") or []
    proofs = rep.get("symbolic_proofs") or []
    by_status = {}
    for p in proofs:
        by_status[p.get("status", "?")] = by_status.get(p.get("status", "?"), 0) + 1
    return {
        "ok": True,
        "ms": round(ms),
        "functions": len(proofs),
        "sierra_statements": rep.get("sierra_statements_analyzed"),
        "status_counts": by_status,
        "findings": [
            {"severity": f.get("severity"), "detector": f.get("detector"), "function": f"{f.get('contract_or_module')}::{f.get('function')}"}
            for f in findings
        ],
        "counterexamples": [
            {"function": p.get("function"), "witness": p.get("counterexample"), "reason": p.get("counterexample_reason")}
            for p in proofs if p.get("status") == "COUNTEREXAMPLE_FOUND"
        ],
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--uc", required=True)
    ap.add_argument("--network", default="mainnet")
    ap.add_argument("--rpc")
    ap.add_argument("--timeout", type=int, default=300)
    ap.add_argument("--results-dir", default="benchmarks/results")
    args = ap.parse_args()
    uc = str(Path(args.uc).resolve())
    results = []
    for label, address in CORPUS:
        res = run_audit(uc, address, args.network, args.rpc, args.timeout)
        res.update({"label": label, "address": address})
        results.append(res)
        status = "ok" if res["ok"] else "ERR"
        extra = f"{res.get('functions', 0)} fns, {len(res.get('findings', []))} findings, {len(res.get('counterexamples', []))} cex" if res["ok"] else res["error"][:80]
        print(f"[{status}] {label:28s} {res['ms']/1000:6.1f}s  {extra}", file=sys.stderr, flush=True)
    stamp = time.strftime("%Y%m%d-%H%M%S")
    out_dir = Path(args.results_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    version = subprocess.run([uc, "--version"], capture_output=True, text=True).stdout.strip()
    doc = {"stamp": stamp, "uc": version, "network": args.network, "results": results}
    (out_dir / f"mainnet-audit-{stamp}.json").write_text(json.dumps(doc, indent=2))
    ok = [r for r in results if r["ok"]]
    print(f"\n{len(ok)}/{len(results)} contracts audited, "
          f"{sum(r['functions'] for r in ok)} Sierra functions, "
          f"{sum(len(r['findings']) for r in ok)} findings, "
          f"{sum(len(r['counterexamples']) for r in ok)} counterexamples, "
          f"{sum(r['ms'] for r in ok)/1000:.1f}s total")
    print("\n| contract | time | functions | proven | guarded | counterexamples | findings |")
    print("|---|---|---|---|---|---|---|")
    for r in results:
        if not r["ok"]:
            print(f"| {r['label']} | error | | | | | {r['error'][:60]} |")
            continue
        sc = r["status_counts"]
        fl = ", ".join(f"{f['severity']} {f['detector'].replace('caracal-', '')} ({f['function'].split('::')[-1]})" for f in r["findings"]) or "none"
        print(f"| {r['label']} | {r['ms']/1000:.1f} s | {r['functions']} | {sc.get('PROVEN_SAFE', 0)} | {sc.get('GUARDED_SAFE', 0)} | {sc.get('COUNTEREXAMPLE_FOUND', 0)} | {fl} |")
    return 0


if __name__ == "__main__":
    sys.exit(main())

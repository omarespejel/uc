#!/usr/bin/env python3
"""Hash differential: uc against starknet-py on random inputs.

    benchmarks/differential/hash_diff.py --uc target/release/uc --out <dir> [--cases 500] [--seed 1] [--classes <glob>]

Compares, case by case: `uc selector` with `get_selector_from_name`; `uc utils contract-address`
with `compute_address` (plain and unique deployer); `uc utils tx-hash` with the V3 invoke,
declare and deploy_account hashes; and, for every `.contract_class.json` matching --classes,
`uc class-hash` (Sierra) and the compiled class hash with starknet-py (Poseidon or Blake2s).
With --sncast-url, every Nth address case uses a class declared there and is also checked
against `sncast utils contract-address` (sncast refuses undeclared classes). Disagreements are listed in
<out>/summary.json; exit status 1 if any.
"""
import argparse
import glob
import inspect
import json
import random
import string
import subprocess
import sys
from pathlib import Path

from starknet_py.hash import address as A
from starknet_py.hash import casm_class_hash as CC
from starknet_py.hash import selector as Sl
from starknet_py.hash import sierra_class_hash as SC
from starknet_py.hash import transaction as T
from starknet_py.hash.utils import pedersen_hash
from starknet_py.hash.hash_method import HashMethod
from starknet_py.net import client_models as M

P = 2**251 + 17 * 2**192 + 1
CHAIN = int.from_bytes(b"SN_SEPOLIA", "big")
NEW_UDC = 0x02CEED65A4BD731034C01113685C831B01C15D7D432F71AFB1CF1634B53A2125


def uc_json(uc, args, cwd=None, stdin=None):
    p = subprocess.run([uc, *args, "--json"], capture_output=True, text=True, cwd=cwd, input=stdin)
    out = p.stdout
    try:
        return json.loads(out[out.index("{"):]), None
    except ValueError:
        return None, (p.stderr or p.stdout)[-400:]


def felt(rng, small=False):
    if small:
        return rng.randint(0, 2**64)
    return rng.choice([rng.randint(0, 255), rng.randint(0, 2**128), rng.randint(0, P - 1), P - 1, 0, 1])


def hx(v):
    return hex(v)


def bounds(rng):
    def b():
        return M.ResourceBounds(max_amount=rng.randint(0, 2**64 - 1), max_price_per_unit=rng.randint(0, 2**128 - 1))
    return M.ResourceBoundsMapping(l1_gas=b(), l1_data_gas=b(), l2_gas=b())


def bounds_json(rb):
    return {k: {"max_amount": hx(getattr(rb, k).max_amount), "max_price_per_unit": hx(getattr(rb, k).max_price_per_unit)} for k in ("l1_gas", "l2_gas", "l1_data_gas")}


def common_fields(rng, kind, sender, nonce, rb, tip, paymaster, nda, fda):
    sig = inspect.signature(T.CommonTransactionV3Fields)
    kwargs = {}
    for name in sig.parameters:
        kwargs[name] = {
            "tx_prefix": getattr(T.TransactionHashPrefix, kind),
            "version": 3,
            "address": sender,
            "tx_hash": None,
            "chain_id": CHAIN,
            "nonce": nonce,
            "resource_bounds": rb,
            "tip": tip,
            "paymaster_data": paymaster,
            "nonce_data_availability_mode": nda,
            "fee_data_availability_mode": fda,
        }.get(name)
    return T.CommonTransactionV3Fields(**{k: v for k, v in kwargs.items() if v is not None or k == "tx_hash"})


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--uc", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--cases", type=int, default=300)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--classes", default="")
    ap.add_argument("--sncast-url", default="", help="RPC where --sncast-classes are declared; enables the sncast comparison")
    ap.add_argument("--sncast-classes", default="", help="comma-separated class hashes declared at --sncast-url")
    ap.add_argument("--sncast-every", type=int, default=5, help="compare every Nth address case with sncast")
    args = ap.parse_args()
    uc = str(Path(args.uc).resolve())
    out = Path(args.out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    rng = random.Random(args.seed)
    mism = {"selector": [], "address": [], "address_vs_sncast": [], "tx_hash": [], "class_hash": [], "compiled_class_hash": []}
    counts = {k: 0 for k in mism}
    counts["sncast_errors"] = 0

    for _ in range(args.cases):
        name = "".join(rng.choice(string.ascii_letters + "_") for _ in range(rng.randint(1, 30)))
        rep, err = uc_json(uc, ["selector", name])
        expected = Sl.get_selector_from_name(name)
        got = ((rep or {}).get("results") or [{}])[0].get("selector")
        counts["selector"] += 1
        if got is None or int(got, 16) != expected:
            mism["selector"].append({"name": name, "uc": got, "ref": hx(expected), "err": err})

    sncast_classes = [int(h, 16) for h in args.sncast_classes.split(",") if h]
    for case in range(args.cases):
        use_sncast = bool(args.sncast_url and sncast_classes) and case % args.sncast_every == 0
        class_hash = rng.choice(sncast_classes) if use_sncast else felt(rng)
        salt = felt(rng)
        calldata = [felt(rng) for _ in range(rng.randint(0, 6))]
        deployer = felt(rng)
        for unique in (False, True):
            cmd = ["utils", "contract-address", "--class-hash", hx(class_hash), "--salt", hx(salt), "--deployer-address", hx(deployer), "--constructor-calldata", *[hx(c) for c in calldata]]
            if unique:
                cmd.append("--unique")
            rep, err = uc_json(uc, cmd)
            if unique:
                # A unique UDC deployment uses salt pedersen(deployer, salt) and deploys from the
                # UDC itself; uc, like sncast 0.60, calls OpenZeppelin's Cairo 1 UDC.
                expected = A.compute_address(class_hash=class_hash, constructor_calldata=calldata, salt=pedersen_hash(deployer, salt), deployer_address=NEW_UDC)
            else:
                expected = A.compute_address(class_hash=class_hash, constructor_calldata=calldata, salt=salt, deployer_address=0)
            got = (rep or {}).get("contract_address")
            counts["address"] += 1
            sncast_got = None
            if use_sncast:
                # sncast rejects an empty `--constructor-calldata`, so omit the flag then.
                sn_cmd = [c for c in cmd[2:] if c != "--constructor-calldata" or calldata]
                sn = subprocess.run(["sncast", "--json", "utils", "contract-address", *sn_cmd, "--url", args.sncast_url], capture_output=True, text=True)
                line = next((l for l in sn.stdout.splitlines() if '"contract_address"' in l), None)
                if line is None:
                    counts["sncast_errors"] += 1
                else:
                    sncast_got = json.loads(line)["contract_address"]
                    counts["address_vs_sncast"] += 1
                    if got is None or int(sncast_got, 16) != int(got, 16):
                        mism["address_vs_sncast"].append({"cmd": cmd, "uc": got, "sncast": sncast_got})
            if got is None or int(got, 16) != expected:
                mism["address"].append({"unique": unique, "class_hash": hx(class_hash), "salt": hx(salt), "deployer": hx(deployer), "calldata": [hx(c) for c in calldata], "uc": got, "ref": hx(expected), "sncast": sncast_got, "err": err})

    da = {"L1": 0, "L2": 1}
    for _ in range(args.cases):
        kind = rng.choice(["INVOKE", "DECLARE", "DEPLOY_ACCOUNT"])
        sender = felt(rng)
        nonce = rng.randint(0, 2**32)
        rb = bounds(rng)
        tip = rng.choice([0, 0, rng.randint(0, 2**32)])
        paymaster = [felt(rng) for _ in range(rng.choice([0, 0, 1, 2]))]
        nda, fda = rng.choice(["L1", "L2"]), rng.choice(["L1", "L2"])
        tx = {"type": kind, "version": "0x3", "nonce": hx(nonce), "resource_bounds": bounds_json(rb), "tip": hx(tip), "paymaster_data": [hx(x) for x in paymaster], "nonce_data_availability_mode": nda, "fee_data_availability_mode": fda}
        add = [felt(rng) for _ in range(rng.choice([0, 0, 1]))]
        if kind == "INVOKE":
            calldata = [felt(rng) for _ in range(rng.randint(0, 8))]
            tx.update({"sender_address": hx(sender), "calldata": [hx(c) for c in calldata], "account_deployment_data": [hx(x) for x in add]})
            cf = common_fields(rng, "INVOKE", sender, nonce, rb, tip, paymaster, M.DAMode(da[nda]), M.DAMode(da[fda]))
            expected = T.compute_invoke_v3_transaction_hash(account_deployment_data=add, calldata=calldata, common_fields=cf)
        elif kind == "DECLARE":
            ch, cch = felt(rng), felt(rng)
            tx.update({"sender_address": hx(sender), "class_hash": hx(ch), "compiled_class_hash": hx(cch), "account_deployment_data": [hx(x) for x in add]})
            cf = common_fields(rng, "DECLARE", sender, nonce, rb, tip, paymaster, M.DAMode(da[nda]), M.DAMode(da[fda]))
            expected = T.compute_declare_v3_transaction_hash(class_hash=ch, account_deployment_data=add, compiled_class_hash=cch, common_fields=cf)
        else:
            ch, salt = felt(rng), felt(rng)
            calldata = [felt(rng) for _ in range(rng.randint(0, 4))]
            addr = A.compute_address(class_hash=ch, constructor_calldata=calldata, salt=salt, deployer_address=0)
            tx.update({"class_hash": hx(ch), "contract_address_salt": hx(salt), "constructor_calldata": [hx(c) for c in calldata]})
            cf = common_fields(rng, "DEPLOY_ACCOUNT", addr, nonce, rb, tip, paymaster, M.DAMode(da[nda]), M.DAMode(da[fda]))
            expected = T.compute_deploy_account_v3_transaction_hash(class_hash=ch, constructor_calldata=calldata, contract_address_salt=salt, common_fields=cf)
        tx_path = out / "tx.json"
        tx_path.write_text(json.dumps(tx))
        rep, err = uc_json(uc, ["utils", "tx-hash", "--tx", str(tx_path), "--chain-id", "SN_SEPOLIA"])
        got = (rep or {}).get("transaction_hash")
        counts["tx_hash"] += 1
        if got is None or int(got, 16) != expected:
            mism["tx_hash"].append({"tx": tx, "uc": got, "ref": hx(expected), "err": err})

    if args.classes:
        from starknet_py.common import create_casm_class, create_sierra_compiled_contract
        for path in sorted(glob.glob(args.classes))[:400]:
            text = Path(path).read_text()
            try:
                contract = create_sierra_compiled_contract(compiled_contract=text)
                expected = SC.compute_sierra_class_hash(contract)
            except Exception as e:  # noqa: BLE001
                mism["class_hash"].append({"path": path, "ref_error": str(e)[:200]})
                continue
            rep, err = uc_json(uc, ["class-hash", path])
            got = (rep or {}).get("class_hash")
            counts["class_hash"] += 1
            if got is None or int(got, 16) != expected:
                mism["class_hash"].append({"path": path, "uc": got, "ref": hx(expected), "err": err})
            casm_path = path.replace(".contract_class.json", ".compiled_contract_class.json")
            if Path(casm_path).is_file():
                casm = create_casm_class(Path(casm_path).read_text())
                poseidon = CC.compute_casm_class_hash(casm, hash_method=HashMethod.POSEIDON)
                blake = CC.compute_casm_class_hash(casm, hash_method=HashMethod.BLAKE2S)
                rep, err = uc_json(uc, ["class-hash", casm_path])
                got = (rep or {}).get("compiled_class_hash")
                counts["compiled_class_hash"] += 1
                if got is None or int(got, 16) != blake:
                    mism["compiled_class_hash"].append({"path": casm_path, "uc": got, "poseidon": hx(poseidon), "blake2s": hx(blake), "err": err})
    summary = {"seed": args.seed, "counts": counts, "mismatches": {k: len(v) for k, v in mism.items()}, "details": {k: v[:20] for k, v in mism.items()}}
    (out / "summary.json").write_text(json.dumps(summary, indent=2))
    print(json.dumps({"counts": counts, "mismatches": summary["mismatches"]}))
    return 1 if any(mism.values()) else 0


if __name__ == "__main__":
    sys.exit(main())

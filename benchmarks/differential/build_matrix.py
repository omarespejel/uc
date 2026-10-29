#!/usr/bin/env python3
"""Build-output differential: `uc build` against `scarb build` across manifest shapes.

    benchmarks/differential/build_matrix.py --uc target/release/uc --out <dir>

For every combination of targets (none, [lib], [[target.starknet-contract]], both), target
options (sierra-text, casm, sierra = false) and source shape (plain functions, plus a
`#[starknet::contract]` module), builds a fresh package with both tools and compares the set of
files in target/dev and their contents. Contract artifacts and text outputs must be
byte-identical; a lib `.sierra.json` must be equal after replacing interned ids with debug
names (scarb's own ids change between runs). Writes <out>/summary.json; exit 1 on any mismatch.
"""
import argparse
import itertools
import json
import os
import subprocess
import sys
import time
from pathlib import Path

PLAIN = """pub fn add(a: u32, b: u32) -> u32 {
    a + b
}

pub fn pick(a: u8, b: u8) -> u8 {
    if a > b { a - b } else { b }
}
"""

CONTRACT = """
#[starknet::interface]
pub trait ICounter<T> {
    fn get(self: @T) -> u64;
    fn bump(ref self: T, by: u64);
}

#[starknet::contract]
pub mod Counter {
    use starknet::storage::{StoragePointerReadAccess, StoragePointerWriteAccess};

    #[storage]
    struct Storage {
        value: u64,
    }

    #[abi(embed_v0)]
    impl CounterImpl of super::ICounter<ContractState> {
        fn get(self: @ContractState) -> u64 {
            self.value.read()
        }
        fn bump(ref self: ContractState, by: u64) {
            self.value.write(self.value.read() + by);
        }
    }
}
"""


def norm_ids(x):
    if isinstance(x, dict):
        if set(x) == {"id", "debug_name"} and isinstance(x["id"], int):
            return {"name": x["debug_name"]} if x["debug_name"] is not None else {"id": x["id"]}
        return {k: norm_ids(v) for k, v in x.items()}
    if isinstance(x, list):
        return [norm_ids(v) for v in x]
    return x


def manifest(name, lib, contract):
    lines = ["[package]", f'name = "{name}"', 'version = "0.1.0"', 'edition = "2024_07"', "", "[dependencies]", 'starknet = "2.18.0"', ""]
    if lib is not None:
        lines.append("[lib]")
        lines += [f"{k} = {str(v).lower()}" for k, v in lib.items()]
        lines.append("")
    if contract is not None:
        lines.append("[[target.starknet-contract]]")
        lines += [f"{k} = {str(v).lower()}" for k, v in contract.items()]
        lines.append("")
    return "\n".join(lines)


def build(tool, cmd, root, env):
    started = time.time()
    p = subprocess.run(cmd, cwd=root, capture_output=True, text=True, env=env)
    files = {}
    dev = root / "target" / "dev"
    if dev.is_dir():
        for f in sorted(dev.iterdir()):
            if f.is_file():
                files[f.name] = f.read_bytes()
    return {"ok": p.returncode == 0, "files": files, "log": (p.stderr + p.stdout)[-600:], "s": round(time.time() - started, 2)}


def compare(a, b):
    problems = []
    if a["ok"] != b["ok"]:
        problems.append(f"exit differs: uc ok={a['ok']} scarb ok={b['ok']}: {a['log'] if not a['ok'] else b['log']}")
    only_uc = sorted(set(a["files"]) - set(b["files"]))
    only_scarb = sorted(set(b["files"]) - set(a["files"]))
    if only_uc:
        problems.append(f"only uc wrote {only_uc}")
    if only_scarb:
        problems.append(f"only scarb wrote {only_scarb}")
    for name in sorted(set(a["files"]) & set(b["files"])):
        x, y = a["files"][name], b["files"][name]
        if x == y:
            continue
        if name.endswith(".sierra.json"):
            if norm_ids(json.loads(x)) == norm_ids(json.loads(y)):
                if x.strip().startswith(b"{\n") != y.strip().startswith(b"{\n"):
                    problems.append(f"{name}: same program, different JSON formatting")
                continue
            problems.append(f"{name}: different program")
        else:
            problems.append(f"{name}: bytes differ ({len(x)} vs {len(y)})")
    return problems


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--uc", required=True)
    ap.add_argument("--out", required=True)
    args = ap.parse_args()
    uc = str(Path(args.uc).resolve())
    out = Path(args.out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, SCARB_OFFLINE="true")
    lib_opts = [None, {}, {"sierra-text": True}, {"casm": True}, {"sierra-text": True, "casm": True}, {"sierra": False, "sierra-text": True}]
    contract_opts = [None, {}, {"casm": True}, {"sierra": False, "casm": True}]
    cases = []
    for i, (lib, contract, with_contract) in enumerate(itertools.product(lib_opts, contract_opts, [False, True])):
        name = f"bm{i}"
        root = out / name
        if root.exists():
            continue
        (root / "src").mkdir(parents=True)
        (root / "Scarb.toml").write_text(manifest(name, lib, contract))
        (root / "src" / "lib.cairo").write_text(PLAIN + (CONTRACT if with_contract else ""))
        scarb_root = out / f"{name}_scarb"
        (scarb_root / "src").mkdir(parents=True)
        (scarb_root / "Scarb.toml").write_text(manifest(name, lib, contract))
        (scarb_root / "src" / "lib.cairo").write_text(PLAIN + (CONTRACT if with_contract else ""))
        a = build("uc", [uc, "build"], root, env)
        b = build("scarb", ["scarb", "build"], scarb_root, env)
        problems = compare(a, b)
        case = {"case": name, "lib": lib, "contract": contract, "contract_module": with_contract, "uc_files": sorted(a["files"]), "scarb_files": sorted(b["files"]), "uc_s": a["s"], "scarb_s": b["s"], "problems": problems}
        cases.append(case)
        print(json.dumps({k: case[k] for k in ("case", "lib", "contract", "contract_module", "problems")}))
    summary = {"cases": len(cases), "mismatched": sum(1 for c in cases if c["problems"]), "details": cases}
    (out / "summary.json").write_text(json.dumps(summary, indent=2))
    print(json.dumps({"cases": summary["cases"], "mismatched": summary["mismatched"]}))
    return 1 if summary["mismatched"] else 0


if __name__ == "__main__":
    sys.exit(main())

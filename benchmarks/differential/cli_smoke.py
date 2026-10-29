#!/usr/bin/env python3
"""Every uc command on a real project, in an empty directory and with bad inputs.

    python3 benchmarks/differential/cli_smoke.py --uc target/release/uc --out <dir> [--rpc <url>]

A command may fail, but never by panicking: a panic, a Rust backtrace or exit code 101 is a
bug. Under --json stdout must hold exactly one JSON document, on success and on failure.
Writes <out>/summary.json; exits 1 on any panic or broken JSON output.
"""
import argparse
import json
import shutil
import subprocess
import sys
from pathlib import Path

COUNTER = '''#[starknet::interface]
pub trait ICounter<T> { fn increase(ref self: T, by: u64); fn get(self: @T) -> u64; }
#[starknet::contract]
pub mod Counter {
    use starknet::storage::{StoragePointerReadAccess, StoragePointerWriteAccess};
    #[storage]
    struct Storage { value: u64 }
    #[abi(embed_v0)]
    impl CounterImpl of super::ICounter<ContractState> {
        fn increase(ref self: ContractState, by: u64) { self.value.write(self.value.read() + by); }
        fn get(self: @ContractState) -> u64 { self.value.read() }
    }
}
pub fn add(a: u64, b: u64) -> u64 { a + b }
#[cfg(test)]
mod tests {
    #[test]
    fn adds() { assert(super::add(2, 3) == 5, 'bad'); }
}
'''

# (label, argv, where): where is "project", "empty" or "both".
CASES = [
    ("version", ["--version"], "empty"),
    ("check", ["check"], "both"),
    ("build", ["build"], "both"),
    ("test", ["test"], "both"),
    ("fmt-check", ["fmt", "--check"], "both"),
    ("lint", ["lint"], "both"),
    ("metadata", ["metadata", "--format-version", "1"], "both"),
    ("tree", ["tree"], "both"),
    ("expand", ["expand"], "both"),
    ("profile", ["profile"], "both"),
    ("abi", ["abi"], "both"),
    ("doc", ["doc"], "both"),
    ("prove", ["prove"], "both"),
    ("audit-local", ["audit"], "both"),
    ("decompile-missing", ["decompile", "no_such_file.contract_class.json"], "both"),
    ("class-hash", ["class-hash"], "both"),
    ("class-hash-missing", ["class-hash", "nope.contract_class.json"], "empty"),
    ("selector", ["selector", "transfer"], "empty"),
    ("selector-bad-hex", ["selector", "0xZZZ", "--decode"], "empty"),
    ("utils-contract-address", ["utils", "contract-address", "--class-hash", "0x1", "--salt", "0x2"], "empty"),
    ("utils-contract-address-bad", ["utils", "contract-address", "--class-hash", "nothex", "--salt", "0x2"], "empty"),
    ("utils-tx-hash-missing", ["utils", "tx-hash", "--tx", "missing.json"], "empty"),
    ("utils-serialize", ["utils", "serialize", "--arguments", "1", "2"], "empty"),
    ("manifest-path", ["manifest-path"], "both"),
    ("clean", ["clean"], "both"),
    ("execute-none", ["execute"], "both"),
    ("migrate-dry", ["migrate", "--dry-run"], "both"),
    ("call-bad-rpc", ["call", "--contract-address", "0x1", "--function", "get", "--url", "http://127.0.0.1:9/rpc"], "empty"),
    ("call-bad-address", ["call", "--contract-address", "zzz", "--function", "get", "--url", "RPC"], "empty"),
    ("get-nonce", ["get", "nonce", "0x1", "--url", "RPC"], "empty"),
    ("get-class-hash-at-missing", ["get", "class-hash-at", "0x123456", "--url", "RPC"], "empty"),
    ("balance", ["balance", "--account", "0x1", "--url", "RPC"], "empty"),
    ("tx-status-unknown", ["tx-status", "0x1234", "--url", "RPC"], "empty"),
    ("trace-unknown", ["trace", "0x1234", "--url", "RPC"], "empty"),
    ("audit-unknown-address", ["audit", "0x123456", "--url", "RPC"], "empty"),
    ("declare-no-account", ["declare", "-c", "Counter", "--url", "RPC"], "project"),
    ("invoke-no-account", ["invoke", "-a", "0x1", "-f", "increase", "-c", "1", "--url", "RPC"], "empty"),
    ("deploy-no-account", ["deploy", "-g", "0x1", "--url", "RPC"], "empty"),
    ("account-help", ["account", "--help"], "empty"),
    ("show-config", ["show-config"], "both"),
    ("doctor-offline", ["doctor", "--offline"], "empty"),
    ("tree-help", ["tree", "--help"], "empty"),
    ("list-registry", ["list", "openzeppelin_token"], "empty"),
    ("check-requirements", ["check-requirements"], "both"),
    ("commands", ["commands"], "empty"),
    ("completions", ["completions", "zsh"], "empty"),
    ("watch-once-missing", ["watch", "0x123456", "--url", "RPC", "--once"], "empty"),
]
JSON_CAPABLE = {"check", "build", "test", "lint", "profile", "prove", "audit-local", "class-hash", "class-hash-missing",
                "selector", "selector-bad-hex", "utils-tx-hash-missing", "call-bad-rpc", "call-bad-address", "tx-status-unknown",
                "trace-unknown", "audit-unknown-address", "declare-no-account", "invoke-no-account", "deploy-no-account",
                "get-class-hash-at-missing", "decompile-missing", "doctor-offline", "watch-once-missing"}


def json_problem(stdout):
    """Why stdout is not exactly one JSON document, or None."""
    decoder, text, i, docs = json.JSONDecoder(), stdout.strip(), 0, 0
    try:
        while i < len(text):
            _, i = decoder.raw_decode(text, i)
            docs += 1
            while i < len(text) and text[i].isspace():
                i += 1
    except ValueError:
        return "stdout is not JSON"
    return None if docs == 1 else f"{docs} JSON documents on stdout"


def run(uc, argv, cwd, timeout=300):
    try:
        p = subprocess.run([uc, *argv], cwd=cwd, capture_output=True, text=True, timeout=timeout)
        return p.returncode, p.stdout, p.stderr
    except subprocess.TimeoutExpired:
        return 124, "", f"timeout after {timeout} s"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--uc", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--rpc", default="http://127.0.0.1:5062/rpc")
    args = ap.parse_args()
    uc = str(Path(args.uc).resolve())
    out = Path(args.out).resolve()
    out.mkdir(parents=True, exist_ok=False)
    rows, bad = [], []
    for label, argv, where in CASES:
        argv = [args.rpc if a == "RPC" else a for a in argv]
        for place in (["project", "empty"] if where == "both" else [where]):
            cwd = out / f"{label}-{place}"
            cwd.mkdir()
            if place == "project":
                (cwd / "src").mkdir()
                (cwd / "Scarb.toml").write_text('[package]\nname = "smoke"\nversion = "0.1.0"\nedition = "2024_07"\n\n[dependencies]\nstarknet = "2.18.0"\n\n[[target.starknet-contract]]\nsierra = true\n')
                (cwd / "src" / "lib.cairo").write_text(COUNTER)
            variants = [argv] + ([argv + ["--json"]] if label in JSON_CAPABLE else [])
            for variant in variants:
                rc, stdout, stderr = run(uc, variant, cwd)
                text = stdout + stderr
                panicked = rc == 101 or "panicked at" in text or "RUST_BACKTRACE" in text
                bad_json = json_problem(stdout) if "--json" in variant else None
                row = {"case": label, "where": place, "argv": variant, "rc": rc, "panicked": panicked, "json_problem": bad_json,
                       "tail": text.strip()[-300:]}
                rows.append(row)
                if panicked or bad_json:
                    bad.append(row)
    (out / "summary.json").write_text(json.dumps({"rows": rows, "bad": bad}, indent=1))
    for row in bad:
        print(json.dumps({k: row[k] for k in ("case", "where", "argv", "rc", "panicked", "json_problem", "tail")}))
    print(json.dumps({"runs": len(rows), "bad": len(bad)}))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())

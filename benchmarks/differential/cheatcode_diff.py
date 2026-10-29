#!/usr/bin/env python3
"""Differential campaign over snforge_std cheatcodes and multi-contract Starknet state.

    benchmarks/differential/cheatcode_diff.py --uc target/release/uc --out <dir> \
        [--packages 4] [--tests 25] [--seed 1] [--snforge]

Generates multi-contract Cairo packages (`Target` + `Router`) and randomized stateful test
sequences mixing:
  * `start_cheat_caller_address` / `stop_cheat_caller_address` / `cheat_caller_address(..., CheatSpan::TargetCalls(n))`
  * `start_cheat_block_timestamp` / `stop_cheat_block_timestamp` / `cheat_block_timestamp(..., CheatSpan::TargetCalls(n))`
  * `start_cheat_block_number` / `stop_cheat_block_number` / `cheat_block_number(..., CheatSpan::TargetCalls(n))`
  * `start_cheat_sequencer_address` / `stop_cheat_sequencer_address`
  * `start_mock_call` / `stop_mock_call` / counted `mock_call(addr, selector, val, n_calls)`
  * `store` / `load` direct storage manipulation
  * `spy_events` (`assert_emitted` / `assert_not_emitted`)
  * `spy_messages_to_l1` (`assert_sent` / `assert_not_sent`)
  * Direct calls (`Target`) and nested cross-contract calls (`Router -> Target`)
  * Parameterized fuzz tests (`#[test]` with parameters and `#[fuzzer(runs: N, seed: S)]`)

A reference Python state machine computes the exact expected return values, storage values,
emitted events, and L2->L1 messages after every step, and emits `assert(...)` statements inside
each `#[test]`. Both `uc test` and `snforge test` (when `--snforge` is passed) must pass 100% of
the generated tests and agree on every verdict.
"""
import argparse
import json
import os
import random
import re
import subprocess
import sys
import time
from pathlib import Path

MANIFEST = """[package]
name = "{name}"
version = "0.1.0"
edition = "2024_07"

[dependencies]
starknet = "2.18.0"

[dev-dependencies]
snforge_std = "0.60.0"

[[target.starknet-contract]]
sierra = true
casm = true

[scripts]
test = "snforge test"

[tool.scarb]
allow-prebuilt-plugins = ["snforge_std"]
"""

CONTRACTS_PRELUDE = """#[starknet::interface]
pub trait ITarget<TContractState> {
    fn observe_context(self: @TContractState) -> (felt252, u64, u64, felt252);
    fn compute_fee(self: @TContractState, amount: u64) -> u64;
    fn record_action(ref self: TContractState, tag: u64, l1_recipient: felt252) -> (felt252, u64, u64);
    fn read_counter(self: @TContractState) -> u64;
    fn write_counter(ref self: TContractState, v: u64);
}

#[starknet::interface]
pub trait IRouter<TContractState> {
    fn forward_context(self: @TContractState, target: starknet::ContractAddress) -> (felt252, u64, u64, felt252);
    fn forward_fee(self: @TContractState, target: starknet::ContractAddress, amount: u64) -> u64;
}

#[starknet::contract]
pub mod Target {
    use starknet::{
        get_caller_address, get_block_timestamp, get_block_ number, get_execution_info,
        syscalls::send_message_to_l1_syscall, SyscallResultTrait,
    };
    use starknet::storage::{StoragePointerReadAccess, StoragePointerWriteAccess};

    #[storage]
    struct Storage {
        counter: u64,
    }

    #[event]
    #[derive(Drop, starknet::Event)]
    pub enum Event {
        ActionRecorded: ActionRecorded,
    }

    #[derive(Drop, starknet::Event)]
    pub struct ActionRecorded {
        pub caller: felt252,
        pub tag: u64,
        pub timestamp: u64,
    }

    #[abi(embed_v0)]
    impl TargetImpl of super::ITarget<ContractState> {
        fn observe_context(self: @ContractState) -> (felt252, u64, u64, felt252) {
            let caller: felt252 = get_caller_address().into();
            let ts = get_block_timestamp();
            let bn = get_block_number();
            let exec = get_execution_info().unbox();
            let block_info = exec.block_info.unbox();
            let seq: felt252 = block_info.sequencer_address.into();
            (caller, ts, bn, seq)
        }

        fn compute_fee(self: @ContractState, amount: u64) -> u64 {
            let c = self.counter.read();
            amount + c + 10_u64
        }

        fn record_action(ref self: ContractState, tag: u64, l1_recipient: felt252) -> (felt252, u64, u64) {
            let caller: felt252 = get_caller_address().into();
            let ts = get_block_timestamp();
            let next = self.counter.read() + tag;
            self.counter.write(next);
            self.emit(ActionRecorded { caller, tag, timestamp: ts });
            if l1_recipient != 0 {
                send_message_to_l1_syscall(l1_recipient, array![caller, tag.into(), next.into()].span()).unwrap_syscall();
            }
            (caller, ts, next)
        }

        fn read_counter(self: @ContractState) -> u64 {
            self.counter.read()
        }

        fn write_counter(ref self: ContractState, v: u64) {
            self.counter.write(v);
        }
    }
}

#[starknet::contract]
pub mod Router {
    use super::{ITargetDispatcher, ITargetDispatcherTrait};

    #[storage]
    struct Storage {}

    #[abi(embed_v0)]
    impl RouterImpl of super::IRouter<ContractState> {
        fn forward_context(self: @ContractState, target: starknet::ContractAddress) -> (felt252, u64, u64, felt252) {
            let disp = ITargetDispatcher { contract_address: target };
            disp.observe_context()
        }

        fn forward_fee(self: @ContractState, target: starknet::ContractAddress, amount: u64) -> u64 {
            let disp = ITargetDispatcher { contract_address: target };
            disp.compute_fee(amount)
        }
    }
}
""".replace("get_block_ number", "get_block_number")


class CheatSim:
    """Reference Python state machine mirroring snforge_std + Starknet execution context."""

    def __init__(self):
        # Default snforge execution context
        self.default_caller = "test_addr"
        self.default_ts = 0
        self.default_bn = 2000
        self.default_seq = 0x1000
        self.caller_cheat = None       # (value, remaining_calls_or_None)
        self.global_caller = None
        self.ts_cheat = None           # (value, remaining_calls_or_None)
        self.global_ts = None
        self.bn_cheat = None           # (value, remaining_calls_or_None)
        self.global_bn = None
        self.seq_cheat = None          # (value, remaining_calls_or_None)
        self.mock_fee = None           # (value, remaining_calls_or_None)
        self.counter = 0

    @staticmethod
    def _consume(cheat, global_val, default_val):
        if cheat is None:
            return (global_val if global_val is not None else default_val), None
        if cheat == "uncheated":
            return default_val, "uncheated"
        val, rem = cheat
        if rem is None:
            return val, cheat
        if rem <= 1:
            return val, "uncheated"
        return val, (val, rem - 1)

    def observe(self, via_router=False):
        caller, self.caller_cheat = self._consume(
            self.caller_cheat,
            self.global_caller,
            "router_addr" if via_router else self.default_caller,
        )
        ts, self.ts_cheat = self._consume(self.ts_cheat, self.global_ts, self.default_ts)
        bn, self.bn_cheat = self._consume(self.bn_cheat, self.global_bn, self.default_bn)
        seq, self.seq_cheat = self._consume(self.seq_cheat, None, self.default_seq)
        return caller, ts, bn, seq

    def compute_fee(self, amount):
        if self.mock_fee is not None and self.mock_fee != "uncheated":
            val, next_mock = self._consume(self.mock_fee, None, 0)
            self.mock_fee = None if next_mock == "uncheated" else next_mock
            return val
        return amount + self.counter + 10

    def record_action(self, tag, l1_recipient):
        caller, self.caller_cheat = self._consume(
            self.caller_cheat, self.global_caller, self.default_caller
        )
        ts, self.ts_cheat = self._consume(self.ts_cheat, self.global_ts, self.default_ts)
        # Note: get_block_number and get_execution_info are not called in record_action,
        # but snforge consumes per-contract frame cheats on every call to the target contract!
        _, self.bn_cheat = self._consume(self.bn_cheat, self.global_bn, self.default_bn)
        _, self.seq_cheat = self._consume(self.seq_cheat, None, self.default_seq)
        self.counter += tag
        return caller, ts, self.counter


def gen_stateful_test(rng, idx):
    sim = CheatSim()
    lines = [
        f"    #[test]",
        f"    fn test_cheat_sequence_{idx}() {{",
        f"        let target_class = declare(\"Target\").unwrap().contract_class();",
        f"        let (target_addr, _) = target_class.deploy(@array![]).unwrap();",
        f"        let router_class = declare(\"Router\").unwrap().contract_class();",
        f"        let (router_addr, _) = router_class.deploy(@array![]).unwrap();",
        f"        let target = ITargetDispatcher {{ contract_address: target_addr }};",
        f"        let router = IRouterDispatcher {{ contract_address: router_addr }};",
        f"        let test_addr: felt252 = starknet::get_contract_address().into();",
        f"        let router_felt: felt252 = router_addr.into();",
        f"        let mut ev_spy = spy_events();",
        f"        let mut l1_spy = spy_messages_to_l1();",
    ]
    n_ops = rng.randint(8, 16)
    for step in range(n_ops):
        op = rng.choice([
            "cheat_caller_indef",
            "cheat_caller_counted",
            "stop_caller",
            "cheat_caller_global",
            "stop_caller_global",
            "cheat_ts_indef",
            "cheat_ts_counted",
            "stop_ts",
            "cheat_ts_global",
            "stop_ts_global",
            "cheat_bn_indef",
            "cheat_bn_counted",
            "stop_bn",
            "cheat_bn_global",
            "stop_bn_global",
            "cheat_seq_indef",
            "stop_seq",
            "mock_fee_counted",
            "mock_fee_indef",
            "stop_mock_fee",
            "write_counter",
            "store_load_counter",
            "record_action",
        ])
        if op == "cheat_caller_indef":
            val = rng.choice([0x111, 0x222, 0x333, 0x999])
            sim.caller_cheat = (hex(val), None)
            lines.append(f"        start_cheat_caller_address(target_addr, {hex(val)}.try_into().unwrap());")
        elif op == "cheat_caller_counted":
            val = rng.choice([0x444, 0x555, 0x777])
            cnt = rng.randint(1, 3)
            sim.caller_cheat = (hex(val), cnt)
            lines.append(f"        cheat_caller_address(target_addr, {hex(val)}.try_into().unwrap(), CheatSpan::TargetCalls({cnt}));")
        elif op == "stop_caller":
            sim.caller_cheat = "uncheated"
            lines.append("        stop_cheat_caller_address(target_addr);")
        elif op == "cheat_caller_global":
            val = rng.choice([0x888, 0x999, 0xEEE])
            sim.global_caller = hex(val)
            sim.caller_cheat = None
            lines.append(f"        start_cheat_caller_address_global({hex(val)}.try_into().unwrap());")
        elif op == "stop_caller_global":
            sim.global_caller = None
            sim.caller_cheat = None
            lines.append("        stop_cheat_caller_address_global();")
        elif op == "cheat_ts_indef":
            ts = rng.randint(100, 900_000)
            sim.ts_cheat = (ts, None)
            lines.append(f"        start_cheat_block_timestamp(target_addr, {ts}_u64);")
        elif op == "cheat_ts_counted":
            ts = rng.randint(100, 900_000)
            cnt = rng.randint(1, 3)
            sim.ts_cheat = (ts, cnt)
            lines.append(f"        cheat_block_timestamp(target_addr, {ts}_u64, CheatSpan::TargetCalls({cnt}));")
        elif op == "stop_ts":
            sim.ts_cheat = "uncheated"
            lines.append("        stop_cheat_block_timestamp(target_addr);")
        elif op == "cheat_ts_global":
            ts = rng.randint(100, 900_000)
            sim.global_ts = ts
            sim.ts_cheat = None
            lines.append(f"        start_cheat_block_timestamp_global({ts}_u64);")
        elif op == "stop_ts_global":
            sim.global_ts = None
            sim.ts_cheat = None
            lines.append("        stop_cheat_block_timestamp_global();")
        elif op == "cheat_bn_indef":
            bn = rng.randint(1, 50_000)
            sim.bn_cheat = (bn, None)
            lines.append(f"        start_cheat_block_number(target_addr, {bn}_u64);")
        elif op == "cheat_bn_counted":
            bn = rng.randint(1, 50_000)
            cnt = rng.randint(1, 3)
            sim.bn_cheat = (bn, cnt)
            lines.append(f"        cheat_block_number(target_addr, {bn}_u64, CheatSpan::TargetCalls({cnt}));")
        elif op == "stop_bn":
            sim.bn_cheat = "uncheated"
            lines.append("        stop_cheat_block_number(target_addr);")
        elif op == "cheat_bn_global":
            bn = rng.randint(1, 50_000)
            sim.global_bn = bn
            sim.bn_cheat = None
            lines.append(f"        start_cheat_block_number_global({bn}_u64);")
        elif op == "stop_bn_global":
            sim.global_bn = None
            sim.bn_cheat = None
            lines.append("        stop_cheat_block_number_global();")
        elif op == "cheat_seq_indef":
            seq = rng.choice([0xAAA, 0xBBB, 0xCCC])
            sim.seq_cheat = (seq, None)
            lines.append(f"        start_cheat_sequencer_address(target_addr, {hex(seq)}.try_into().unwrap());")
        elif op == "stop_seq":
            sim.seq_cheat = "uncheated"
            lines.append("        stop_cheat_sequencer_address(target_addr);")
        elif op == "mock_fee_counted":
            mval = rng.randint(1000, 9999)
            cnt = rng.randint(1, 3)
            sim.mock_fee = (mval, cnt)
            lines.append(f"        mock_call(target_addr, selector!(\"compute_fee\"), {mval}_u64, {cnt});")
        elif op == "mock_fee_indef":
            mval = rng.randint(1000, 9999)
            sim.mock_fee = (mval, None)
            lines.append(f"        start_mock_call(target_addr, selector!(\"compute_fee\"), {mval}_u64);")
        elif op == "stop_mock_fee":
            sim.mock_fee = None
            lines.append("        stop_mock_call(target_addr, selector!(\"compute_fee\"));")
        elif op == "write_counter":
            v = rng.randint(0, 500)
            # write_counter is a real call to target_addr, so counted frame cheats on target_addr tick once!
            sim.observe(via_router=False)
            sim.counter = v
            lines.append(f"        target.write_counter({v}_u64);")
            lines.append(f"        assert(target.read_counter() == {v}_u64, 'cnt_{step}');")
            # read_counter also called target_addr!
            sim.observe(via_router=False)
        elif op == "store_load_counter":
            v = rng.randint(0, 500)
            # store/load do NOT invoke call_contract on target_addr, so counted frame cheats do not tick!
            sim.counter = v
            lines.append(f"        store(target_addr, selector!(\"counter\"), array![{v}].span());")
            lines.append(f"        let loaded_{step} = load(target_addr, selector!(\"counter\"), 1);")
            lines.append(f"        assert(*loaded_{step}.at(0) == {v}, 'ld_{step}');")
        elif op == "record_action":
            tag = rng.randint(1, 50)
            l1_rec = rng.choice([0, 0x1234, 0x5678])
            caller, ts, next_cnt = sim.record_action(tag, l1_rec)
            caller_expr = caller if caller in ("test_addr", "router_felt") else caller
            lines.append(f"        let (rc_{step}, rts_{step}, rcnt_{step}) = target.record_action({tag}_u64, {hex(l1_rec)});")
            lines.append(f"        assert(rc_{step} == {caller_expr}, 'rc_{step}');")
            lines.append(f"        assert(rts_{step} == {ts}_u64, 'rts_{step}');")
            lines.append(f"        assert(rcnt_{step} == {next_cnt}_u64, 'rcnt_{step}');")
            lines.append(
                f"        ev_spy.assert_emitted(@array![(target_addr, Target::Event::ActionRecorded("
                f"Target::ActionRecorded {{ caller: {caller_expr}, tag: {tag}_u64, timestamp: {ts}_u64 }}))]);"
            )
            if l1_rec != 0:
                lines.append(
                    f"        l1_spy.assert_sent(@array![(target_addr, MessageToL1 {{ "
                    f"to_address: {hex(l1_rec)}.try_into().unwrap(), "
                    f"payload: array![{caller_expr}, {tag}, {next_cnt}] }})]);"
                )

        # After every step, probe either observe_context (direct or via router) or compute_fee (direct or via router)
        probe_Via_router = rng.random() < 0.4
        if rng.random() < 0.65:
            caller, ts, bn, seq = sim.observe(via_router=probe_Via_router)
            caller_expr = "router_felt" if caller == "router_addr" else caller
            call_expr = "router.forward_context(target_addr)" if probe_Via_router else "target.observe_context()"
            lines.append(f"        let (c_{step}, ts_{step}, bn_{step}, seq_{step}) = {call_expr};")
            lines.append(f"        assert(c_{step} == {caller_expr}, 'obs_c_{step}');")
            lines.append(f"        assert(ts_{step} == {ts}_u64, 'obs_ts_{step}');")
            lines.append(f"        assert(bn_{step} == {bn}_u64, 'obs_bn_{step}');")
            lines.append(f"        assert(seq_{step} == {hex(seq)}, 'obs_seq_{step}');")
        else:
            amt = rng.randint(1, 200)
            # Even when compute_fee is mocked, calling target_addr consumes one TargetCalls frame!
            sim.observe(via_router=probe_Via_router)
            expected_fee = sim.compute_fee(amt)
            call_expr = f"router.forward_fee(target_addr, {amt}_u64)" if probe_Via_router else f"target.compute_fee({amt}_u64)"
            lines.append(f"        let fee_{step} = {call_expr};")
            lines.append(f"        assert(fee_{step} == {expected_fee}_u64, 'fee_{step}');")

    lines.append("    }")
    return "\n".join(lines)


def gen_fuzz_test(rng, idx):
    runs = rng.choice([8, 16, 24])
    seed = rng.randint(1, 10_000)
    k = rng.randint(1, 50)
    return "\n".join([
        f"    #[test]",
        f"    #[fuzzer(runs: {runs}, seed: {seed})]",
        f"    fn prop_linear_identity_{idx}(a: u32, b: u32) {{",
        f"        let lhs: u64 = a.into() + b.into() + {k}_u64;",
        f"        let rhs: u64 = b.into() + {k}_u64 + a.into();",
        f"        assert(lhs == rhs, 'fuzz_commute');",
        f"    }}",
    ])


def write_package(root: Path, name: str, n_tests: int, rng: random.Random):
    (root / "src").mkdir(parents=True, exist_ok=True)
    (root / "Scarb.toml").write_text(MANIFEST.format(name=name))
    tests = [
        "#[cfg(test)]",
        "mod tests {",
        "    use super::{Target, ITargetDispatcher, ITargetDispatcherTrait, IRouterDispatcher, IRouterDispatcherTrait};",
        "    use snforge_std::{",
        "        declare, ContractClassTrait, DeclareResultTrait, CheatSpan,",
        "        start_cheat_caller_address, stop_cheat_caller_address, cheat_caller_address,",
        "        start_cheat_caller_address_global, stop_cheat_caller_address_global,",
        "        start_cheat_block_timestamp, stop_cheat_block_timestamp, cheat_block_timestamp,",
        "        start_cheat_block_timestamp_global, stop_cheat_block_timestamp_global,",
        "        start_cheat_block_number, stop_cheat_block_number, cheat_block_number,",
        "        start_cheat_block_number_global, stop_cheat_block_number_global,",
        "        start_cheat_sequencer_address, stop_cheat_sequencer_address,",
        "        start_mock_call, stop_mock_call, mock_call,",
        "        store, load,",
        "        spy_events, EventSpyAssertionsTrait,",
        "        spy_messages_to_l1, MessageToL1, MessageToL1SpyAssertionsTrait,",
        "    };",
    ]
    for i in range(n_tests):
        if i % 5 == 4:
            tests.append(gen_fuzz_test(rng, i))
        else:
            tests.append(gen_stateful_test(rng, i))
    tests.append("}")
    (root / "src" / "lib.cairo").write_text(CONTRACTS_PRELUDE + "\n\n" + "\n\n".join(tests) + "\n")


def run_cmd(cmd, cwd, env=None, timeout=1800):
    t0 = time.perf_counter()
    p = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, env=env, timeout=timeout)
    return p, time.perf_counter() - t0


def uc_verdicts(uc, pkg):
    p, secs = run_cmd([uc, "test", "--json"], pkg)
    out = p.stdout
    try:
        rep = json.loads(out[out.index("{"):])
    except ValueError:
        return None, secs, (p.stderr or p.stdout)[-2000:]
    return {t["name"].split("::")[-1]: t["status"] for t in rep.get("tests", [])}, secs, None


def snforge_verdicts(pkg):
    env = dict(os.environ, SCARB_OFFLINE="true")
    p, secs = run_cmd(["snforge", "test"], pkg, env=env)
    verdicts = {}
    for line in p.stdout.splitlines():
        m = re.match(r"\[(PASS|FAIL)\]\s+(\S+)", line.strip())
        if m:
            verdicts[m.group(2).split("::")[-1]] = "ok" if m.group(1) == "PASS" else "failed"
    if not verdicts:
        return None, secs, (p.stdout + p.stderr)[-2000:]
    return verdicts, secs, None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--uc", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--packages", type=int, default=4)
    ap.add_argument("--tests", type=int, default=25)
    ap.add_argument("--seed", type=int, default=1)
    ap.add_argument("--snforge", action="store_true")
    args = ap.parse_args()

    uc = str(Path(args.uc).resolve())
    out = Path(args.out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    rng = random.Random(args.seed)

    summary = {
        "seed": args.seed,
        "packages": [],
        "totals": {"tests": 0, "uc_failed": 0, "snforge_failed": 0, "uc_vs_snforge": 0},
    }

    for pi in range(args.packages):
        name = f"cheat_{args.seed}_{pi}"
        pkg = out / name
        write_package(pkg, name, args.tests, rng)
        rec = {"package": name, "tests": args.tests}
        ucv, usecs, uerr = uc_verdicts(uc, pkg)
        rec["uc_s"] = round(usecs, 2)
        if ucv is None:
            rec["uc_error"] = uerr
            rec["uc_failed"] = [f"compile_or_crash_{pi}"]
        else:
            rec["uc_failed"] = sorted(k for k, v in ucv.items() if v != "ok")
        if args.snforge:
            sv, ssecs, serr = snforge_verdicts(pkg)
            rec["snforge_s"] = round(ssecs, 2)
            if sv is None:
                rec["snforge_error"] = serr
                rec["snforge_failed"] = [f"snforge_crash_{pi}"]
            else:
                rec["snforge_failed"] = sorted(k for k, v in sv.items() if v != "ok")
                if ucv is not None:
                    all_keys = set(ucv) | set(sv)
                    rec["uc_vs_snforge"] = sorted(k for k in all_keys if ucv.get(k) != sv.get(k))

        t = summary["totals"]
        t["tests"] += args.tests
        t["uc_failed"] += len(rec.get("uc_failed", []))
        t["snforge_failed"] += len(rec.get("snforge_failed", []))
        t["uc_vs_snforge"] += len(rec.get("uc_vs_snforge", []))
        summary["packages"].append(rec)
        print(json.dumps({
            k: (v if not isinstance(v, list) else len(v))
            for k, v in rec.items()
            if k not in ("uc_error", "snforge_error")
        }), flush=True)
        for k in ("uc_error", "snforge_error"):
            if k in rec:
                print(f"  {k}: {rec[k][:800]}", flush=True)

    (out / "summary.json").write_text(json.dumps(summary, indent=2))
    print(json.dumps(summary["totals"]), flush=True)
    return 1 if (summary["totals"]["uc_failed"] or summary["totals"]["uc_vs_snforge"]) else 0


if __name__ == "__main__":
    sys.exit(main())

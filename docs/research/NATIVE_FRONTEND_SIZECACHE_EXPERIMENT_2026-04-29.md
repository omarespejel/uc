# Native Frontend Size Cache Experiment (2026-04-29)

## Decision

Do not ship the Cairo `2.14` helper-lane size-estimate memoization patch.

The experiment added an in-process cache for exact `estimate_code_size()` results inside the
patched `cairo-lang-compiler` helper crate. The hypothesis was that repeated inline-size checks on
the same concrete functions would stop recompiling identical dummy Sierra/CASM fragments and reduce
`native_frontend_compile_ms` on heavy contracts like monero and braavos.

The real same-window harness did not support that hypothesis. The patch slightly improved one cold
braavos run, but it regressed monero and glint cold builds materially and also made warm-noop
behavior worse across most of the measured set.

## Scope

- `uc` worktree: `/Users/espejelomar/StarkNet/compiler-starknet/_pr_work/uc-perf-native-frontier-20260429`
- `uc` binary: `target/release/uc`
- Cairo lane: external helper `2.14.0`
- Contracts:
  - `monero_atomic_swap`
  - `braavos_account`
  - `zcash_relay`
  - `glint_contracts`
- Sample counts: `3` cold, `3` warm-noop
- Warm settle seconds: `2.2`
- Same host, same helper lane, same harness, sequential same-window reruns

## Input Artifacts

Reference rerun:

- `/Users/espejelomar/StarkNet/compiler-starknet/_pr_work/uc-perf-native-frontier-20260429/benchmarks/results/real-repo-bench-perf-frontier-20260429-samewindow-reference.json`
- `/Users/espejelomar/StarkNet/compiler-starknet/_pr_work/uc-perf-native-frontier-20260429/benchmarks/results/real-repo-bench-perf-frontier-20260429-samewindow-reference.md`

Patched rerun:

- `/Users/espejelomar/StarkNet/compiler-starknet/_pr_work/uc-perf-native-frontier-20260429/benchmarks/results/real-repo-bench-perf-frontier-20260429-samewindow-patched-sizecache.json`
- `/Users/espejelomar/StarkNet/compiler-starknet/_pr_work/uc-perf-native-frontier-20260429/benchmarks/results/real-repo-bench-perf-frontier-20260429-samewindow-patched-sizecache.md`

Earlier diagnostic baseline used to choose the experiment:

- `/Users/espejelomar/StarkNet/compiler-starknet/_pr_work/uc-perf-native-frontier-20260429/benchmarks/results/real-repo-bench-perf-frontier-20260429.json`
- `/tmp/uc-monero-size-trace-20260429.tsv`

## Why This Was Tried

The initial targeted sweep showed:

- `native_frontend_compile_ms` dominates heavy cold builds.
- Warm-noop behavior is already very strong on monero and braavos.
- The existing local monero trace and the refreshed `2026-04-29` trace both show repeated
  `estimate_size` / `dummy_program_for_size_estimation` events on a small set of concrete helper
  functions, especially:
  - `core::byte_array::ByteArrayImpl::at`
  - `core::bytes_31::Bytes31Impl::at`
  - `core::Felt252PartialEq::eq`
  - `core::bytes_31::one_shift_left_bytes_u128_nz`

That made exact size-estimate memoization the cheapest plausible helper-lane experiment to test
before touching broader lowering/inlining behavior.

## Result Summary

`uc` p50 deltas between the same-window reference helper and the patched helper:

| Contract | Cold `uc` p50 | Patched cold `uc` p50 | Delta | Warm `uc` p50 | Patched warm `uc` p50 | Delta |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `monero_atomic_swap` | `9908.728 ms` | `15948.330 ms` | `+6039.602 ms` (`+60.95%`) | `54.614 ms` | `82.833 ms` | `+28.219 ms` (`+51.67%`) |
| `braavos_account` | `3582.401 ms` | `3448.280 ms` | `-134.121 ms` (`-3.74%`) | `55.587 ms` | `133.359 ms` | `+77.772 ms` (`+139.91%`) |
| `glint_contracts` | `2757.957 ms` | `3021.751 ms` | `+263.794 ms` (`+9.56%`) | `59.471 ms` | `59.219 ms` | `-0.252 ms` (`-0.42%`) |
| `zcash_relay` | `603.250 ms` | `572.320 ms` | `-30.930 ms` (`-5.13%`) | `66.067 ms` | `108.962 ms` | `+42.895 ms` (`+64.93%`) |

Interpretation:

- The patch is not robust enough for production.
- A slight cold improvement on one or two contracts does not compensate for severe monero
  regression and broad warm-noop degradation.
- The cached exact-size path likely adds enough synchronization and/or keying overhead to cancel
  out the avoided work, while still missing the deeper lowering/inlining cost centers.

## Rejected Patch Shape

The rejected patch was a lane-only `cairo-lang-compiler` change that memoized the final integer
result of `estimate_code_size()` in a process-global cache keyed by:

- compiler DB instance pointer
- concrete function internal Salsa id

This patch is intentionally not checked in after the experiment.

## Recommendation

Do not pursue more wrapper-level or light helper-cache work for the current cold-build hotspot.

The next credible speed work remains deeper Cairo `2.14` helper-lane frontend profiling and
optimization, specifically around:

1. repeated lowering/inlining size-estimation work,
2. dummy Sierra generation churn,
3. avoiding exact-size estimation entirely for obviously tiny core helpers only if artifact
   equivalence and real-harness wins can both be proven.

Until that deeper helper-lane work exists, the honest product story is:

- `uc` already wins strongly on warm-noop heavy contracts,
- `uc` often wins on cold supported builds,
- the remaining monero/braavos cold bottleneck is still inside Cairo `2.14` frontend behavior,
  not in `uc` wrapper glue.

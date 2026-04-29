# Native Performance Frontier (2026-04-29)

## Decision

The next speed work should split into two separate tracks:

1. heavy-contract cold-build optimization inside the Cairo `2.14` helper frontend,
2. small-project warm-noop latency reduction in the wrapper/daemon path.

Do not mix them.

## Measured Contract Set

Current focused native-supported set:

- `monero_atomic_swap`
- `braavos_account`
- `zcash_relay`
- `glint_contracts`

Diagnostic benchmark lane and artifact:

- Lane id: `real-repo-bench-perf-frontier-20260429`
- Artifact location: `benchmarks/results/real-repo-bench-perf-frontier-20260429.json` in the local perf worktree
- Conditions: local Apple M3 Pro macOS workstation (18 GiB RAM), Cairo external-helper lane `2.14.0`, `--engine uc --daemon-mode off --offline`, `3` cold runs, `3` warm-noop runs, `2.2s` warm settle, sequential same-window reruns

## Baseline Result

Cold/warm `uc` versus Scarb from the focused 4-contract sweep:

| Contract | Cold speedup | Warm-noop speedup |
| --- | ---: | ---: |
| `monero_atomic_swap` | `1.30x` | `138.97x` |
| `braavos_account` | `2.34x` | `168.02x` |
| `zcash_relay` | `1.78x` | `0.61x` |
| `glint_contracts` | `3.77x` | `2.68x` |

Interpretation:

- `uc` is already very strong on warm-noop heavy contracts.
- `uc` is usually ahead on cold builds for the supported set.
- `zcash_relay` remains the important warm-noop outlier.

## Heavy Cold Path Finding

Cold heavy builds are still dominated by `native_frontend_compile_ms`.

Representative telemetry from the same artifact:

| Contract | `compile_ms` | `native_frontend_compile_ms` |
| --- | ---: | ---: |
| `monero_atomic_swap` | `7441.471 ms` | `7037.784 ms` |
| `braavos_account` | `12685.849 ms` | `11276.351 ms` |
| `glint_contracts` | `3260.599 ms` | `3170.764 ms` |
| `zcash_relay` | `473.007 ms` | `455.379 ms` |

Conclusion:

- for monero, braavos, and glint, the remaining cold cost is still overwhelmingly frontend work,
- session prep, fingerprinting, CASM, and artifact writes are secondary.

## Braavos and Monero Trace Shape

The helper-lane trace points for `estimate_size` / `dummy_program_for_size_estimation` show that
both monero and braavos repeatedly hit a relatively small shared set of core helpers.

Monero hot examples:

- `core::byte_array::ByteArrayImpl::at`
- `core::bytes_31::Bytes31Impl::at`
- `core::Felt252PartialEq::eq`
- `core::bytes_31::one_shift_left_bytes_u128_nz`

Braavos hot examples:

- `core::assert`
- `core::Felt252PartialEq::eq`
- `core::array::ArrayImpl::append`
- `core::starknet::storage::StorablePointerReadAccessImpl::read`

Braavos also surfaces app-level event helpers repeatedly, but the pattern is still the same:

- repeated size-estimation work,
- repeated dummy-program generation,
- concentrated in shared inlining/specialization decisions.

Trace provenance:

- Captured by running the local Cairo `2.14` helper with `UC_CAIRO214_SIZE_TRACE=/abs/path/to/trace.tsv` during daemon-free diagnostic builds
- These TSV files are local diagnostics only, not durable benchmark artifacts
- Example local filenames used during this pass: `uc-braavos-size-trace-20260429.tsv`, `uc-monero-size-trace-20260429.tsv`

## Rejected Optimization

The exact-size memoization helper patch was tested and rejected:

- [NATIVE_FRONTEND_SIZECACHE_EXPERIMENT_2026-04-29.md](/Users/espejelomar/StarkNet/compiler-starknet/_pr_work/uc-perf-native-frontier-20260429/docs/research/NATIVE_FRONTEND_SIZECACHE_EXPERIMENT_2026-04-29.md)

That experiment showed:

- a global helper cache around `estimate_code_size()` is not a safe speed win,
- it regressed monero badly and hurt warm-noop behavior,
- so the next frontend work must be narrower than “memoize exact size everywhere”.

## Small Warm Path Finding

Manual warm-noop `zcash_relay` runs show that fingerprinting is not the main remaining cost.

Manual warm cache-hit result:

- `elapsed_ms`: about `28.54 ms`
- measured telemetry sum:
  - `fingerprint_ms`: about `0.85 ms`
  - `cache_lookup_ms`: about `0.06 ms`
  - `cache_restore_ms`: about `0.08 ms`

So most of the remaining warm-noop latency on a tiny project is outside the measured compile/cache
phases:

- process startup,
- CLI/setup work,
- command orchestration overhead.

That means:

- further fingerprint micro-optimizations are unlikely to move the needle materially on tiny warm
  projects,
- if we want tiny-project warm wins, the best surface is process persistence or daemon/native
  orchestration, not hashing.

## Daemon Caveat

Daemon mode is not yet a clean native speed surface for external-helper Cairo `2.14` lanes.

Observed behavior:

- daemon startup was healthy locally,
- but `--daemon-mode require` on external-helper `2.14` builds downgraded to `uc_scarb`,
- so those numbers are not valid native-speed evidence.

This makes daemon work a separate compatibility/perf track:

- useful for agent UX if fixed,
- not currently usable for native launch claims on helper-backed lanes.

Daemon-free baseline command used for correctness/perf diagnosis:

- `uc build --engine uc --daemon-mode off --offline`

## Recommendation

### Track A: heavy cold builds

Focus on helper-lane frontend behavior, specifically:

1. instrument the callsites that invoke `estimate_size()` inside lowering/inlining/specialization,
2. prove which repeated calls are avoidable without changing generated Sierra/CASM,
3. test only narrow helper-lane patches against monero and braavos in same-window reruns.

Good candidates to inspect next:

- `cairo-lang-lowering/src/inline/mod.rs`
- `cairo-lang-lowering/src/specialization.rs`
- `cairo-lang-compiler/src/db.rs`

### Track B: tiny warm projects

Focus on process persistence and daemon/helper correctness, not fingerprint tweaks.

Specifically:

1. fix daemon native routing for external-helper `2.14` lanes,
2. re-benchmark `zcash_relay` and similar tiny projects with daemon-native warm hits,
3. only after daemon routing is correct, compare daemon warm-noop against `daemon-mode off`.

## What Not To Do Next

- do not spend more time on wrapper-level cache glue for monero/braavos cold builds,
- do not ship another helper-wide exact-size memoization experiment,
- do not treat daemon-backed Scarb fallback as native acceleration evidence.

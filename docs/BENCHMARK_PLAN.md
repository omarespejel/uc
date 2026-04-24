# Benchmark Plan

## Objective
Measure and prove that `uc` outperforms Scarb on real workflows while maintaining correctness.

## Matrix
1. `build.cold`: remove workspace build artifacts and build.
2. `build.warm_noop`: build again without changes.
3. `build.warm_edit`: edit one Cairo file and rebuild.
4. `metadata.online_cold`: metadata with empty global cache.
5. `metadata.offline_warm`: metadata with warm cache and `--offline`.

## Workloads
- `scarb/examples/hello_world`
- `scarb/examples/workspaces`
- `scarb/examples/dependencies` (metadata path)
- Optional heavy profile: `stwo_cairo_verifier` with required features.

## Output Artifacts
- JSON per run under `benchmarks/results/`.
- Markdown summary per run under `benchmarks/results/`.
- Reviewed baseline snapshots under `benchmarks/baselines/`.

## Baseline Rule
Before changing `uc` engine behavior, rerun baseline against current Scarb and snapshot results.

## Eligibility Rule
Do not mix native-ineligible manifests into `uc` vs Scarb speedup claims.
Benchmark reports must separate:
- native-eligible workloads that were actually measured against `uc` native build
- native-eligible workloads that failed during timed execution, with exit code and log path
- native-ineligible workloads that were skipped, along with the exact reason

Examples of native-ineligible reasons that should be reported explicitly:
- exact `cairo-version` mismatch against the native compiler
- legacy package editions (`2023_01`, `2023_10`, `2023_11`) without an exact `[package].cairo-version`

## Comparator Rule
Every build-path engine change must run dual-run comparison (`scarb-direct` vs `uc build`) and record:
- artifact mismatch count,
- diagnostics similarity,
- candidate vs baseline elapsed time.

## Gate Thresholds
- Gate A (Performance): warm rebuild p95 >= 40% faster than Scarb baseline.
- Gate B (Correctness): zero artifact hash mismatches and diagnostics parity >= 99.5%.

## Execution
```bash
./benchmarks/scripts/run_local_benchmarks.sh --matrix research --tool scarb
./benchmarks/scripts/run_local_benchmarks.sh --matrix research --tool uc
./benchmarks/scripts/run_dual_run_comparator.sh
```

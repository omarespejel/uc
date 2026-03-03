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

## Comparator Rule
Every build-path engine change must run dual-run comparison (`scarb-direct` vs `uc build`) and record:
- artifact mismatch count,
- diagnostics similarity,
- candidate vs baseline elapsed time.

## Gate Thresholds
- Gate A: warm rebuild p95 >= 40% faster than Scarb baseline.
- Gate A: zero artifact hash mismatches and diagnostics parity >= 99.5%.

## Execution
```bash
./benchmarks/scripts/run_local_benchmarks.sh --matrix research
./benchmarks/scripts/run_dual_run_comparator.sh
```

# Baseline Report (2026-03-03)

## Scope
Initial baseline and first `uc` speed signal run on the research matrix.

## Commands Used
```bash
./benchmarks/scripts/run_local_benchmarks.sh --matrix research --tool scarb
./benchmarks/scripts/run_local_benchmarks.sh --matrix research --tool uc
./benchmarks/scripts/compare_benchmark_results.sh --baseline <scarb.json> --candidate <uc.json> --out <delta.md>
```

## Scarb Baseline p95
- `hello_world`
  - `build.cold`: `3363.971 ms`
  - `build.warm_noop`: `51.707 ms`
  - `build.warm_edit`: `2547.85 ms`
- `workspaces`
  - `build.cold`: `960.396 ms`
  - `build.warm_noop`: `29.37 ms`
  - `build.warm_edit`: `362.695 ms`
- `dependencies`
  - `metadata.online_cold`: `1591.516 ms`
  - `metadata.offline_warm`: `22.075 ms`

## uc Signal p95
- `hello_world`
  - `build.cold`: `1997.631 ms`
  - `build.warm_noop`: `12.464 ms`
  - `build.warm_edit`: `420.32 ms`
- `workspaces`
  - `build.cold`: `846.555 ms`
  - `build.warm_noop`: `17.066 ms`
  - `build.warm_edit`: `373.838 ms`
- `dependencies`
  - `metadata.online_cold`: `1828.481 ms`
  - `metadata.offline_warm`: `29.521 ms`

## Interpretation
1. Warm no-op gains are strong and repeatable with `uc`.
2. Warm-edit gains are strong on `hello_world` and near-parity/slightly regressed on `workspaces` p95.
3. Metadata path is currently slower in `uc`; optimization is pending.

## Artifacts
- `benchmarks/baselines/2026-03-03-scarb-baseline.json`
- `benchmarks/baselines/2026-03-03-scarb-baseline.md`
- `benchmarks/baselines/2026-03-03-uc-baseline.json`
- `benchmarks/baselines/2026-03-03-uc-baseline.md`
- `benchmarks/baselines/2026-03-03-uc-vs-scarb-delta.md`

## Comparator Baseline
- `hello_world`: 0 artifact mismatches, diagnostics similarity 100%.
- `workspaces`: 0 artifact mismatches, diagnostics similarity 100%.
- Detailed artifacts:
  - `benchmarks/baselines/2026-03-03-compare-summary.md`
  - `benchmarks/baselines/2026-03-03-compare-hello_world.json`
  - `benchmarks/baselines/2026-03-03-compare-workspaces.json`

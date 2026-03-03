# Baseline Report (2026-03-03)

## Scope
Initial Scarb baseline run for `uc` program gate tracking.

Command used:
```bash
./benchmarks/scripts/run_local_benchmarks.sh --matrix research
```

## Key Results (p95)
- `hello_world`:
  - `build.cold`: 4511.191 ms
  - `build.warm_noop`: 47.721 ms
  - `build.warm_edit`: 1445.466 ms
- `workspaces`:
  - `build.cold`: 5644.983 ms
  - `build.warm_noop`: 66.484 ms
  - `build.warm_edit`: 3762.847 ms
- `dependencies`:
  - `metadata.online_cold`: 3698.907 ms
  - `metadata.offline_warm`: 145.476 ms

## Observations
1. Warm no-op builds are already low-latency; primary win opportunity is warm edit rebuild path.
2. Metadata has a large online-to-offline gap, validating lockfile-first and cache-first design priorities.
3. Workspaces warm-edit variance is high (max 3762.847 ms vs min 988.306 ms), so p95 optimization and stability are required.

## Next Steps
1. Implement sessionized compile MVP and rerun same matrix.
2. Add artifact and diagnostics comparator for correctness gates.
3. Track baseline deltas in milestone reviews.

## Artifacts
- `benchmarks/baselines/2026-03-03-scarb-baseline.json`
- `benchmarks/baselines/2026-03-03-scarb-baseline.md`

## Comparator Baseline
- `hello_world`: 0 artifact mismatches, diagnostics similarity 100%.
- `workspaces`: 0 artifact mismatches, diagnostics similarity 100%.
- Detailed artifacts:
  - `benchmarks/baselines/2026-03-03-compare-summary.md`
  - `benchmarks/baselines/2026-03-03-compare-hello_world.json`
  - `benchmarks/baselines/2026-03-03-compare-workspaces.json`

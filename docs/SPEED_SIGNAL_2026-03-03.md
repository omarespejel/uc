# Speed Signal Report (2026-03-03)

## Objective
Provide the first measurable speed signal for `uc` (`--engine uc`) versus Scarb on the research matrix.

## Inputs
- Baseline: `benchmarks/baselines/2026-03-03-scarb-baseline.json`
- Candidate: `benchmarks/baselines/2026-03-03-uc-baseline.json`
- Delta: `benchmarks/baselines/2026-03-03-uc-vs-scarb-delta.md`

## Key Results
1. Warm no-op builds are materially faster with `uc`.
- `hello_world` p95: `51.707 ms -> 12.464 ms` (`+75.89%` faster)
- `workspaces` p95: `29.37 ms -> 17.066 ms` (`+41.89%` faster)

2. Warm edit rebuilds show mixed but positive early signal.
- `hello_world` p95: `2547.85 ms -> 420.32 ms` (`+83.5%` faster)
- `workspaces` p95: `362.695 ms -> 373.838 ms` (`-3.07%`, regression)

3. Metadata path is not faster yet.
- `online_cold` p95: `1591.516 ms -> 1828.481 ms` (`-14.89%`)
- `offline_warm` p95: `22.075 ms -> 29.521 ms` (`-33.73%`)

## Interpretation
- `uc` local cache engine is already delivering strong warm no-op wins.
- Warm-edit performance still needs stabilization on multi-crate workspace patterns.
- Metadata acceleration is not implemented yet and remains a known gap.

## Next Actions
1. Improve `uc` warm-edit stability for workspace rebuilds (focus on fingerprint granularity and invalidation scope).
2. Optimize metadata command path or keep it routed to Scarb until native resolver fast path is implemented.
3. Keep dual-run comparator gate mandatory while iterating performance.

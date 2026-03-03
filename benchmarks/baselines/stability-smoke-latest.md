# Stability Benchmark Summary (20260303-171712)

- Generated at: 2026-03-03T23:22:51Z
- Matrix: smoke
- Cycles: 5
- Runs: 12
- Cold runs: 12
- CPU set: <none>
- Nice level: 0
- Build mode: offline
- UC daemon mode: require
- Run order: alternates per cycle (scarb-first, uc-first)
- Strict pinning: false
- Warm settle seconds: 2.2

| Scenario | Workload | Median p95 delta % | Mean p95 delta % | p95 delta stdev | Min p95 delta % | Max p95 delta % |
|---|---|---:|---:|---:|---:|---:|
| build.cold | scarb_smoke | 16.56 | -0.64 | 31.42 | -54.36 | 32.32 |
| build.warm_edit | scarb_smoke | 98.32 | 97.82 | 0.83 | 96.36 | 98.54 |
| build.warm_edit_semantic | scarb_smoke | -2.99 | -4.8 | 22.85 | -44.91 | 25.93 |
| build.warm_noop | scarb_smoke | 66.99 | 68.3 | 5.12 | 61.07 | 76.25 |

# Stability Benchmark Summary (20260303-172944)

- Generated at: 2026-03-03T23:35:26Z
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
| build.cold | scarb_smoke | -0.97 | 2.96 | 10.62 | -8.79 | 22.79 |
| build.warm_edit | scarb_smoke | 98.61 | 98.53 | 0.35 | 97.99 | 99.03 |
| build.warm_edit_semantic | scarb_smoke | 9.49 | 10.14 | 17.82 | -16.08 | 36.87 |
| build.warm_noop | scarb_smoke | 74.37 | 74.62 | 3.02 | 70.48 | 79.71 |

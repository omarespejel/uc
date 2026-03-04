# Stability Benchmark Summary (20260304-051007)

- Generated at: 2026-03-04T11:23:04Z
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
| build.cold | scarb_smoke | -3.55 | -78.55 | 118.96 | -261.07 | 47.69 |
| build.warm_edit | scarb_smoke | 99.43 | 98.85 | 1.4 | 96.07 | 99.76 |
| build.warm_edit_semantic | scarb_smoke | 49.18 | 42.02 | 33.49 | -17.04 | 77.56 |
| build.warm_noop | scarb_smoke | 81.34 | 70.18 | 32.98 | 6.04 | 95.99 |

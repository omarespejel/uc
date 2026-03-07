# Stability Benchmark Summary (20260307-095030)

- Generated at: 2026-03-07T16:06:14Z
- Matrix: smoke
- Cycles: 5
- Runs: 12
- Cold runs: 12
- CPU set: <none>
- Nice level: 0
- Build mode: offline
- UC daemon mode: off
- Run order: alternates per cycle (scarb-first, uc-first)
- Strict pinning: false
- Host preflight mode: warn
- Warm settle seconds: 2.2

| Scenario | Workload | Median p95 delta % | Mean p95 delta % | p95 delta stdev | Min p95 delta % | Max p95 delta % |
|---|---|---:|---:|---:|---:|---:|
| build.cold | scarb_smoke | 62.35 | 24.83 | 66.25 | -91.7 | 86.76 |
| build.returning_cold | scarb_smoke | 99.11 | 99.08 | 0.6 | 98.27 | 99.9 |
| build.warm_edit | scarb_smoke | 97.85 | 98.21 | 0.97 | 97.07 | 99.47 |
| build.warm_edit_semantic | scarb_smoke | 43.91 | 50.78 | 23.31 | 18.98 | 85.12 |
| build.warm_noop | scarb_smoke | 57.27 | 56.61 | 14.33 | 37.49 | 79.25 |

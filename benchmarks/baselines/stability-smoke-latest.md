# Stability Benchmark Summary (20260303-162831)

- Generated at: 2026-03-03T22:33:06Z
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
- Warm settle seconds: 2.2

| Scenario | Workload | Median p95 delta % | Mean p95 delta % | p95 delta stdev | Min p95 delta % | Max p95 delta % |
|---|---|---:|---:|---:|---:|---:|
| build.cold | scarb_smoke | 4.52 | -2.94 | 35.02 | -57.04 | 36.06 |
| build.warm_edit | scarb_smoke | 98.08 | 97.64 | 1.3 | 95.22 | 98.88 |
| build.warm_edit_semantic | scarb_smoke | -3.62 | -5.84 | 13.65 | -30.45 | 6.72 |
| build.warm_noop | scarb_smoke | 70.18 | 67.69 | 5.53 | 60.83 | 74.41 |

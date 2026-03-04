# Benchmark Delta Report

- Generated at: 2026-03-03T10:25:19Z
- Baseline: scarb 2.14.0 (682b29e13 2025-11-25)
- Candidate: uc (local build; scarb backend version: scarb 2.14.0 (682b29e13 2025-11-25))

| Scenario | Workload | Baseline p50 (ms) | Candidate p50 (ms) | p50 delta % | Baseline p95 (ms) | Candidate p95 (ms) | p95 delta % |
|---|---|---:|---:|---:|---:|---:|---:|
| build.cold | hello_world | 2140.786 | 858.489 | 59.9 | 3363.971 | 1997.631 | 40.62 |
| build.cold | workspaces | 889.514 | 835.775 | 6.04 | 960.396 | 846.555 | 11.85 |
| build.warm_edit | hello_world | 527.639 | 391.321 | 25.84 | 2547.85 | 420.32 | 83.5 |
| build.warm_edit | workspaces | 334.174 | 329.774 | 1.32 | 362.695 | 373.838 | -3.07 |
| build.warm_noop | hello_world | 25.61 | 11.509 | 55.06 | 51.707 | 12.464 | 75.89 |
| build.warm_noop | workspaces | 28.65 | 15.349 | 46.43 | 29.37 | 17.066 | 41.89 |
| metadata.offline_warm | dependencies | 20.268 | 19.512 | 3.73 | 22.075 | 29.521 | -33.73 |
| metadata.online_cold | dependencies | 1567.474 | 1758.143 | -12.16 | 1591.516 | 1828.481 | -14.89 |

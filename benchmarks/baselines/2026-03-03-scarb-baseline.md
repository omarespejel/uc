# SCARB Benchmark (20260303-042447)

## Environment
- Generated at: 2026-03-03T10:25:10Z
- Matrix: research
- Host: TMWM-G5FXKKXLXY
- Tool: scarb 2.14.0 (682b29e13 2025-11-25)
- Workspace root: /Users/espejelomar/StarkNet/compiler-starknet

## Summary
| Scenario | Workload | Runs | p50 (ms) | p95 (ms) | mean (ms) | min (ms) | max (ms) |
|---|---|---:|---:|---:|---:|---:|---:|
| build.cold | hello_world | 3 | 2140.786 | 3363.971 | 2244.033 | 1227.342 | 3363.971 |
| build.warm_noop | hello_world | 5 | 25.61 | 51.707 | 31.861 | 22.468 | 51.707 |
| build.warm_edit | hello_world | 5 | 527.639 | 2547.85 | 925.963 | 462.139 | 2547.85 |
| build.cold | workspaces | 3 | 889.514 | 960.396 | 905.594 | 866.872 | 960.396 |
| build.warm_noop | workspaces | 5 | 28.65 | 29.37 | 28.323 | 26.545 | 29.37 |
| build.warm_edit | workspaces | 5 | 334.174 | 362.695 | 333.655 | 316.571 | 362.695 |
| metadata.online_cold | dependencies | 3 | 1567.474 | 1591.516 | 1552.123 | 1497.38 | 1591.516 |
| metadata.offline_warm | dependencies | 5 | 20.268 | 22.075 | 19.93 | 17.674 | 22.075 |

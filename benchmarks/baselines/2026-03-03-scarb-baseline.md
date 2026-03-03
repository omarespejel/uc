# Scarb Baseline Benchmark (20260303-032209)

## Environment
- Generated at: 2026-03-03T09:23:03Z
- Matrix: research
- Host: TMWM-G5FXKKXLXY
- Tool: scarb 2.14.0 (682b29e13 2025-11-25)
- Workspace root: /Users/espejelomar/StarkNet/compiler-starknet

## Summary
| Scenario | Workload | Runs | p50 (ms) | p95 (ms) | mean (ms) | min (ms) | max (ms) |
|---|---|---:|---:|---:|---:|---:|---:|
| build.cold | hello_world | 3 | 3115.231 | 4511.191 | 3378.95 | 2510.427 | 4511.191 |
| build.warm_noop | hello_world | 5 | 39.342 | 47.721 | 40.451 | 32.565 | 47.721 |
| build.warm_edit | hello_world | 5 | 1290.758 | 1445.466 | 1251.64 | 960.974 | 1445.466 |
| build.cold | workspaces | 3 | 4573.176 | 5644.983 | 4510.685 | 3313.896 | 5644.983 |
| build.warm_noop | workspaces | 5 | 50.684 | 66.484 | 53.298 | 42.055 | 66.484 |
| build.warm_edit | workspaces | 5 | 1969.981 | 3762.847 | 2099.147 | 988.306 | 3762.847 |
| metadata.online_cold | dependencies | 3 | 2666.493 | 3698.907 | 2868.763 | 2240.888 | 3698.907 |
| metadata.offline_warm | dependencies | 5 | 41.2 | 145.476 | 59.948 | 27.528 | 145.476 |

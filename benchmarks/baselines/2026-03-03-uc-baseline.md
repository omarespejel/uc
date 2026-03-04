# UC Benchmark (20260303-042405)

## Environment
- Generated at: 2026-03-03T10:24:23Z
- Matrix: research
- Host: <redacted>
- Tool: uc (local build; scarb backend version: scarb 2.14.0 (682b29e13 2025-11-25))
- Workspace root: <workspace-root>

## Summary

| Scenario | Workload | Runs | p50 (ms) | p95 (ms) | mean (ms) | min (ms) | max (ms) |
|---|---|---:|---:|---:|---:|---:|---:|
| build.cold | hello_world | 3 | 858.489 | 1997.631 | 1193.849 | 725.428 | 1997.631 |
| build.warm_noop | hello_world | 5 | 11.509 | 12.464 | 11.306 | 9.959 | 12.464 |
| build.warm_edit | hello_world | 5 | 391.321 | 420.32 | 400.998 | 388.543 | 420.32 |
| build.cold | workspaces | 3 | 835.775 | 846.555 | 835.036 | 822.778 | 846.555 |
| build.warm_noop | workspaces | 5 | 15.349 | 17.066 | 15.188 | 13.248 | 17.066 |
| build.warm_edit | workspaces | 5 | 329.774 | 373.838 | 339.88 | 318.058 | 373.838 |
| metadata.online_cold | dependencies | 3 | 1758.143 | 1828.481 | 1753.715 | 1674.52 | 1828.481 |
| metadata.offline_warm | dependencies | 5 | 19.512 | 29.521 | 21.495 | 18.767 | 29.521 |

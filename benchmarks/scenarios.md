# Scenario Matrix

## S1 Warm No-op
- Setup: workspace already built once.
- Action: run build again with no changes.
- KPI: `build.warm.p50_ms`, `build.warm.p95_ms`.

## S2 Warm Single-file Edit
- Setup: workspace already built once.
- Action: edit one source file, run build.
- KPI: `build.warm.p95_ms`.

## S3 Cold Build
- Setup: clear local caches for tool-under-test.
- Action: run build once.
- KPI: `build.cold.p95_ms`.

## S4 Profile/Feature Change
- Setup: workspace already built in default profile.
- Action: run build with alternate profile/features.
- KPI: cache effectiveness + latency deltas.

## S5 CI Simulated
- Setup: clean checkout-like environment.
- Action: run standard pipeline.
- KPI: `ci.duration_percent_change`, cache-hit rate.


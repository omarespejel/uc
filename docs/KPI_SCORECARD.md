# KPI Scorecard

## Primary KPIs
- `build.warm.p95_ms`
- `build.warm.p50_ms`
- `build.cold.p95_ms`
- `artifact.hash_mismatch_count`
- `diagnostic.parity_percent`
- `ci.cache_hit_percent`
- `ci.duration_percent_change`
- `daemon.fallback_rate_percent`

## Secondary KPIs
- `max_rss_mb`
- `cpu_time_ms`
- `benchmark.flake_rate_percent`
- `migration.auto_success_percent`

## Reporting Cadence
- Weekly: trend chart and blockers.
- Milestone close: KPI deltas vs previous milestone baseline.

## Guardrails
- No release if `artifact.hash_mismatch_count > 0` on release matrix.
- No release if `diagnostic.parity_percent < 99.5`.


# KPI Scorecard

## Core KPIs
- `build.warm.p50_ms`
- `build.warm.p95_ms`
- `build.cold.p95_ms`
- `metadata.warm.ms`
- `artifact.hash_mismatch_count`
- `diagnostics.parity_percent`
- `ci.cache_hit_percent`
- `ci.duration_percent_change`

## Reliability KPIs
- `fallback.success_percent`
- `benchmark.flake_rate_percent`
- `daemon.crash_rate_percent`

## Delivery KPIs
- `milestone.commitment_completion_percent`
- `migration.auto_success_percent`

## Guardrails
- Release blocked if `artifact.hash_mismatch_count > 0`.
- Release blocked if `diagnostics.parity_percent < 99.5`.
- Rollout blocked if `fallback.success_percent < 100` on validation matrix.

## Reporting
- Weekly performance review with trend deltas.
- Milestone-close gate review with Go/No-Go recommendation.

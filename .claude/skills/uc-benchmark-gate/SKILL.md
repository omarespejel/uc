# uc Benchmark Gate

Use this skill when preparing or validating performance claims.

## Standard Lanes
- Smoke strict gate: `make benchmark-strict-smoke`
- Research strict gate: `make benchmark-strict-research`
- Fast local check: `make perf-fast`

## Rules
- Prefer pinned CPU and strict pinning when available.
- Keep daemon mode explicit in every report.
- Record whether the run was offline and whether native fallback was allowed.
- Report the exact scenario and sample counts.

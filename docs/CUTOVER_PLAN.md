# Cutover Plan

## Phase 1: Shadow Mode
- Run `uc` and Scarb in parallel in CI.
- Compare artifact hashes and diagnostics.
- Keep Scarb as execution source of truth.

## Phase 2: Default in Non-Prod CI
- `uc` becomes default in selected CI lanes.
- Scarb remains one-click fallback.
- Track reliability and latency regressions daily.

## Phase 3: Default in Mainline CI
- Promote `uc` to blocking mainline lane.
- Retain Scarb fallback lane for one milestone.

## Phase 4: Legacy Sunset
- Freeze Scarb lane updates.
- Publish migration completion report.
- Remove Scarb lane after stable milestone close.

## Exit Criteria
- KPI stage gates passed.
- No P0 parity regressions for two consecutive weeks.
- Migration automation >= 90% success.

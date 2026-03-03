# Cutover Plan

## Phase 1: Opt-in
- Early adopters use `uc` in non-blocking CI lane.
- Dual-run comparison retained.

## Phase 2: Default in CI
- `uc` becomes default in blocking CI lane.
- Legacy lane retained as fallback for one milestone.

## Phase 3: Legacy Sunset
- Freeze legacy tooling updates.
- Publish migration completion report.
- Remove legacy lane.

## Exit Criteria
- Migration auto-success >=90%.
- No blocker parity regressions for 2 consecutive weeks.
- KPI targets sustained for one milestone.


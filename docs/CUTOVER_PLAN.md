# Cutover Plan

## Phase 1: Shadow Mode
- Run `uc` and Scarb in parallel in CI.
- Compare artifact hashes and diagnostics.
- Keep Scarb as execution source of truth.
- Add read-only Scarb-compatible project import and report unsupported features explicitly.

## Phase 2: Default in Non-Prod CI
- `uc` becomes default in selected CI lanes.
- Scarb remains one-click fallback.
- Track reliability and latency regressions daily.
- Compare project-model metadata against Scarb metadata before command behavior changes.

## Phase 3: Default in Mainline CI
- Promote `uc` to blocking mainline lane.
- Retain Scarb fallback lane for one milestone.
- Retain resolver, lockfile, and artifact parity checks as blocking gates.

## Phase 4: Compatibility Maturity
- Freeze compatibility behavior only after parity gates are met.
- Publish migration completion report.
- Keep rollback steps documented for every default behavior change.

## Exit Criteria
- KPI stage gates passed.
- No P0 parity regressions for two consecutive weeks.
- Migration automation >= 90% success.
- Project metadata and lockfile parity pass on the supported corpus.

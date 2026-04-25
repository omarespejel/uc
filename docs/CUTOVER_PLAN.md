# Cutover Plan

## Phase 1: Shadow Mode
- Run `uc` and Scarb in parallel in CI.
- Compare artifact hashes and diagnostics.
- Keep Scarb as execution source of truth.
- Import Scarb manifests and lockfiles into the `uc` project model in read-only mode.

## Phase 2: Default in Non-Prod CI
- `uc` becomes default in selected CI lanes.
- Scarb remains one-click fallback.
- Track reliability and latency regressions daily.
- `uc metadata` may serve from the `uc` project model only where metadata parity is proven.

## Phase 3: Default in Mainline CI
- Promote `uc` to blocking mainline lane.
- Retain Scarb fallback lane for one milestone.
- Resolver/source-cache ownership can be enabled only for workspaces that pass lockfile and source parity gates.

## Phase 4: Legacy Sunset
- Freeze Scarb lane updates.
- Publish migration completion report.
- Remove Scarb lane after stable milestone close.
- Keep Scarb import support for legacy manifests even after default fallback removal.

## Exit Criteria
- KPI stage gates passed.
- No P0 parity regressions for two consecutive weeks.
- Migration automation >= 90% success.
- Project metadata parity passes for supported workspaces.
- Lockfile/source resolver parity passes for supported workspaces.

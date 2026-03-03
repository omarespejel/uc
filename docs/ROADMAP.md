# Roadmap

## Milestone 0: Program Foundations (2 weeks)
- Benchmark baseline harness.
- KPI dashboard schema.
- GitHub operating stack (labels, milestones, seeded issues, project).
- Architecture decision records (ADRs) scaffold.

## Milestone 1: Core Build Engine MVP (6 weeks)
- Sessionized compile daemon.
- Stable compile API (no process-local IDs over wire).
- Local content-addressed artifact cache.
- Dual-run comparator vs baseline.

## Milestone 2: Migration Tooling + Command Surface (6 weeks)
- `uc migrate` for existing projects.
- Build/check/test primary workflows.
- Error taxonomy and troubleshooting docs.

## Milestone 3: CI and Proving Acceleration (6 weeks)
- Remote cache.
- Cache policy controls and invalidation.
- Prove path acceleration and trace artifact reuse.

## Milestone 4: Cutover + Sunset (4 weeks)
- Org-level CI default switch.
- Migration completion dashboard.
- Legacy tooling deprecation plan.

## Stage Gates
- Gate A: p95 warm build improvement >=40% with correctness parity on MVP matrix.
- Gate B: migration success >=90% auto-convert on corpus.
- Gate C: CI cache-hit >=70% on mainline and PR runs.


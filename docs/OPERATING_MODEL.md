# Operating Model

## Planning Cadence
- Monday: planning and commit window.
- Wednesday: risk checkpoint.
- Friday: gate/status review.

## Work Breakdown
- Epic -> Feature -> Task/Bug.
- Every issue tied to a milestone.
- Every feature has measurable acceptance criteria.

## Default Delivery Flow
1. Define hypothesis and KPI target.
2. Implement with instrumentation.
3. Run benchmark/comparator matrix.
4. Merge only if gate criteria are met.

## Definition of Ready
- Clear problem statement.
- Quantified acceptance criteria.
- Milestone assignment and owner.

## Definition of Done
- Code + tests merged.
- Benchmark or comparator evidence attached.
- Docs and ADR updated.
- Issue/project state updated.

## Escalation Rules
- Any correctness mismatch creates P0 blocker.
- Two consecutive missed gate targets trigger replan within 48h.

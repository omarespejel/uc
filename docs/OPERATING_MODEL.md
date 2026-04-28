# Operating Model

## Planning Cadence
- Monday: planning and commit window.
- Wednesday: risk checkpoint.
- Friday: gate/status review.

## Work Breakdown
- Epic -> Feature -> Task/Bug.
- Every issue tied to a milestone.
- Every feature has measurable acceptance criteria.
- Agent-facing contract changes require doc updates in the same slice.

## Default Delivery Flow
1. Define hypothesis and KPI target.
2. Define the machine-readable contract and policy surface.
3. Implement with instrumentation.
4. Run benchmark/comparator matrix.
5. Merge only if gate criteria are met.

## Definition of Ready
- Clear problem statement.
- Quantified acceptance criteria.
- Milestone assignment and owner.
- Explicit statement of phase behavior (inspect/resolve/fetch/toolchain/build as relevant).

## Definition of Done
- Code + tests merged.
- Benchmark or comparator evidence attached.
- Docs, ADR, and agent instructions updated.
- Issue/project state updated.

## Escalation Rules
- Any correctness mismatch creates P0 blocker.
- Any silent fallback, silent network access in locked/offline lanes, or silent lockfile mutation is a blocker for agent-facing surfaces.
- Two consecutive missed gate targets trigger replan within 48h.

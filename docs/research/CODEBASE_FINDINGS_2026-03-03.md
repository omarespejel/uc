# Codebase Findings (2026-03-03)

## Repositories Reviewed
- `scarb`
- `cairo`
- `salsa`
- `stwo-cairo`
- `cairo-compiler-workshop`

## Boundary Findings
- Scarb drives compilation orchestration and currently creates compiler DB state per invocation path.
- Cairo compiler APIs accept an existing DB and can execute compilation against it.
- Salsa query architecture provides tracked invalidation and snapshot semantics.

## Caching Findings
- Scarb has local incremental artifact/fingerprint caching.
- Current gaps include process persistence and broader shared cache strategy.

## Risk Findings
- Plugin/proc-macro lifecycle is a major integration risk.
- Correctness drift must be caught with dual-run comparator.
- Performance claims need reproducible, script-driven baseline and trend tracking.

## Actionable Technical Direction
1. Build session manager with explicit session keying.
2. Build deterministic cache key model.
3. Build comparator before broad rollout.
4. Keep hard fallback path during migration.

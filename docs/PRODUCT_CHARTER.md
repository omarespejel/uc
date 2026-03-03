# Product Charter

## Vision
`uc` is the default Cairo build/proving platform for 2026+, optimized for developer speed, deterministic outputs, and cloud-native CI.

## Product Principles
1. Performance is a feature.
2. Determinism is non-negotiable.
3. Observability before optimization.
4. Migration must be low-friction.
5. Ship in measured, gated phases.

## North-Star Outcomes
- Developer loop: edit->build in <3s p95 on warm paths.
- CI loop: >=70% artifact reuse across branches.
- Reliability: automated fallback and no blocker regressions in rollout.

## Scope Boundaries
- In scope: build graph, compile daemon, caching, migration tooling, proving acceleration.
- Out of scope (initial): broad command parity guarantees with legacy tooling.


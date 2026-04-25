# Product Charter

## Product
`uc`: agent-first Cairo compiler and Scarb-compatible project tool.

## Vision
By default, Cairo developers and CI should run `uc` for package resolution, build, test, execute, prove, lint, and format workflows with better speed, stronger observability, and deterministic outputs.

The next product step is a first-party project model that can read existing Scarb manifests and lockfiles while preserving compatibility gates.

## Why Now
- Warm-path build latency is a direct productivity tax.
- Existing workflows duplicate work across commands and sessions.
- Agents need structured project state before they can safely fix, retry, benchmark, or stop.
- Teams need a cloud-ready cache model for shared CI acceleration.

## Product Principles
1. Performance is a first-class feature.
2. Determinism is non-negotiable.
3. Correctness gates are enforced before rollout.
4. Observability is required for every subsystem.
5. Migration should be measurable and reversible.
6. Project state should be typed and machine-readable before command defaults change.

## Success Outcomes
- Warm rebuild p95: at least 40% faster than Scarb baseline on target matrix.
- CI reuse: at least 70% cache hit rate on mainline + PR flows.
- Correctness gate target before default rollout: 0 artifact hash mismatches and diagnostics parity >= 99.5%.
- Reliability: fallback/recovery path success 100% in rollout matrix.

## Scope
- In scope:
  - Resolver/source engine.
  - Scarb-compatible project import.
  - `uc` project and lockfile model.
  - Build graph planner.
  - Compiler session manager/daemon.
  - Local + remote content-addressed cache.
  - Core command surface (`build/check/test/execute/prove/lint/fmt/metadata`).
- Out of scope for initial proof:
  - LSP-native implementation.
  - Advanced STWO proof caching beyond baseline hooks.
  - Changing defaults before parity gates pass.

## Stakeholders
- Developer productivity owners.
- CI/platform owners.
- Compiler/prover teams.

# Product Charter

## Product
`uc`: agent-first Cairo project control plane, package manager, and compiler/build platform.

## Vision
By default, agents and CI should run `uc` for project inspection, support detection, dependency resolution, source fetching, toolchain acquisition, build, check, test, execute, prove, lint, and format workflows with deterministic outputs, explicit fallback behavior, and machine-readable results.

The next product step is a first-party project model that can read existing Scarb manifests and lockfiles while preserving compatibility gates.

## Why Now
- Agents need deterministic execution surfaces, not human-only CLIs.
- Warm-path build latency is still a direct productivity and CI tax.
- Existing workflows duplicate work across commands and sessions.
- Cairo toolchain and package behavior need a single observable control plane.

## Product Principles
1. Stable JSON and schema versioning are product features.
2. Determinism is non-negotiable.
3. No silent fallback, network access, toolchain download, or lockfile mutation in locked lanes.
4. Performance is a first-class feature.
5. Observability is required for every subsystem.
6. Migration should be measurable and reversible.
7. Project state should be typed and machine-readable before command defaults change.

## Success Outcomes
- Agent-facing commands expose stable versioned machine-readable outputs.
- Pre-build support detection and fallback classification are trustworthy on the validation matrix.
- Warm rebuild p95: at least 40% faster than Scarb baseline on target native-supported workloads.
- Correctness gate before default rollout: 0 artifact hash mismatches and diagnostics parity >= 99.5%.
- Reliability: fallback/recovery path success 100% in rollout matrix.

## Scope
- In scope:
  - First-party project model.
  - Resolver/source engine.
  - Scarb-compatible project import.
  - `uc` project and lockfile model.
  - Source store and cache lifecycle.
  - Toolchain and helper-lane manager.
  - Build graph planner.
  - Compiler session manager/daemon.
  - Local + remote content-addressed cache.
  - Core command surface (`project inspect`, `support native`, `resolve`, `fetch`, `toolchain ensure`, `build/check/test/execute/prove/lint/fmt/metadata`).
- Out of scope for initial proof:
  - LSP-native implementation.
  - Advanced STWO proof caching beyond baseline hooks.
  - Changing defaults before parity gates pass.

## Stakeholders
- AI agents and automation owners.
- CI/platform owners.
- Developer productivity owners.
- Compiler/prover teams.

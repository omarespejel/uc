# Architecture Blueprint

## Goal
Build `uc` as an agent-first Cairo project control plane with a performance-first architecture.

## Top-Level Components
1. `uc-cli` / agent surface
- User command surface and machine-readable execution protocol.
- Primary operations: inspect, support, resolve, fetch, toolchain ensure, plan, build, compare, explain.

2. `project-model-core`
- Parse and normalize `Scarb.toml`, `Scarb.lock`, workspace topology, packages, targets, profiles, and source origins.
- Emit versioned machine-readable workspace state.

3. `resolver-core`
- Dependency resolution, lockfile management, and source metadata fetch.
- Deterministic and lockfile-first by default.

4. `source-store-core`
- Registry/git/path source acquisition.
- Shared source store with status, prune/GC, and offline-readiness checks.

5. `toolchain-core`
- Cairo toolchain selection, helper-lane acquisition, and policy enforcement.
- Explicit expected/found toolchain reporting.

6. `planner-core`
- Workspace graph expansion and compilation-unit planning.
- Stable fingerprints and deterministic scheduling.

7. `compile-service`
- Sessionized compiler state manager.
- Incremental rebuild path for warm edits.

8. `cache-core`
- Local CAS + remote CAS for artifacts and execution metadata.
- Artifacts keyed by source hash + compiler signature + options.

9. `comparator-core`
- Artifact and diagnostics parity checks against Scarb during rollout.

## Data Flow
1. CLI/API parses command, policy, and workspace root.
2. Project model loads manifest/lock/workspace state.
3. Support/toolchain preflight determines native eligibility before build.
4. Resolver and source store compute the locked fetch/resolve plan.
5. Planner computes deterministic execution units.
6. Compile service loads/creates a session and executes the plan.
7. Cache reads/writes artifact objects and metadata.
8. Comparator runs in dual mode until cutover confidence is met.

## Non-Goals (initial proof)
- LSP-native support.
- Full STWO proof cache optimization.

## Key Technical Constraints
- Machine-readable commands must be versioned and forward-compatible.
- Locked/offline lanes must not mutate lockfiles or touch the network implicitly.
- Fallback state must always be explicit and queryable.
- Session keys include workspace + compiler version + profile/features/cfg/plugin signature.
- Deterministic outputs across machines are mandatory.

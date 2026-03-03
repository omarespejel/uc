# Architecture Blueprint

## Goal
Build `uc` as a next-generation compiler platform with a performance-first architecture.

## Top-Level Components
1. `uc-cli`
- User command surface (`build/check/test/execute/prove/lint/fmt/metadata`).

2. `resolver-core`
- Dependency resolution, lockfile management, source metadata fetch.
- Deterministic and lockfile-first by default.

3. `planner-core`
- Workspace graph expansion and compilation-unit planning.
- Stable fingerprints and deterministic scheduling.

4. `compile-service`
- Sessionized compiler state manager.
- Incremental rebuild path for warm edits.

5. `cache-core`
- Local CAS + remote CAS.
- Artifacts keyed by source hash + compiler signature + options.

6. `comparator-core`
- Artifact and diagnostics parity checks against Scarb during rollout.

## Data Flow
1. CLI parses command + workspace context.
2. Resolver/planner compute deterministic build plan.
3. Compile service loads/creates session and executes plan.
4. Cache reads/writes artifact objects and metadata.
5. Comparator runs in dual mode until cutover confidence is met.

## Non-Goals (initial proof)
- LSP-native support.
- Full STWO proof cache optimization.

## Key Technical Constraints
- No process-local compiler IDs in wire contracts.
- Session key includes workspace + compiler version + profile/features/cfg/plugin signature.
- Deterministic outputs across machines are mandatory.

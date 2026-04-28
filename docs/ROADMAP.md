# Roadmap

## Milestone 0: Foundations and Agent Contract (2 weeks)
- Benchmark harness finalized with Scarb baseline report.
- KPI scorecard and gate criteria locked.
- Architecture decisions documented (ADR series).
- Agent-facing report/schema conventions and repo-local instructions established.

## Milestone 1: Build Proof MVP (4-6 weeks)
- `uc` compile service MVP with session lifecycle.
- Dual-run comparator for artifact and diagnostics parity.
- Performance gate run across benchmark matrix.
- Go/No-Go decision for platform continuation.

## Milestone 2: Agent-First Control Plane (4-6 weeks)
- First-party project inspection and native support classification.
- Stable machine-readable outputs and error taxonomy.
- Build planning surface with explicit side-effect reporting.
- Add `check`, `test`, `lint`, `metadata` on `uc` core path as the project model/parity gates allow.

## Milestone 3: Resolver, Fetch, and Toolchain Ownership (6 weeks)
- Resolver and source-fetch fast path (lockfile-first, bounded concurrency).
- Shared source store with status and prune/GC behavior.
- Toolchain/helper-lane ensure path with explicit policy controls.
- Metadata can be served from the project model behind `UC_METADATA_SOURCE=project-model`; see `docs/PROJECT_MODEL_STRATEGY.md`.

## Milestone 4: Command Surface Expansion and CI/Proving (6 weeks)
- Expand core command coverage one command at a time behind stable JSON/report contracts.
- Remote cache with policy controls and invalidation.
- `execute`/`prove` acceleration path integration.
- CI default lane pilot with rollback controls.

## Milestone 5: Cutover (4 weeks)
- `uc` default in org CI.
- Workspace migration completion dashboard.
- Legacy compatibility lane deprecation plan.

## Stage Gates
- Gate A: warm p95 improvement >= 40% with correctness parity on native-supported workloads.
- Gate B: support classification and fallback accounting are trustworthy on the validation corpus.
- Gate C: resolver/fetch/toolchain locked mode is deterministic and offline-ready.
- Gate D: CI cache hit >= 70% and stable for one full milestone.
- Gate E: project metadata and lockfile parity pass before default behavior changes.

## Project Model Track
- Phase 0: document the first-party project model contract.
- Phase 1: add read-only `uc project inspect` with stable JSON.
- Phase 2: compare project-model metadata against Scarb metadata on the support corpus.
- Phase 3: expose resolver and source-cache reports keyed by lockfile content.
- Phase 4: expand one command at a time only after parity gates pass.

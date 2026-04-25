# Roadmap

## Milestone 0: Foundations (2 weeks)
- Benchmark harness finalized with Scarb baseline report.
- KPI scorecard and gate criteria locked.
- PR/issue templates and milestone governance running.
- Architecture decisions documented (ADR series).

## Milestone 1: Performance Proof MVP (4-6 weeks)
- `uc` compile service MVP with session lifecycle.
- Dual-run comparator for artifact and diagnostics parity.
- Performance gate run across benchmark matrix.
- Go/No-Go decision for platform continuation.

## Milestone 2: Command Surface Expansion (6 weeks)
- Add `check`, `test`, `lint`, `metadata` on `uc` core path.
- Add read-only `uc project inspect --manifest-path <Scarb.toml> --format json`.
- Resolver and source-fetch fast path (lockfile-first, bounded concurrency).
- Metadata can be served from the project model behind an explicit gate.
- Error taxonomy and troubleshooting docs.

## Milestone 3: CI and Proving Acceleration (6 weeks)
- Remote cache with policy controls and invalidation.
- `execute`/`prove` acceleration path integration.
- CI default lane pilot with rollback controls.

## Milestone 4: Compatibility Maturity (4 weeks)
- `uc` default in org CI.
- Workspace migration completion dashboard.
- Compatibility lane governance and rollback policy.

## Stage Gates
- Gate A: warm p95 improvement >= 40% with correctness parity.
- Gate B: migration automation success >= 90% on corpus.
- Gate C: CI cache hit >= 70% and stable for one full milestone.
- Gate D: project metadata and lockfile parity pass before default behavior changes.

## Project Model Track

- Phase 0: document the first-party project model contract.
- Phase 1: add read-only `uc project inspect` with stable JSON.
- Phase 2: compare project-model metadata against Scarb metadata on the support corpus.
- Phase 3: expose resolver and source-cache reports keyed by lockfile content.
- Phase 4: expand one command at a time only after parity gates pass.

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
- Add read-only `uc project inspect` over Scarb manifests and lockfiles.
- Move `uc metadata` toward the `uc` project model with Scarb as comparator.
- Add `check`, `test`, `lint`, `metadata` on `uc` core path.
- Resolver and source-fetch fast path (lockfile-first, bounded concurrency).
- Error taxonomy and troubleshooting docs.

## Milestone 3: CI and Proving Acceleration (6 weeks)
- Remote cache with policy controls and invalidation.
- `execute`/`prove` acceleration path integration.
- CI default lane pilot with rollback controls.

## Milestone 4: Cutover and Sunset (4 weeks)
- `uc` default in org CI.
- `uc` owns project metadata and resolver for supported workspaces.
- Workspace migration completion dashboard.
- Scarb compatibility lane deprecation and sunset.

## Stage Gates
- Gate A: warm p95 improvement >= 40% with correctness parity.
- Gate B: migration automation success >= 90% on corpus.
- Gate C: CI cache hit >= 70% and stable for one full milestone.
- Gate D: project metadata and lockfile parity pass on the supported corpus before Scarb fallback removal.

## Scarb Sunset Track

See `docs/SCARB_SUNSET_STRATEGY.md`.

The intended sequence is compatibility first, replacement second, removal last:

1. Import and inspect existing `Scarb.toml` / `Scarb.lock` projects.
2. Generate `uc` project metadata without shelling out to Scarb.
3. Own resolver/source-cache behavior behind explicit gates.
4. Expand `uc` command surfaces.
5. Remove Scarb from default workflows only after parity and rollback gates pass.

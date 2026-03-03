# Research Synthesis (2026-03-03)

## Context
This synthesis consolidates all prior deep research completed in `/Users/espejelomar/StarkNet/compiler-starknet` and converts it into `uc` project guidance.

## Verified Technical Findings
1. Scarb currently rebuilds compiler database state on each invocation.
2. Cairo compiler already uses Salsa incremental query groups and supports snapshotting.
3. Existing Scarb incremental caching is artifact/fingerprint-oriented but not process-persistent.
4. Warm-path latency opportunity is primarily from session/state reuse, not resolver rewrite alone.
5. Proving workloads have high ROI for cache reuse, but should follow after core build proof.

## Measured Signals From Prior Runs
- Large warm-vs-cold gaps on heavy test flows.
- Metadata has major online vs offline delta.
- Lint/test workflows are strong candidates for incremental reuse wins.

## Product Implication
`uc` should be built as a next-generation compiler platform, with build-path acceleration as the first validated wedge and hard correctness gates.

## Program Decision
- Replacement strategy: full `uc` command surface over time.
- First gate: performance + correctness proof on build matrix.
- Expansion order: build proof -> command expansion -> CI/prove acceleration -> full cutover.

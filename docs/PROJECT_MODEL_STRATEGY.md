# Project Model Strategy

## Decision

`uc` needs a first-party, Scarb-compatible project model before it expands metadata, resolver, build, check, test, lint, and format surfaces.

The launch path remains Scarb-compatible because that is the safest adoption path for existing Cairo projects. The next product step is to make the project model explicit, typed, and agent-readable.

## Why This Matters

If agents only see build output, they have to infer project state from command failures. A first-party project model gives agents one stable control plane:

- package graph
- workspace graph
- target graph
- lockfile summary
- dependency and source summary
- native toolchain request
- diagnostics
- fallback status

## Non-Negotiables

1. Preserve Scarb compatibility while parity is being proven.
2. Do not change deployed artifacts without comparator evidence.
3. Keep `Scarb.toml` and `Scarb.lock` import readable throughout the transition.
4. Prefer additive `uc` commands before changing defaults.
5. Every project-model surface must emit stable JSON for agents.
6. Every compatibility fallback must be explicit in reports and benchmarks.
7. Local validation and benchmark evidence are the source of truth.

## Project Model Boundary

| Surface | Current State | Next Target | Gate |
| --- | --- | --- | --- |
| Project manifest | `Scarb.toml` is the source of truth | `uc project inspect` reads Scarb manifests and optional `Uc.toml` overlays | supported workspaces inspect without hidden mutation |
| Lockfile | `Scarb.lock` is consumed through compatibility paths | typed lockfile summary in stable JSON | lockfile parsing matches supported corpus |
| Metadata | `uc metadata` uses compatibility behavior | metadata can be served from the project model behind a gate | metadata parity passes on corpus |
| Build graph | build path selects native lane and records fallback | build planning consumes project model directly | artifact and diagnostic parity pass |
| Resolver and source cache | resolver behavior remains compatibility-oriented | deterministic source/cache report | resolver parity and cache invalidation tests pass |
| Commands | build/support/agent/replay/MCP surfaces exist | add check/test/lint/fmt/execute/prove gradually | command parity and JSON reports are stable |

## Phased Plan

### Phase 0: Contract

This document. No runtime behavior changes.

### Phase 1: Read-Only Project Model

Add `uc project inspect --manifest-path <Scarb.toml> --format json`.

It should parse `Scarb.toml`, read `Scarb.lock` when present, report package/workspace/target/dependency/toolchain metadata, classify unsupported features with stable diagnostics, and avoid mutating files.

### Phase 2: Metadata Parity

Make `uc metadata` capable of serving from the project model behind the `UC_METADATA_SOURCE` gate. Scarb metadata remains the comparison oracle while parity is measured.

The operational contract for this gate is:

- default/off: unset `UC_METADATA_SOURCE` or set `UC_METADATA_SOURCE=compatibility`
- enable project-model metadata: set `UC_METADATA_SOURCE=project-model`
- disable project-model metadata: unset `UC_METADATA_SOURCE` or set `UC_METADATA_SOURCE=compatibility`
- unsupported values must fail closed with a stable diagnostic before metadata is emitted

Example operator flow:

```sh
UC_METADATA_SOURCE=project-model uc metadata --manifest-path Scarb.toml --report-path /tmp/uc-metadata-project-model.json
UC_METADATA_SOURCE=compatibility uc metadata --manifest-path Scarb.toml --report-path /tmp/uc-metadata-compatibility.json
```

The gate can become default only after project-model metadata matches the compatibility output on the supported corpus and the rollout owner records the comparator evidence.

### Phase 3: Resolver And Source Cache

Add deterministic, offline-aware resolver/source-cache reporting keyed by lockfile content.

### Phase 4: Command Surface Expansion

Add `uc check`, `uc test`, `uc lint`, `uc fmt`, `uc execute`, and `uc prove` one at a time, each with `--manifest-path`, `--offline`, `--json`, `--report-path`, and stable diagnostics.

## PR Sequence

1. `project-model-contract`: add this strategy and align docs.
2. `uc-project-inspect-schema`: schema and fixtures.
3. `uc-project-inspect-command`: read-only command implementation.
4. `metadata-from-project-model`: gated metadata parity path.
5. `resolver-cache-readonly`: lockfile/source-cache reports.
6. `resolver-cache-active`: gated resolver/source behavior.
7. `command-surface-expansion`: add one command at a time.

## Launch Messaging Boundary

Safe launch wording:

> `uc` is a Scarb-compatible, agent-first Cairo compiler and project tool. It starts by accelerating and instrumenting existing Cairo workspaces, then adds a typed project model that agents can inspect before build, metadata, and benchmark work.

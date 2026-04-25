# Scarb Sunset Strategy

## Decision

`uc` is not a permanent Scarb wrapper.

The launch path stays Scarb-compatible because that is the lowest-risk adoption path for existing Cairo projects. The end state is different: `uc` owns the project, package, metadata, build, test, and benchmark surfaces, while Scarb becomes a compatibility importer and temporary fallback lane.

## Why This Matters

If `uc` only accelerates `scarb build`, then Scarb keeps owning the developer workflow, the metadata contract, and the package-manager failure modes. That limits how far an agent-first compiler can go.

The agent-first product needs one stable control plane:

- one project model
- one lockfile interpretation
- one diagnostics contract
- one benchmark/support matrix
- one replayable failure format
- one permission model for safe actions

Scarb compatibility is the migration bridge. It is not the destination.

## Non-Negotiables

1. Preserve Scarb compatibility until parity is proven.
2. Do not change deployed artifacts without comparator evidence.
3. Keep `Scarb.toml` and `Scarb.lock` import readable throughout the transition.
4. Prefer additive `uc` commands before changing defaults.
5. Every replacement surface must emit stable JSON for agents.
6. Every fallback to Scarb must be explicit in reports and benchmarks.
7. Local validation and benchmark evidence are the source of truth.

## Ownership Boundary

| Surface | Current state | Replacement target | Sunset condition |
| --- | --- | --- | --- |
| Project manifest | `Scarb.toml` is the source of truth | `uc project inspect` reads Scarb manifests and optional `Uc.toml` overlays | `uc` can inspect every supported workspace without shelling out to Scarb |
| Lockfile | `Scarb.lock` is consumed indirectly through Scarb metadata/build paths | `uc` has a typed lockfile model and stable JSON report | lockfile parsing matches Scarb on the supported corpus |
| Metadata | `uc metadata` still delegates to Scarb-compatible metadata paths | `uc metadata` is generated from the `uc` project model | metadata parity gate passes on the corpus |
| Build graph | Scarb remains fallback and source of truth for unsupported native cases | `uc` plans build graph directly, selects native lane before compile, and records fallback only when explicit | artifact hash and diagnostics parity pass before default rollout |
| Package/source resolution | Scarb registry and source behavior are still assumed | `uc` owns resolver, source fetch, cache keys, and offline policy | resolver parity and cache invalidation tests pass |
| Test/check/lint/fmt | Not yet owned by `uc` | `uc check`, `uc test`, `uc lint`, and `uc fmt` share the same project graph | command parity and agent JSON reports are stable |
| Failure handling | Scarb failures are wrapped when they reach `uc` | all critical paths produce stable diagnostic codes and replay bundles | agents can fix/retry/stop without parsing prose |

## Phased Plan

### Phase 0: Contract

Status: this document.

Define the replacement boundary, migration gates, and PR sequence. No runtime behavior changes.

### Phase 1: Read-Only Project Model

Add `uc project inspect --manifest-path <Scarb.toml> --format json`.

The command should:

- parse `Scarb.toml` directly
- load `Scarb.lock` when present
- report package, workspace, target, dependency, and toolchain metadata
- classify unsupported manifest features with stable diagnostics
- avoid mutating files
- avoid invoking Scarb unless an explicit compatibility probe is requested

Agent contract:

- agents call `uc project inspect` before `uc metadata`, `uc build`, or benchmark work
- reports include `schema_version`, `manifest_path`, `workspace_root`, `packages`, `targets`, `dependencies`, `lockfile`, `diagnostics`, and `fallback_used`

### Phase 2: Metadata Parity

Make `uc metadata` capable of serving from the read-only project model.

The gate is not speed. The gate is semantic parity:

- same package graph
- same target graph
- same relevant compiler/toolchain metadata
- explicit known differences
- stable JSON schema

Scarb remains a comparison oracle until this passes.

### Phase 3: Resolver And Source Cache

Move registry/source resolution into `uc`.

The resolver must be deterministic, offline-aware, and cache-keyed by lockfile content. It must expose why a package was selected and whether any network fetch happened.

Required report fields:

- source kind
- resolved revision/version
- cache hit/miss
- offline policy
- retryability
- fallback status

### Phase 4: Command Surface Ownership

Add the missing user-facing commands on the `uc` project graph:

- `uc check`
- `uc test`
- `uc lint`
- `uc fmt`
- `uc execute`
- `uc prove`

Each command must support:

- `--manifest-path`
- `--offline`
- `--json`
- `--report-path`
- stable diagnostics
- replayable failure bundles where useful

### Phase 5: Default Cutover

Promote `uc` from compatibility bridge to default project tool only after gates pass.

Scarb remains:

- an importer for legacy manifests
- a comparator oracle during rollout
- a fallback lane for one milestone after default cutover

Scarb is removed from default workflows only when supported workspaces have completed migration and fallback reports show no critical dependency on Scarb behavior.

## PR Sequence

1. `scarb-sunset-contract`
   - Add this strategy.
   - Update roadmap, product charter, cutover plan, command surface docs, and repo map.

2. `uc-project-inspect-schema`
   - Add JSON schema and docs for the read-only project model.
   - Add fixtures for simple package, workspace, local dependency, git dependency, registry dependency, target config, and unsupported manifest.

3. `uc-project-inspect-command`
   - Implement the command with direct manifest parsing.
   - Add regression tests in `crates/uc-cli/src/main_tests.rs`.

4. `metadata-from-project-model`
   - Teach `uc metadata` to serve from the project model behind an opt-in flag.
   - Add comparator fixtures against Scarb metadata.

5. `resolver-cache-readonly`
   - Add lockfile/source-cache parsing and read-only cache reports.

6. `resolver-cache-active`
   - Allow `uc` to resolve/fetch sources directly.
   - Keep Scarb fallback explicit and benchmarked.

7. `command-surface-expansion`
   - Add `check`, `test`, `lint`, `fmt`, `execute`, and `prove` one at a time.

## Launch Messaging Boundary

Do not say "`uc` replaces Scarb" at launch unless the replacement gates above have passed.

Correct launch wording before cutover:

> `uc` is a Scarb-compatible, agent-first Cairo compiler and project tool. It starts by accelerating and instrumenting existing Scarb workspaces, and it is designed to take over the project/package surfaces once parity gates pass.

Correct internal positioning:

> We are funding the path to sunset Scarb, but the migration strategy is compatibility first, replacement second, and removal last.


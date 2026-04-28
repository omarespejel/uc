# Agent-First Package Manager and Compiler Research (2026-04-28)

## Scope
This note captures the current best-practice direction for package managers and compiler-adjacent project tools, then maps those practices onto `uc`.

The focus is not generic package-manager history. It is the implementation pattern that is emerging in 2026 across leading tools and what changes when agents, not humans, are the main users.

## Executive Summary
The modern pattern is consistent:
1. Lockfile-first planning is the default.
2. Fetch is a first-class phase, separate from build/install.
3. Shared caches/stores are explicit subsystems.
4. Offline/frozen behavior is explicit and strict.
5. Runtime/toolchain acquisition is built into the product.
6. Machine-readable project metadata is a first-class API.
7. Supply-chain controls are moving into the default path.

For `uc`, this means package management must be treated as:
- project model,
- resolve,
- fetch,
- source store,
- toolchain ensure,
- policy enforcement,
- and machine-readable diagnostics.

It is not just "download packages faster".

## What the current leaders are doing

### 1. Lockfile-first planning
- `uv` exposes a universal lockfile and positions itself as a single workflow tool rather than a narrow installer.
- Cargo splits `metadata`, `fetch`, and `vendor`, with `cargo fetch` making later commands offline-capable while the lockfile is unchanged.
- `pnpm fetch` pulls packages from the lockfile into the store and explicitly ignores the package manifest for that step.
- `npm ci` requires a lockfile, errors on manifest/lock drift, and keeps installs frozen.
- Bun supports `bun install --lockfile-only` and writes a text lockfile by default.

Design implication for `uc`:
- locked mode should be normal,
- the lockfile should be the planning boundary,
- and any mutation should require an explicit update path.

### 2. Fetch is a first-class phase
The most modern tools no longer force users to discover package acquisition only through a later build/install command.

- Cargo: `fetch` and `vendor` are explicit.
- pnpm: `fetch` is explicit and optimized for build-layer reuse.
- Bun: lockfile-only still primes cache metadata and remote source data.

Design implication for `uc`:
- `resolve` and `fetch` should be real commands,
- not hidden implementation details behind `build`.

### 3. Shared caches and stores are product surfaces
- `uv` documents a global cache and explicitly states cache-safety properties: thread-safe, append-only, file-locking around environment mutation.
- Bun documents a global cache and cheap realization via hardlinks or `clonefile`.
- pnpm exposes store management, status, add, prune, and usage tracking.

Design implication for `uc`:
- source store and toolchain store need their own lifecycle,
- separate from build artifact cache,
- with status and prune/GC behavior that agents can query.

### 4. Toolchain ownership matters
- `uv` installs and manages Python versions and can auto-download them when necessary.

Design implication for `uc`:
- toolchain acquisition cannot stay an external afterthought if `uc` is supposed to be the control plane.
- Cairo version lanes, helper lanes, and expected/found reporting must be owned by `uc`.

### 5. Isolation and reproducibility are now default expectations
- Bun's isolated installs explicitly prevent phantom dependencies and improve monorepo determinism.
- `npm ci` is oriented toward automated and deployment environments.

Design implication for `uc`:
- undeclared or unexpected sources should be visible,
- source origins should be explicit,
- and agent workflows should prefer plan/fetch/build separation.

### 6. Supply-chain controls are entering the default workflow
- pnpm added `minimumReleaseAge` to delay freshly published packages.
- npm exposes provenance verification with `npm audit signatures`.

Design implication for `uc`:
- policy controls should exist for source trust, freshness, allowed hosts, and integrity/provenance checks.
- Even if all of them are not implemented immediately, the design should reserve that surface.

### 7. Machine-readable metadata is part of the product, not a convenience flag
- Cargo's `cargo metadata` is an explicit, versioned JSON interface for workspace and dependency state.
- MCP's current protocol split between resources, prompts, and tools matches the idea that a system should expose inspectable state separately from executable actions.

Design implication for `uc`:
- machine-readable output cannot be a secondary log format.
- it should be the primary contract for agent-facing commands.

## What "agent-first" changes

### Human-first CLI model
A human-first tool optimizes for:
- concise terminal output,
- implicit defaults,
- convenience over explicit planning,
- "just try the build" workflows.

### Agent-first control-plane model
An agent-first tool optimizes for:
- explicit phase boundaries,
- stable schemas,
- policy controls,
- preflight support classification,
- replayability,
- resumability,
- and remediation-grade failure payloads.

The consequence is important:
`uc` should be treated as a deterministic execution protocol with a CLI wrapper.

## Required agent-first surface for `uc`

### 1. Project model
`uc project inspect --json`
Must report:
- workspace root,
- packages and targets,
- lockfile presence and state,
- dependency source origins,
- offline readiness,
- selected profiles and relevant manifest schema markers.

### 2. Native support preflight
`uc support native --json`
Must report:
- native-supported / native-unsupported / fallback-likely / build-blocked,
- exact toolchain lane requested,
- expected vs found,
- whether a helper/toolchain acquisition step is required,
- whether fallback is allowed by policy.

### 3. Resolution
`uc resolve --locked --json`
Must report:
- selected graph,
- source origins,
- lockfile drift or mismatch,
- network requirement,
- and whether the result is reproducible offline.

### 4. Fetch
`uc fetch --locked --json`
Must report:
- what was fetched,
- what came from cache/store,
- unresolved/missing sources,
- integrity/provenance status where available,
- and whether the workspace is now offline-ready.

### 5. Toolchain acquisition
`uc toolchain ensure --json`
Must report:
- requested Cairo/helper lane,
- selected acquisition source,
- policy decisions,
- final installed state,
- and replay/remediation info on failure.

### 6. Build planning and execution
`uc build --plan-only --json`
`uc build --json`
Must report:
- exact plan,
- cache/store use,
- compile backend,
- fallback used or not,
- artifact/log paths,
- timings by phase,
- retryability,
- and stable error codes.

### 7. Failure explanation
`uc explain <id> --json`
Must let an agent recover the exact reason for:
- support rejection,
- resolution failure,
- fetch failure,
- toolchain failure,
- native compile failure,
- or fallback activation.

## Product rules that follow from this
1. JSON is primary; terminal prose is secondary.
2. Plan before action.
3. No silent fallback.
4. No silent network in locked/offline flows.
5. No silent lockfile mutation.
6. No silent toolchain download.
7. Every phase should be individually inspectable and replayable.

## Recommended roadmap shift for `uc`

### Keep
- Build proof and comparator work stay important.
- Native performance on supported modern Cairo lanes remains a core value proposition.

### Move earlier
1. First-party project model.
2. Stable schemas and error taxonomy.
3. Native support preflight.
4. Lockfile-first resolve.
5. Fetch and source store.
6. Toolchain ensure.

### Move later
- terminal UX polish,
- command aliases,
- convenience behavior that weakens explicit policy or state reporting.

## Concrete next implementation slices
1. Keep `AGENTS.md`, `.codex/START_HERE.md`, and `docs/agent/*.md` as the checked-in source of truth for agents and review bots.
2. Add first-party `project inspect` report with versioned schema.
3. Add explicit `support native` report.
4. Add `resolve` as a lockfile-first read path.
5. Add `fetch` plus source-store status/prune primitives.
6. Add `toolchain ensure` with expected/found toolchain reporting.
7. Make `build --plan-only` emit the same phase model used by real execution.

## Sources
Primary sources used:
- uv docs: https://docs.astral.sh/uv/
- uv cache: https://docs.astral.sh/uv/concepts/cache/
- uv Python versions: https://docs.astral.sh/uv/concepts/python-versions/
- uv features: https://docs.astral.sh/uv/getting-started/features/
- Cargo metadata: https://doc.rust-lang.org/nightly/cargo/commands/cargo-metadata.html
- Cargo fetch: https://doc.rust-lang.org/nightly/cargo/commands/cargo-fetch.html
- Cargo vendor: https://doc.rust-lang.org/cargo/commands/cargo-vendor.html
- pnpm fetch: https://pnpm.io/cli/fetch
- pnpm store: https://pnpm.io/cli/store
- pnpm settings (`minimumReleaseAge`): https://pnpm.io/settings#minimumreleaseage
- Bun lockfile: https://bun.sh/docs/pm/lockfile
- Bun global cache: https://bun.sh/docs/pm/global-cache
- Bun isolated installs: https://bun.sh/docs/pm/isolated-installs
- npm ci: https://docs.npmjs.com/cli/v11/commands/npm-ci/
- npm provenance: https://docs.npmjs.com/viewing-package-provenance/
- MCP specification: https://modelcontextprotocol.io/specification/2025-06-18

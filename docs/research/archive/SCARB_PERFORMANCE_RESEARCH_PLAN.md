# Scarb Replacement Research Plan (Performance-First)

## 1. Goal
Build a Scarb-compatible next-generation compiler plan focused on lower latency, lower network overhead, and better cache reuse for large Cairo workspaces.

Primary success criteria:
- Keep existing `Scarb.toml` / `Scarb.lock` compatibility.
- Preserve command compatibility for your real workflows first.
- Deliver measurable speedups against current Scarb baselines.

## 2. Current Local Facts (from this workspace)

### Repos available
- `cairo`
- `scarb`
- `salsa`
- `stwo-cairo`
- `cairo-compiler-workshop`
- Added for architecture benchmarking references:
  - `cargo`
  - `uv`

### Real command footprint in your repos (outside `scarb/`)
- `scarb execute`
- `scarb test`
- `scarb prove`
- `scarb lint`
- `scarb fmt`
- `scarb metadata`

### Compatibility risk already observed
- Installed binary: `scarb 2.14.0`
- `cairo/corelib/Scarb.toml` uses `edition = "2025_12"`, which fails on Scarb 2.14.0.
- Meaning: platform must support newer manifest/edition schema or it will block current Cairo repos.

### Baseline timings captured on this machine
- `stwo_cairo_verifier`:
  - `scarb test --package stwo_cairo_verifier --features qm31_opcode` (first run): `496.80s`
  - same command (warm): `76.03s`
  - `scarb lint --features=qm31_opcode --deny-warnings`: `20.81s`
  - `scarb fmt --check`: `1.76s`
- `scarb/examples/dependencies`:
  - `scarb --global-cache-dir /tmp/scarb_global_cache_deps metadata --format-version 1`: `1.73s`
  - same with `--offline`: `0.01s`

Interpretation:
- Large avoidable online overhead exists in metadata/dependency paths.
- Heavy commands are dominated by compile/lint/test pipelines, so package-manager-only optimization is not enough; we also need compiler/lint cache strategy.

## 3. Upstream Signals to Align With

From Scarb upstream:
- Issue #2452: "Use incremental cache in linter" (open).
- Issue #1640: PGO evaluation shows about 1.13x speedup on tested build scenario.
- Issue #1544: historical repeated registry download behavior (closed, but pattern matters for regression guards).
- Scarb benchmarking guidance exists (`guidelines/BENCHMARKING.md`) and recommends `hyperfine`, warm/cold split, and incremental/non-incremental split.

## 4. Replacement Strategy Choice

## Recommendation
Do **not** do a hard from-scratch CLI rewrite first.

Use a **hybrid rollout**:
1. Build a new high-performance core engine (resolver + cache + fetch scheduler + build planning).
2. Keep a Scarb-compatible CLI/front-end surface during migration.
3. Replace subsystems behind compatibility boundaries in phases.

Why this is best:
- Full rewrite has high parity risk (workspaces, profiles, plugins, lock semantics, execute/prove flows).
- Fork-only approach gives less architectural freedom and slower long-term gains.
- Hybrid lets you cut latency early while preserving ecosystem compatibility.

## 5. Architecture for "Ultra Performance"

## 5.1 Core design principles
- Lockfile-first execution in normal mode.
- Network-free default for locked dependencies unless explicitly refreshed.
- Content-addressed, globally shared cache.
- Persistent daemon for graph + metadata + compiler/linter state reuse.
- Bounded parallelism with backpressure (avoid `usize::MAX` fan-out behavior).
- Deterministic scheduling and reproducible outputs.

## 5.2 Subsystems to build
- `resolver-core`:
  - PubGrub-based solver with fast lock-preference and deterministic priority ordering.
  - Multi-source metadata prefetch queue with bounded concurrency.
- `source-core`:
  - Registry adapter with ETag/Last-Modified caching and local index DB.
  - Git adapter with strict locked-rev fast path and refresh TTL policy.
- `build-graph-core`:
  - Workspace graph + feature/profile expansion.
  - Stable hashing for unit fingerprints.
- `artifact-cache-core`:
  - Local CAS store (manifests, summaries, compiler/linter artifacts).
  - Separate "hot metadata cache" and "artifact cache" retention policies.
- `compat-cli`:
  - Command-level compatibility for `metadata/build/check/test/execute/prove/lint/fmt`.
  - Pass-through fallback to Scarb for not-yet-ported commands during migration.

## 6. Phased Plan (Order of Execution)

## Phase 0 (Week 0-1): Measurement Harness First
- Freeze benchmark projects:
  - `stwo-cairo/stwo_cairo_verifier`
  - `scarb/examples/dependencies`
  - 1 larger external Cairo project (OpenZeppelin/Alexandria style)
- Build reproducible benchmark scripts:
  - cold cache
  - warm cache
  - online
  - offline
  - incremental on/off
- Collect flamegraphs / trace events for:
  - `metadata`
  - `lint`
  - `test`
  - `execute/prove`

Deliverable:
- `benchmarks/baseline/*.json`
- Decision: performance budget per command and per subsystem.

## Phase 1 (Week 1-3): Fast Metadata + Resolver Vertical Slice
- Implement new resolver/source path for:
  - `metadata`
  - `fetch`
  - lockfile read/write
- Add strict `--locked` mode with no network touches by default.
- Add cache key tracing for every remote request.
- Validate exact parity of resolved graph against Scarb for selected projects.

Expected gain:
- `metadata` warm on locked workspaces: 5x to 20x depending on remote deps.

## Phase 2 (Week 3-6): Build Planning + Incremental Cache Integration
- Build deterministic compilation-unit planner.
- Implement build fingerprinting + artifact reuse in CAS.
- Integrate linter with dependency cache reuse (equivalent intent to upstream issue #2452).
- Add daemon mode to avoid repeated graph rebuild across commands.

Expected gain:
- `lint` warm: 1.3x to 2.0x
- `test` warm (no source changes): 1.2x to 1.8x

## Phase 3 (Week 6-9): Command Parity for Your Workflow
- Port and harden:
  - `execute`
  - `prove`
  - `test`
  - `lint`
  - `fmt`
  - `metadata`
- CI drop-in for `stwo-cairo` matrix.
- Failure-mode parity checks (same errors where required).

Deliverable:
- "compat mode" binary usable in CI for your current command matrix.

## Phase 4 (Week 9-12): Hardening + Rollout
- Dual-run mode in CI:
  - run both Scarb and platform on same commits
  - compare graph, artifacts, output metadata, exit status
- Progressive rollout:
  - developer opt-in
  - non-blocking CI lane
  - blocking CI lane
- Keep fallback path for unsupported corner cases.

## 7. Performance Targets

Targets should be validated against Phase 0 baselines:
- `metadata` (warm, locked): <= 150ms for medium workspace.
- `metadata` (warm, mixed git+registry): at least 5x faster than current online baseline.
- `lint` (warm): >= 30% faster.
- `test` (warm): >= 25% faster.
- First cold build/test: >= 20% faster or equivalent runtime with lower network bytes and better determinism.

## 8. Risks and Controls

### Risk: Manifest/edition drift
- Control: implement versioned manifest parser + compatibility tests against latest Cairo repos.

### Risk: Lockfile semantic mismatch
- Control: lockfile differential tests (Scarb vs uc) on real projects.

### Risk: Plugin/proc-macro edge cases
- Control: keep Scarb fallback for unsupported macro/plugin scenarios until parity suite is green.

### Risk: Network nondeterminism
- Control: `--locked`/offline-by-default behavior with explicit refresh command.

### Risk: Regressions hidden by aggregate timings
- Control: per-stage timers (resolve/fetch/plan/compile/lint/test-runtime) in CI benchmark reports.

## 9. Immediate Next Actions

1. Define the minimal v1 command scope:
   - `metadata`, `fetch`, `lint`, `test`, `execute`, `prove`.
2. Implement benchmark harness and publish baseline JSON.
3. Build resolver/source prototype with lockfile-first no-network fast path.
4. Integrate dual-run CI job against `stwo-cairo` command matrix.

## 10. Questions That Matter Before Implementation
- Should v1 prioritize CI speed (`lint/test/execute/prove`) over full developer CLI parity?
- Do you require strict byte-for-byte `scarb metadata` JSON compatibility, or semantic compatibility is enough?
- Are you OK with defaulting to no-network in locked mode (explicit refresh required)?
- Should we keep Scarb as automatic fallback for unsupported commands during transition?

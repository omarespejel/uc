# Cairo Source Deep Research for UC Supremacy (2026-03-06)

## Objective

Find the highest-ROI production path to make native `uc` consistently faster than Scarb, with focus on cold-path and warm-semantic builds.

## Repos and code paths audited

### `uc`

- `crates/uc-cli/src/main.rs`
  - native session rebuild: `build_native_compile_session_state` (around lines 6669-6804)
  - native compile execution: `run_native_build_inner` (around lines 7995-8260)

### Cairo compiler

- `crates/cairo-lang-starknet/src/compile.rs`
  - fresh DB path: `compile_path` (lines 42-60)
  - prepared DB compile: `compile_prepared_db` (lines 114-126)
- `crates/cairo-lang-compiler/src/lib.rs`
  - compile entrypoints and warmup behavior (lines 73-220)
- `crates/cairo-lang-lowering/src/cache/mod.rs`
  - cache load: `load_cached_crate_functions` (lines 63-108)
  - cache generation: `generate_crate_cache` (lines 125-180+)
  - important TODO: cache compatibility validation is not fully enforced (line 92)
- `crates/cairo-lang-lowering/src/db.rs`
  - cache read path used by lowering query: `cached_multi_lowerings` / `priv_function_with_body_multi_lowering` (lines 413-436)

### Scarb incremental internals

- `scarb/src/compiler/incremental/compilation.rs`
  - load caches + inject `cache_file` into crate config: lines 121-208
  - save caches with fingerprints: lines 277-402

### Upstream roadmap/issues

- Cairo incremental roadmap: `starkware-libs/cairo#7053` (open)
- Scarb constraints and experiments:
  - `software-mansion/scarb#588`
  - `software-mansion/scarb#2061`
  - `software-mansion/scarb#2416`
  - `software-mansion/scarb#2764`
- Cairo cache-related PRs:
  - `starkware-libs/cairo#7199`
  - `starkware-libs/cairo#8393`
  - `starkware-libs/cairo#8837`
  - `starkware-libs/cairo#9014`
  - `starkware-libs/cairo#9402` (open)

## What is already strong in `uc`

1. Persistent daemon worker/session cache.
2. Restart-safe changed-file journal + replay cursor.
3. Keyed source override updates and impacted-contract subset compile.
4. Native buildinfo/session image sidecars to reduce repeated workspace scanning.

These are the right structural pieces, but they do not yet capture the largest compiler-side cold rebuild savings.

## Main performance gap still open

`uc` still pays expensive cold rebuild costs when it must reconstruct a session:

1. Rebuild fresh `RootDatabase`.
2. Run `setup_project`.
3. Rehydrate source tracking metadata.
4. Rebuild dependency/compiler state when cache compatibility guardrails reject reuse.

Scarb wires cache lifecycle (fingerprint + load + inject + save) around Cairo lowering caches. `uc` now implements this lifecycle too, but still needs stronger compatibility-key hardening and broader benchmark-verified wins in cold paths.

## Highest-ROI recommendation

Harden and expand the native crate-cache lifecycle in `uc` using Cairo lowering cache APIs:

1. Keep loading cached crate blobs at session rebuild and inject as `cache_file`.
2. Keep saving refreshed crate blobs after successful native compile for affected crates.
3. Tighten compatibility keys and rejection telemetry so reuse is safe and explainable.

This is the largest remaining structural win before more invasive compiler changes.

## Production implementation blueprint

### Phase 1: Native crate-cache sidecar (core)

Add under `.uc/cache/native-session/crate-cache-v1/`:

1. Blob files keyed by deterministic cache key.
2. Index JSON with:
   - schema version
   - cairo-lang version
   - corelib signature
   - profile/cfg digest
   - plugin/fingerprint digest
   - crate identity
   - blob hash + size

Apply on session rebuild:

1. After `setup_project`, map `CrateInput -> CrateId`.
2. For each crate with a compatible entry, load blob as `BlobLongId::Virtual`.
3. Update crate config `cache_file` to the loaded blob.

Persist on successful compile:

1. For impacted crates (or all main crates initially), run `generate_crate_cache`.
2. Atomically write blob + index entry.
3. Keep size budget + eviction policy.

### Phase 2: Compatibility hardening

Because Cairo cache loader currently has a TODO around stronger mismatch checks, `uc` must enforce guardrails in its own sidecar key:

1. Include dimensions that can invalidate semantics:
   - cairo-lang version
   - corelib hash/version
   - profile/compiler flags
   - cfg set
   - dependency surface signature
2. Reject cache on any mismatch.
3. Emit reason-coded telemetry counters for every cache miss/reject reason.

### Phase 3: Cold-path startup trimming around cache load

1. When journal says no relevant changes and sidecar compatibility is valid, avoid full tracked-source walk.
2. Keep one conservative full scan fallback for ambiguous cursor/fresh-instance transitions.
3. Keep current keyed invalidation flow for correctness after edits.

### Phase 4: Gate before supremacy claims

Run locked benchmark gate only on pinned hosts:

- fixed daemon mode
- strict CPU pinning
- alternating order
- `12/12` warm + `12/12` cold samples
- pass criteria:
  - warm semantic p95 stable win
  - warm noop p95 >= +20%
  - no catastrophic cold outliers

## Expected impact (realistic)

Current caveat: the latest locked benchmark gate in this PR branch did not pass cleanly (cold-path regressions/outliers are still present), so the expectations below remain directional until the gate is green on pinned hosts.

Conservative expectation after Phase 1+2:

1. Largest gains on session rebuild and cold-like invocations where DB state must be rebuilt.
2. Warm/noop stays strong due existing daemon + keyed invalidation.
3. Warm semantic becomes less volatile because less repeated lowering work is recomputed after worker/session transitions.

This is the most credible path to durable supremacy, not a one-off benchmark spike.

## TDD plan (must-have)

1. Cache load path:
   - build once, persist cache, restart daemon, verify native compile succeeds with cache injection.
2. Compatibility invalidation:
   - mutate any key dimension (corelib hash/profile/cfg), assert cache rejected and rebuild occurs.
3. Safety:
   - corrupt blob/index, assert graceful ignore + rebuild (no panic).
4. Determinism:
   - compare native artifacts with and without cache injection for same inputs.
5. Eviction:
   - enforce cap and verify LRU/TTL pruning behavior.

## Why this is better than only wrapper optimizations

Wrapper-level improvements reduce overhead in low milliseconds. The remaining bottleneck is compiler query/lowering work on session rebuilds. Reusing Cairo crate caches targets that bottleneck directly.

## Primary sources

### Cairo/Scarb source and roadmap

1. Cairo incremental roadmap issue: <https://github.com/starkware-libs/cairo/issues/7053>
2. Cairo lowering cache implementation: <https://github.com/starkware-libs/cairo/blob/main/crates/cairo-lang-lowering/src/cache/mod.rs>
3. Cairo lowering DB cache usage: <https://github.com/starkware-libs/cairo/blob/main/crates/cairo-lang-lowering/src/db.rs>
4. Scarb incremental cache loading/saving: <https://github.com/software-mansion/scarb/blob/main/scarb/src/compiler/incremental/compilation.rs>
5. Scarb issue on coarse/full rebuild limitation: <https://github.com/software-mansion/scarb/issues/588>
6. Scarb issue on corelib partial compilation tryout: <https://github.com/software-mansion/scarb/issues/2061>
7. Scarb issue about shared compiler DB experiment: <https://github.com/software-mansion/scarb/issues/2416>

### Modern compiler/build-system practice references

1. Rust incremental model (red-green, stable hashing): <https://rustc-dev-guide.rust-lang.org/queries/incremental-compilation-in-detail.html>
2. Salsa algorithm and tracked-input model: <https://salsa-rs.github.io/salsa/reference/algorithm.html>
3. Salsa `synthetic_write` caveat: <https://docs.rs/salsa/latest/salsa/trait.Database.html>
4. Swift driver scheduling and dependency-graph behavior: <https://github.com/swiftlang/swift/blob/main/docs/DriverInternals.md>
5. Swift dependency analysis model: <https://github.com/swiftlang/swift/blob/main/docs/DependencyAnalysis.md>
6. Watchman fresh-instance/since semantics: <https://facebook.github.io/watchman/docs/cmd/query>
7. ThinLTO incremental cache and pruning: <https://clang.llvm.org/docs/ThinLTO.html>
8. TypeScript persisted incremental build state (`.tsbuildinfo`): <https://www.typescriptlang.org/tsconfig/incremental.html>
9. Bazel remote cache AC/CAS model: <https://bazel.build/versions/6.4.0/remote/caching>

## Implementation update (2026-03-07)

Applied in `uc` after the source audit:

1. Added a restart-safe buildinfo replay-seed path:
   - when sidecar source-root mtime mismatches, `uc` can still seed from persisted buildinfo only if a replayable source-journal delta exists (non-overflowed, pending, unambiguous cursor boundary).
   - this avoids unnecessary full workspace scans on daemon/session rebuild when changed-file replay is available.
2. Made eager keyed override slot priming opt-in:
   - default is now lazy slot creation (`UC_NATIVE_EAGER_KEYED_SLOT_PRIME=0` by default).
   - this removes cold-start startup work that scaled with total tracked source files.
3. Added TDD regression tests for replay-seed safety:
   - positive case: replay seed accepted only with pending journal delta.
   - negative case: ambiguous cursor boundary is rejected.

## SOTA refresh update (2026-03-07, Linux/macOS focus)

### Additional primary-source findings

1. Persistent worker architecture is still the highest ROI for startup-heavy builds.
   - Bazel documents persistent workers as a direct way to avoid repeated process startup and reports large gains in practice.
   - Swift driver supports compile-server mode for similar reasons.
2. Fine-grained invalidation + persisted incremental state is the durable pattern.
   - Rust incremental (red-green) and TypeScript `.tsbuildinfo` both keep durable keyed state and avoid recomputing unaffected units.
3. File watcher semantics must be cursor-based and restart-safe.
   - Watchman `since`/fresh-instance semantics reinforce the need for conservative fallback when cursor continuity is ambiguous.
4. Cold-path compile flags matter.
   - Clang docs show reduced debug-info modes (`-gline-tables-only`) to cut work when full debug metadata is not required.
5. Fast build tools minimize orchestration overhead and rely on precise dependency metadata.
   - Ninja emphasizes low overhead and explicit incremental scheduling.

### Applied in this pass

1. Added cold-path statement-location gating for native compile:
   - new env: `UC_NATIVE_CAPTURE_STATEMENT_LOCATIONS_ON_COLD` (default: `false`)
   - incremental changed-file builds still capture statement locations by default for impacted-unit precision.
2. Wired compile config explicitly with per-request capture decision:
   - `native_compiler_config(..., capture_statement_locations)` now receives the decision instead of always enabling.
3. Skipped dependency extraction work when statement-location capture is disabled:
   - avoids extra cold-path traversal of debug-info annotations when not needed.
4. Added tests:
   - `native_should_capture_statement_locations_with_flags_disables_when_base_disabled`
   - `native_should_capture_statement_locations_with_flags_defaults_off_on_cold`
   - `native_should_capture_statement_locations_with_flags_keeps_incremental_enabled`
5. Made dependency metadata probing opt-in on native path:
   - new env: `UC_NATIVE_DEPENDENCY_METADATA_ENABLED` (default: `false`)
   - by default `uc` now uses manifest-only dependency discovery and skips Scarb metadata probing during native context build, reducing cold startup overhead on supported cases.

### Measurement note

Quick smoke runs remain consistent on warm wins and noisy on cold:

1. warm_noop: strong win
2. warm_edit: strong win
3. warm_edit_semantic: modest win
4. cold: still behind Scarb and variable with short sample lanes

Conclusion: to move cold from "sometimes better" to "durably better", the next step is still deeper compiler-side reuse (persisted keyed compiler artifacts), not only wrapper-level startup cuts.

### Sources used in this refresh

1. Bazel persistent workers: <https://bazel.build/versions/7.1.0/remote/persistent>
2. Bazel scalability/perf concepts: <https://bazel.build/versions/8.4.0/rules/performance>
3. Rust incremental model: <https://rustc-dev-guide.rust-lang.org/queries/incremental-compilation-in-detail.html>
4. Swift driver internals (compile server): <https://github.com/swiftlang/swift-driver/blob/main/Sources/SwiftDriver/Driver/Driver.swift>
5. Watchman query semantics: <https://facebook.github.io/watchman/docs/cmd/query>
6. Ninja manual: <https://ninja-build.org/manual>
7. TypeScript incremental buildinfo: <https://www.typescriptlang.org/tsconfig/incremental.html>
8. Clang users manual: <https://clang.llvm.org/docs/UsersManual.html>
9. sccache README: <https://github.com/mozilla/sccache/blob/main/README.md>

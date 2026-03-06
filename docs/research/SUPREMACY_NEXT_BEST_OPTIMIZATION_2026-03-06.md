# Supremacy Next-Best Optimization Research (2026-03-06)

## Executive Decision

The next highest-ROI optimization is:

1. **Replace aggregate file-override input updates with keyed file-input invalidation in the native compiler path.**
2. **Back it with a restart-safe changed-file journal so keyed invalidation remains correct across daemon restarts.**

Why this is next:

- `uc` already has watcher deltas, impacted-subset contract compile, and persisted session metadata.
- The remaining high-cost invalidation point is still aggregate (`set_file_overrides(Some(map))`), which risks broader-than-needed recomputation.
- Modern compiler systems consistently win by combining long-lived workers with fine-grained dependency invalidation and persisted incremental state.

## Current `uc` State (Code Audit)

Positive (already in place):

1. File watcher + changed/removed journal delta path in daemon mode:
   - `native_take_source_journal_delta` in [`crates/uc-cli/src/main.rs`](/Users/espejelomar/StarkNet/compiler-starknet/uc/crates/uc-cli/src/main.rs:6052)
2. Full source-scan fallback when watcher is unavailable/overflowed:
   - [`crates/uc-cli/src/main.rs`](/Users/espejelomar/StarkNet/compiler-starknet/uc/crates/uc-cli/src/main.rs:6062)
3. Impacted-contract subset compile using source dependency index:
   - `native_impacted_contract_indices` in [`crates/uc-cli/src/main.rs`](/Users/espejelomar/StarkNet/compiler-starknet/uc/crates/uc-cli/src/main.rs:5094)
4. Persisted native session image (tracked sources + dependency index):
   - restore path in [`crates/uc-cli/src/main.rs`](/Users/espejelomar/StarkNet/compiler-starknet/uc/crates/uc-cli/src/main.rs:5520)

Remaining hotspot:

1. Incremental source edits still rewrite an aggregate override map and publish it as one input:
   - read/clone map: [`crates/uc-cli/src/main.rs`](/Users/espejelomar/StarkNet/compiler-starknet/uc/crates/uc-cli/src/main.rs:6313)
   - setter write: [`crates/uc-cli/src/main.rs`](/Users/espejelomar/StarkNet/compiler-starknet/uc/crates/uc-cli/src/main.rs:6327)
2. Session-image restore validity gate uses source-root directory mtimes:
   - [`crates/uc-cli/src/main.rs`](/Users/espejelomar/StarkNet/compiler-starknet/uc/crates/uc-cli/src/main.rs:4589)

## Deep Research Synthesis (Primary Sources)

## 1) Fine-grained invalidation is the core performance lever

1. Rust incremental model uses a dependency graph and red/green marking to avoid recomputing unchanged query results.
2. Salsa explicitly frames efficient incremental recomputation around tracked inputs + dependency tracking; coarse synthetic writes invalidate broadly.
3. Swift dependency analysis uses dependency edges and conservative fallbacks when dependency information is incomplete.

Implication for `uc`:

- Keep invalidation at per-file/per-unit key granularity; avoid aggregate single-input rewrites for file deltas.

## 2) Long-lived workers are required but not sufficient

1. Buck2 and Bazel persistent workers both rely on daemonized processes to amortize startup.
2. Worker models still emphasize isolation/serialization constraints and memory tradeoffs.

Implication for `uc`:

- Daemon is correct direction, but supremacy requires fine-grained invalidation inside the daemon worker, not only process reuse.

## 3) Persisted incremental state + deterministic cache keys are table stakes

1. TypeScript incremental mode persists graph metadata (`.tsbuildinfo`) for faster subsequent builds.
2. Go build cache includes explainability/debug knobs for why cache keys miss (`GODEBUG=gocachehash=1`).
3. Bazel remote caching emphasizes action cache + CAS and warns that non-hermetic env/tool differences reduce hit rates.

Implication for `uc`:

- Persist keyed invalidation metadata and emit reason-coded miss/invalidation telemetry as first-class signals.

## 4) Changed-file journals must handle fresh/overflow cases

1. Watchman documents `is_fresh_instance` behavior and freshness semantics for `since` queries.

Implication for `uc`:

- Journal cursor/reset handling should be explicit and measurable; fallback full scans are necessary but should be rare and reason-coded.

## 5) Artifact restore path should remain clone/link-first

1. ccache documents `file_clone`/`hard_link` restore options and direct/manifest modes.

Implication for `uc`:

- Keep reflink/hardlink-first restore path and ensure restore path is skipped on verified on-disk cache-hit.

## Recommended Production Plan

### Phase A (Highest ROI): keyed invalidation plumbing

1. **Patch/fork Cairo filesystem input layer used by native path** so changed files update keyed tracked inputs (not aggregate map replacement).
2. Keep compatibility bridge for unchanged code paths until parity is proven.
3. Add telemetry:
   - `native_invalidation_mode={keyed,aggregate_fallback}`
   - `native_changed_files_count`
   - `native_impacted_units_count`
   - `native_full_recompute_reason`

TDD gates:

1. Single-file edit only invalidates dependent units.
2. Unrelated file edit does not trigger impacted-unit rebuild.
3. Removed file invalidates exactly impacted dependents.
4. Aggregate fallback path remains correct under unsupported edge cases.

### Phase B: restart-safe journal + session-image safety

1. Persist watcher cursor/journal metadata under `.uc/cache/native-session/`.
2. On daemon start:
   - attempt journal replay from cursor;
   - if cursor invalid/fresh instance, force one bounded full scan and refresh cursor.
3. Tighten session-image restore guard:
   - include compact file index signature (count + aggregate hash of `(path,size,mtime)`), not only root directory mtimes.

TDD gates:

1. Edit while daemon is down -> next daemon build must detect and recompile impacted units.
2. Watcher overflow/fresh-instance path performs exactly one bounded full scan then returns to journal mode.
3. Corrupted session-image metadata is rejected safely (no false no-op).

### Phase C: benchmark and release gate

1. Locked benchmark lane:
   - pinned CPU host, strict pinning, fixed daemon mode, alternating order, 12/12 warm + 12/12 cold.
2. Pass criteria before supremacy claims:
   - warm-noop p95 median >= +20% vs Scarb baseline;
   - warm semantic edit stable win (no catastrophic outlier);
   - cold p95 variance envelope stable (no single-cycle regression spikes).

## Risk and Mitigation

1. Risk: Cairo fork/patch drift.
   - Mitigation: isolate keyed-invalidations patch behind thin adapter; track upstream Cairo changes in parity CI.
2. Risk: invalidation bug causes stale outputs.
   - Mitigation: add parity CI that compares native outputs vs Scarb artifacts on supported fixtures.
3. Risk: daemon memory growth from richer incremental state.
   - Mitigation: enforce caps + LRU/TTL + explicit eviction telemetry.

## Why this is better than chasing micro-optimizations now

Micro-optimizations (extra copy reductions, small startup tweaks) can help, but they do not remove the remaining structural invalidation cost. The largest remaining ROI is to ensure changed-file updates stay keyed from input to impacted unit compilation with restart-safe correctness.

## Primary Sources

1. Rust incremental compilation (red/green, query DAG): https://rustc-dev-guide.rust-lang.org/queries/incremental-compilation.html
2. Rust incremental in detail: https://rustc-dev-guide.rust-lang.org/queries/incremental-compilation-in-detail.html
3. Salsa overview: https://salsa-rs.github.io/salsa/overview.html
4. Salsa `Database` API (`synthetic_write`, eviction hooks): https://docs.rs/salsa/latest/salsa/trait.Database.html
5. Swift Driver docs: https://github.com/swiftlang/swift/blob/main/docs/Driver.md
6. Swift dependency analysis: https://github.com/swiftlang/swift/blob/main/docs/DependencyAnalysis.md
7. Buck2 daemon concept: https://buck2.build/docs/concepts/daemon/
8. Bazel persistent workers: https://bazel.build/docs/persistent-workers
9. Bazel remote caching (AC/CAS + key caveats): https://bazel.build/remote/caching
10. Watchman clockspec: https://facebook.github.io/watchman/docs/clockspec
11. Watchman query semantics (`is_fresh_instance`): https://facebook.github.io/watchman/docs/cmd/query
12. TypeScript project references/build mode: https://www.typescriptlang.org/docs/handbook/project-references.html
13. TypeScript incremental (`.tsbuildinfo`): https://www.typescriptlang.org/tsconfig/incremental.html
14. Go build/test caching: https://pkg.go.dev/cmd/go#hdr-Build_and_test_caching
15. ccache manual (`file_clone`, `hard_link`, direct mode): https://ccache.dev/manual/latest.html
16. Ninja manual (deps log / incremental semantics): https://ninja-build.org/manual.html
17. LLVM ThinLTO incremental cache model: https://clang.llvm.org/docs/ThinLTO.html

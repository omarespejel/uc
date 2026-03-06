# Supremacy Next-Step Research (2026-03-06)

## Why this update

We need the next execution step to maximize speed leadership against Scarb while keeping production safety and parity.

## Fresh Measurement Snapshot (Post-change)

Fast perf check (smoke, alternating order, `runs=8`, `cold-runs=4`, `UC daemon mode=require`) produced:

1. `build.cold` p95 median: `1131ms` (Scarb) vs `652ms` (`uc`) -> `1.73x` faster.
2. `build.warm_noop` p95 median: `36ms` (Scarb) vs `15ms` (`uc`) -> `2.40x` faster.
3. `build.warm_edit` p95 median: `737ms` (Scarb) vs `15ms` (`uc`) -> `49.13x` faster.
4. `build.warm_edit_semantic` p95 median: `728ms` (Scarb) vs `22ms` (`uc`) -> `33.09x` faster.

Artifacts:
- `benchmarks/results/perf-fast-delta-20260305-221724-scarb-first.md`
- `benchmarks/results/perf-fast-delta-20260305-221724-uc-first.md`

## What top compiler/build systems do (and why it matters for `uc`)

1. Rust incremental (`rustc`) and Salsa:
- Key point: track dependencies at query/input granularity and only recompute invalidated nodes.
- `uc` implication: eliminate coarse session-wide churn in incremental paths and prefer file-keyed/state-keyed updates.

2. Buck2 daemon + DICE model:
- Key point: daemonized, incremental graph state across requests is the default performance model.
- `uc` implication: keep daemon-first perf lane and avoid per-request O(N) bookkeeping on no-op cycles.

3. Bazel persistent workers:
- Key point: keep workers warm and reuse process memory/state to avoid repeated startup.
- `uc` implication: prioritize warm worker/session efficiency over local one-shot process micro-optimizations.

4. Watchman clockspec:
- Key point: file-change journal since cursor is preferable to full workspace walks each cycle.
- `uc` implication: watcher delta path should stay primary; full scans should remain fallback only.

5. TypeScript incremental/project references:
- Key point: keep persistent build state and rebuild only impacted project subgraphs.
- `uc` implication: continue contract-level impacted subset compile and burn down any full-rebuild fallbacks.

6. Clang precompiled headers / clangd preamble reuse:
- Key point: preserve expensive frontend state and reuse aggressively across edits.
- `uc` implication: continue reducing session-prepare overhead and avoid recomputing unchanged state.

7. pyperf + Criterion:
- Key point: claims must be backed by stable benchmark conditions and repeated samples.
- `uc` implication: keep pinned host and alternating-order lanes as the quality gate before "supremacy" claims.

## Applied in this pass

1. Removed unnecessary tracked-source map cloning on daemon no-op cycles in `with_native_compile_session`.
2. Added in-place source-journal delta application to avoid whole-map copies on changed-file updates.
3. Added unit regression to guarantee in-place and copy semantics are equivalent.

## Next highest-ROI implementation sequence

1. Extend the same "no coarse copy" approach to fallback full-scan transitions.
- Goal: avoid avoidable map churn when recovering from watcher overflow/error.

2. Promote daemon require mode to primary benchmark gate for performance claims.
- Keep a secondary daemon-off lane as compatibility regression signal, not primary speed KPI.

3. Burn down native fallback reasons on supported fixtures to near zero.
- Any fallback-to-Scarb in supported CI lanes should remain a hard failure.

4. Move from aggregate source-root change checks toward persistent file-keyed state transitions.
- Maintain deterministic, reason-coded invalidation telemetry per cycle.

## Sources

- Rust incremental internals: https://rustc-dev-guide.rust-lang.org/queries/incremental-compilation-in-detail.html
- Salsa overview: https://salsa-rs.github.io/salsa/
- Buck2 daemon: https://buck2.build/docs/concepts/daemon/
- Buck2 architecture: https://buck2.build/docs/developers/architecture/buck2/
- Bazel persistent workers: https://bazel.build/docs/persistent-workers
- Watchman clockspec: https://facebook.github.io/watchman/docs/clockspec
- TypeScript incremental builds: https://www.typescriptlang.org/tsconfig/incremental.html
- TypeScript project references: https://www.typescriptlang.org/docs/handbook/project-references
- Clang precompiled headers / modules internals: https://clang.llvm.org/docs/PCHInternals.html
- clangd index design: https://clangd.llvm.org/design/indexing
- ccache manual (`file_clone`, `hard_link`): https://ccache.dev/manual/latest.html
- pyperf system tuning: https://pyperf.readthedocs.io/en/latest/system.html
- Criterion FAQ: https://bheisler.github.io/criterion.rs/book/faq.html

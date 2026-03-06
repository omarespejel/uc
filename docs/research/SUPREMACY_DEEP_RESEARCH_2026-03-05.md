# Supremacy Deep Research Addendum (2026-03-05)

## Objective

Identify the highest-ROI, production-safe path to make `uc` consistently faster than Scarb on supported workloads, with repeatable benchmark evidence.

## What Top Compiler Implementations Actually Do

1. Fine-grained invalidation keyed by inputs, not coarse global rebuilds.
- Rust incremental compilation tracks query dependencies and reuses unchanged query results.
- Swift uses dependency analysis and incremental scheduling to rebuild only impacted files.
- TypeScript stores project graph state in `.tsbuildinfo` for incremental rebuild decisions.

2. Keep workers/process state warm across requests.
- Buck2 runs with a daemon (`buckd`) to avoid repeated startup and retain incremental state.
- Bazel persistent workers keep tool processes alive and reuse them across actions.
- Kotlin compiler daemon keeps compiler state in a long-lived process.

3. Drive rebuild decisions from reliable changed-file journals.
- Watchman clockspec/cursors provide "changes since cursor" semantics.

4. Keep artifact restore cheap.
- ccache supports `file_clone`/`hard_link` restore strategies to avoid full copy cost where supported.

5. Treat benchmarking as an engineering system, not a one-off run.
- pyperf guidance emphasizes pinned CPUs and stable host settings.
- Criterion documentation emphasizes statistical interpretation and noise control.

## Source-Grounded Implications for `uc`

1. The biggest remaining cold/warm ROI is not another micro-cache; it is strict file-keyed invalidation plus impacted-unit recompilation with persistent worker state.
2. Any fallback-to-Scarb on supported fixtures hides native performance and must be treated as a gate failure.
3. Benchmark claims require locked conditions (pinning, fixed mode, alternating order, enough samples) or results will drift.

## `uc` Next Implementation Sequence (Highest ROI)

1. Native-only reliability gate for supported fixtures.
- Keep `UC_NATIVE_BUILD_MODE=require` and `UC_NATIVE_DISALLOW_SCARB_FALLBACK=1` in CI.
- Add/keep tests that supported fixtures pass with Scarb unavailable.
- Fail CI on any fallback log line for supported lanes.

2. Replace remaining coarse invalidation surfaces.
- Ensure changed-file sets are the primary invalidation source.
- Keep full-scan fallback only for watcher overflow/error paths.
- Make fallback reason codes explicit in telemetry.

3. Expand impacted-unit compile coverage.
- Rebuild only impacted contracts/units from changed-file sets.
- Reuse unchanged artifacts and merge manifest entries deterministically.
- Keep conservative fallback to full compile when dependency index is incomplete.

4. Strengthen persistent worker stability.
- Keep daemon session/cache memory bounded (caps + TTL/LRU eviction).
- Expose high-water/eviction/fallback counters for operational visibility.
- Keep daemon behavior deterministic under long-running workloads.

5. Lock benchmark gate before "supremacy" claims.
- Pinned CPU, strict pinning, fixed daemon mode, alternating order, 12/12 warm+cold samples.
- Require warm p95 win threshold and reject catastrophic cold outliers.

## Practical Pass/Fail Bar

1. Native fallback rate on supported fixtures: near-zero and trending down.
2. Warm edit/no-op p95 vs Scarb: stable win across repeated gates.
3. Cold p95 variance: stable, no catastrophic single-cycle outliers.
4. Daemon memory behavior: bounded with visible eviction telemetry.

## References

- Rust incremental internals: https://rustc-dev-guide.rust-lang.org/queries/incremental-compilation-in-detail.html
- TypeScript incremental/project references: https://www.typescriptlang.org/tsconfig/incremental.html
- TypeScript project references: https://www.typescriptlang.org/docs/handbook/project-references
- Swift driver internals: https://raw.githubusercontent.com/swiftlang/swift-driver/refs/heads/main/Sources/SwiftDriver/Driver/Driver.md
- Swift dependency analysis: https://raw.githubusercontent.com/swiftlang/swift/refs/heads/main/docs/DependencyAnalysis.md
- Buck2 daemon (`buckd`) docs: https://buck2.build/docs/concepts/daemon/
- Buck2 architecture: https://buck2.build/docs/developers/architecture/buck2/
- Bazel persistent workers: https://bazel.build/docs/persistent-workers
- Watchman clockspec: https://facebook.github.io/watchman/docs/clockspec
- ccache manual (`file_clone`, `hard_link`): https://ccache.dev/manual/4.12.1.html
- pyperf system tuning: https://pyperf.readthedocs.io/en/latest/system.html
- Criterion FAQ: https://bheisler.github.io/criterion.rs/book/faq.html

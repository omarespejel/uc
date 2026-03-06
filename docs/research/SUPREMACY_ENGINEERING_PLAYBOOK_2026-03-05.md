# Supremacy Engineering Playbook (2026-03-05)

## Goal

Reach consistent, measurable performance leadership versus Scarb for supported workloads, without sacrificing correctness or upgrade cadence.

## What Top Compiler/Build Systems Do

1. Fine-grained invalidation, not coarse rebuilds.
   - Rust compiler incremental mode tracks dependencies at query granularity and reuses prior results when unchanged.
   - Salsa is explicitly designed for on-demand recomputation of only invalidated queries.
2. Keep long-lived worker state warm.
   - Buck2 runs as a daemon to avoid repeated process startup and maintain incremental state.
   - Kotlin compiler daemon keeps compiler process state and amortizes startup overhead.
3. Feed precise changed-file sets from a journal/watcher.
   - Watchman clockspec/cursors allow querying only files changed since the previous build.
4. Use content-addressed cache with cheap restore semantics.
   - ccache supports reflink/hardlink-style restore options to reduce copy overhead.
   - Go and Bazel rely on deterministic action/build caches to skip redundant work.
5. Benchmark under locked conditions.
   - pyperf and Criterion guidance both stress pinned/stable environments to reduce noise and avoid false regressions.

## Source -> Claim -> `uc` Action

| Source | What it contributes | Concrete `uc` action |
|---|---|---|
| Rustc dev guide: incremental compilation + red-green | Recompute only when dependency graph proves changed | Replace coarse session invalidation with per-file keys + dependency edges |
| Salsa book | Query-based incremental architecture for selective recompute | Model contract/unit compilation and dependency extraction as cached queries |
| Buck2 architecture (daemon + incrementality) | Persistent daemon with retained graph state | Keep native compiler DB/session in daemon across requests |
| Kotlin daemon docs | Production precedent for warm compiler workers | Enforce daemon-on for perf lanes; keep cold fallback lanes separate |
| Watchman clockspec docs | Reliable changed-file journal since cursor | Add daemon file-journal to avoid full workspace file walks |
| ccache manual (`file_clone`, `hard_link`) | Lower-cost artifact restore path | Prefer reflink/hardlink restore when filesystem supports it |
| Go build cache docs | Stable cache key discipline for skip/reuse | Tighten deterministic key inputs and cache miss explainability |
| Bazel remote cache docs | Action-result keying and cache hygiene | Extend cache metadata with action reason codes + periodic GC controls |
| pyperf system tuning + Criterion FAQ | Benchmark variance control in CI | Keep pinned CPU, fixed mode, alternating order, and 12/12 gate |

## Current Gap in `uc` (As of This Update)

1. Native dependency surface had false fallback causes in workspace path alias cases and from `dev-dependencies`.
2. Native path now gets farther, but still has semantic parity gaps (example: workspace fixture visibility behavior mismatch).
3. Cold-path speed ceiling remains capped while supported cases still require Scarb fallback in some scenarios.

## Completed in This PR Iteration

1. Canonicalized native local dependency source roots before dedupe/conflict checks.
2. Excluded `dev-dependencies` from native build dependency surface (runtime build path only).
3. Added regression tests for:
   - dev-dependency exclusion from native preflight surface
   - path alias canonicalization (e.g., `../` aliases) without false conflicts

## Next Execution Sequence (Highest ROI First)

1. Native-only gate for supported fixtures.
   - CI must run `NativeBuildMode::Require` and fail on any fallback-to-Scarb.
   - Add explicit test that supported fixture builds with Scarb unavailable.
2. File-keyed invalidation + daemon journal.
   - Persist `(path, size, mtime, content-hash)` map.
   - Pull changed-file set from watcher cursor, not full directory walk.
3. Impacted-unit compile only.
   - Map changed files -> impacted contracts/units.
   - Rebuild only impacted units; merge unchanged artifacts from previous state.
4. Persistent compiler worker state.
   - Keep compiler DB/session in daemon memory with bounded caches.
   - Add memory caps + LRU/TTL eviction + telemetry.
5. Native artifact parity completion.
   - Ensure native outputs match deployment expectations for supported targets.
   - Burn down fallback reason counters to near-zero before disabling default fallback.
6. Locked benchmark gate before supremacy claims.
   - Pinned CPU host, strict pinning, alternating order, 12/12 runs.
   - Require stable warm p95 win and no catastrophic cold outliers.

## Metrics to Track Weekly

1. Native fallback rate (% builds using Scarb fallback) on supported fixtures.
2. Warm noop p50/p95 delta vs Scarb.
3. Cold build p50/p95 delta vs Scarb with variance envelope.
4. Cache hit ratio (fingerprint + artifact restore).
5. Daemon memory high-water mark and eviction counts.

## References

- Rustc dev guide (incremental): https://rustc-dev-guide.rust-lang.org/queries/incremental-compilation-in-detail.html
- Rustc book (`-C incremental`): https://doc.rust-lang.org/rustc/codegen-options/index.html#incremental
- Salsa book: https://salsa-rs.github.io/salsa/
- Buck2 architecture: https://buck2.build/docs/developers/architecture/buck2/
- Kotlin daemon docs: https://kotlinlang.org/docs/kotlin-daemon.html
- Watchman clockspec/cursors: https://facebook.github.io/watchman/docs/clockspec
- ccache manual: https://ccache.dev/manual/4.12.1.html
- Go build cache: https://pkg.go.dev/cmd/go#hdr-Build_and_test_caching
- Bazel remote cache: https://bazel.build/remote/caching
- pyperf system tuning: https://pyperf.readthedocs.io/en/latest/system.html
- Criterion FAQ: https://bheisler.github.io/criterion.rs/book/faq.html

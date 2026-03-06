# uc Supremacy Research (2026-03-06)

## Goal
Ship a production-grade native-first `uc` pipeline that is consistently faster than Scarb on warm and materially better on cold, while staying network-compatible.

## What top compiler/build systems do (and what applies to `uc`)

### 1) Rust compiler: query DAG + red/green reuse + stable IDs
- `rustc` incremental is based on a query DAG and red/green marking (`try_mark_green`) to avoid recomputing unchanged queries.
- It persists incremental state across sessions and uses stable identifiers / stable hashing to compare query results across runs.
- It explicitly documents the cost tradeoff of hashing/fingerprinting, and uses selective persistence policies.

Why it matters for `uc`:
- Keep pushing from coarse invalidation to contract/file-keyed invalidation.
- Persist compact, stable fingerprints and dependency edges; avoid expensive full-hash work when not required.

### 2) Swift driver: compiler-as-orchestrator with dependency-aware scheduling
- Swift driver treats incremental builds as dependency-graph scheduling, not just file timestamp checks.
- It keeps a job graph, can skip jobs based on dependency analysis, and supports batching similar jobs to reduce process overhead.
- Swift docs emphasize that incremental builds need compiler-emitted dependency metadata and dynamic scheduling during the build.

Why it matters for `uc`:
- Keep `uc` daemon as the orchestrator of scheduling decisions.
- Prefer one orchestrated invocation with persistent state over repeated stateless subprocesses.
- Keep dependency metadata first-class for impacted-unit rebuild selection.

### 3) LLVM ThinLTO: summary-first + cached incremental backends + cache pruning
- ThinLTO is designed to be scalable and incremental: summaries are merged first, heavy optimization happens in parallel backends.
- Incremental performance depends on a dedicated cache plus explicit cache pruning policy.

Why it matters for `uc`:
- Preserve the split between lightweight planning/index and heavyweight compile.
- Keep cache policy explicit (size + age + prune cadence) and observable.

### 4) TypeScript build mode: orchestrator entrypoint + persisted project graph
- `tsc --build` behaves as a build orchestrator: detect up-to-date projects, build only stale ones in dependency order.
- TypeScript persists graph/build state (`.tsbuildinfo`) for fast subsequent runs and lower startup overhead.

Why it matters for `uc`:
- Treat daemon as canonical orchestrator and keep compact persisted "buildinfo" state for fast no-op decisions.
- Push startup work out of the CLI hot path.

### 5) Bazel remote cache: action cache + CAS + strict key determinism
- Bazel separates action metadata cache from CAS blobs.
- It warns explicitly that environment leakage and untracked external tools reduce cache correctness/hit rate.
- It has disk cache GC policy controls and concurrency safety caveats.

Why it matters for `uc`:
- Keep cache keys deterministic and explicitly scoped (env/toolchain/workspace).
- Keep AC/CAS model clear and instrument miss reasons.

### 6) Go build cache: automatic correctness dimensions + cache introspection
- Go cache keys include source/compiler/options changes and is safe for concurrent invocations.
- Go provides debug knobs to explain cache-hash decisions.

Why it matters for `uc`:
- Maintain explicit cache-key dimensions and add explainability for cache misses/fallbacks in diagnostics.

## Recommended next implementation order (highest ROI first)

1. **Daemon-first native orchestration path (reduce duplicate startup work):**
   - Keep moving eligibility/fallback decisioning to daemon-side state when safe.
   - Minimize client-side preflight/probe overhead for known fallback-native keys.

2. **Persisted native buildinfo sidecar (`.uc/native-buildinfo.json`):**
   - Store contract-level fingerprints + dependency edges + prior impacted set.
   - Fast no-op path without full workspace walk when journal indicates no changes.

3. **Impacted-unit compile hardening:**
   - Ensure partial rebuild selection uses stable, complete dependency index.
   - Keep conservative fallback only when index completeness is unknown.

4. **Cache key determinism audit + telemetry:**
   - Emit reason-coded cache miss/fallback counters (key mismatch dimensions, unsupported features, dependency-index incomplete, etc.).

5. **AC/CAS policy hardening:**
   - Continue hardlink/reflink-first restore path.
   - Enforce size+age GC policy for local cache and daemon shared cache with strict telemetry.

## Already implemented in this PR branch during this pass
- Stale fallback hint expiry and cleanup (`UC_NATIVE_FALLBACK_HINT_RETRY_SECS`, default 30m).
- Native-auto preflight short-circuit when fallback hint is active, to skip redundant preflight work.

## Sources
- Rust incremental compilation overview: https://rustc-dev-guide.rust-lang.org/queries/incremental-compilation.html
- Rust incremental in detail (persistence, stable IDs, fingerprints): https://rustc-dev-guide.rust-lang.org/queries/incremental-compilation-in-detail.html
- Swift Driver model and incremental builds: https://github.com/swiftlang/swift/blob/main/docs/Driver.md
- Swift driver internals (dependency graph scheduling/batching): https://github.com/swiftlang/swift/blob/main/docs/DriverInternals.md
- Swift dependency analysis model: https://github.com/swiftlang/swift/blob/main/docs/DependencyAnalysis.md
- Clang ThinLTO (incremental cache + pruning): https://clang.llvm.org/docs/ThinLTO.html
- TypeScript project references + build mode: https://www.typescriptlang.org/docs/handbook/project-references.html
- TypeScript incremental (`.tsbuildinfo`): https://www.typescriptlang.org/tsconfig/incremental.html
- Bazel remote caching (AC+CAS, key pitfalls, GC): https://bazel.build/remote/caching
- Go build/test caching (correctness dimensions, debug knobs): https://pkg.go.dev/cmd/go/
- Ninja dep handling and startup optimization (`deps` DB): https://ninja-build.org/manual

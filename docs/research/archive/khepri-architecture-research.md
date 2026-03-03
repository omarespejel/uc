# Khepri: Next-Generation Cairo Build Acceleration Layer

**Research & Architecture Document**
**Date:** 2026-03-03
**Status:** Research complete, ready for design review

---

## 1. Executive Summary

After deep exploration of five codebases (Scarb, Cairo compiler, Salsa, STWO-Cairo, and the Cairo compiler workshop), this document proposes **Khepri** — a persistent compilation daemon that sits between Scarb and the Cairo compiler to dramatically accelerate builds, unify the language server, and extend caching to the STWO proving pipeline.

**Key finding:** The Cairo compiler already uses Salsa 0.26.0 with 100+ tracked queries across 6 query groups. Scarb already uses PubGrub and has crate-level caching. The architectural gap is not in individual components — it's in the **lack of a persistent process that holds the Salsa Database across builds and shares it with the language server**.

**Estimated impact:**
- Warm rebuild after single-file edit: **~1-3s** (vs current ~15-30s)
- IDE + build memory: **~50% reduction** (shared database)
- CI with remote cache: **~80% reduction** in compilation time for unchanged deps
- STWO reproving with cached traces: **minutes saved per proof cycle**

---

## 2. What We Found: Current Architecture

### 2.1 Scarb's Compilation Pipeline (Verified)

```text
scarb build
  → ops::compile()                              [scarb/src/ops/compile.rs:119]
  → ops::resolve_workspace()                    [PubGrub resolver]
  → ops::generate_compilation_units()           [per-target per-package]
  → compile_units()                             [spawns thread per unit]
    → compile_cairo_unit_inner()                [scarb/src/ops/compile.rs:271]
      → load_incremental_artifacts()            [check fingerprint cache]
      → build_scarb_root_database()             [scarb/src/compiler/db.rs:37]
        → RootDatabase::builder()               [cairo-lang-compiler crate]
        → .detect_corelib()
        → .with_optimizations()
        → .build()                              [creates NEW Salsa Database]
      → apply_plugins()                         [macro + analyzer plugins]
      → compile_prepared_db_program_artifact()  [cairo-lang-compiler::lib.rs]
      → save_incremental_artifacts()            [write fingerprint + cache]
```

**Critical observation:** On every `scarb build`, a **new RootDatabase is created from scratch**. Even with crate-level caching, the database construction, corelib loading, and plugin initialization happens every time. This is the primary latency source for warm builds.

### 2.2 The Scarb ↔ Cairo Compiler Boundary

Scarb invokes the Cairo compiler **as a library** (not subprocess). The boundary:

| Direction | Data | Function |
|-----------|------|----------|
| Scarb → Cairo | `&dyn CloneableDatabase` (Salsa DB) | `compile_prepared_db_program_artifact()` |
| Scarb → Cairo | `Vec<CrateId>` | Main crates to compile |
| Scarb → Cairo | `CompilerConfig` | Gas, debug, diagnostics settings |
| Cairo → Scarb | `ProgramArtifact` (Sierra JSON + CASM) | Return value |
| Cairo → Scarb | Diagnostics | Via callback in CompilerConfig |

**No serialization at boundary.** Pure in-memory library calls with shared Salsa Database.

Source: `scarb/src/compiler/compilers/lib.rs:76`, `starknet_contract/compiler.rs:131`

### 2.3 Cairo Compiler's Salsa Architecture (Verified)

**Salsa version:** 0.26.0 (modern, proc-macro based)

**RootDatabase** (`cairo-lang-compiler/src/db.rs:68`):
```rust
#[salsa::db]
#[derive(Clone)]
pub struct RootDatabase {
    storage: salsa::Storage<RootDatabase>,
}
```

**Six query groups, layered by dependency:**

```text
FilesGroup          [INPUT: crate_configs, file_overrides, flags, cfg_set]
    ↓
ParserGroup         [TRACKED: file_syntax, file_module_syntax, diagnostics]
    ↓
DefsGroup           [INPUT: macro_plugins | TRACKED: ~50 symbol table queries]
    ↓
SemanticGroup       [INPUT: analyzer_plugins | TRACKED: type checking, resolution]
    ↓
LoweringGroup       [INPUT: optimizations | TRACKED: lowered_body (4 stages), borrow_check]
    ↓
SierraGenGroup      [TRACKED: sierra type/function generation]
```

**Input queries (what triggers invalidation):**

| Input | Location | What It Controls |
|-------|----------|-----------------|
| `crate_configs` | FilesGroup | Which crates exist, their roots |
| `file_overrides` | FilesGroup | File content (designed for LSP!) |
| `flags` | FilesGroup | Compiler flags |
| `cfg_set` | FilesGroup | Conditional compilation |
| `macro_plugins` | DefsGroup | Which macros are active |
| `analyzer_plugins` | SemanticGroup | Which analyzers run |
| `optimizations` | LoweringGroup | Optimization config |

**Derived queries (automatically cached + invalidated):**
100+ queries across all groups. Salsa tracks which inputs each query accessed and only recomputes when those specific inputs change.

### 2.4 Can the Database Be Long-Lived? YES.

Confirmed from source:

1. **RootDatabase implements `Clone`** — enables snapshots for parallel reads
2. **`db.snapshot()`** method exists (`db.rs:105`) — creates read-only copies
3. **`file_overrides` input** explicitly documented as "Mostly used by language server and tests" — designed for incremental updates
4. **`compile_prepared_db_program()`** takes `&dyn Database` — works with any pre-existing database, not just freshly created ones
5. **Salsa's red-green algorithm** handles invalidation automatically when inputs change

**This is the key architectural enabler.** A daemon can hold a `RootDatabase`, update `file_overrides` when files change, and recompile — Salsa automatically invalidates only the affected queries.

### 2.5 Scarb's Current Caching (What Already Works)

**Cache key** (`scarb/src/compiler/incremental/artifacts_fingerprint.rs`):
```rust
UnitArtifactsFingerprint {
    unit: u64,           // Hash of source files + deps + config
    target: u64,         // Hash of Scarb target settings
    local: Vec<LocalFingerprint>,  // Artifact paths + checksums
}
```

**What's cached:**

| Artifact | Format |
|----------|--------|
| Sierra IR | `{target}.sierra.json` |
| CASM | `{target}.casm` |
| Sierra (debug) | `{target}.sierra` |
| Lowering cache | Binary Salsa blob via `generate_crate_cache()` |
| Fingerprint | JSON in `.scarb/fingerprints/` |

**What's NOT cached (gaps):**
- No remote cache (no S3/R2/shared storage)
- No cross-project cache sharing (same OpenZeppelin version compiled N times)
- No STWO trace/proof caching
- Database rebuilt from scratch every invocation (no persistent process)
- Crate-level granularity only (change 1 file in a crate → recompile entire crate)

### 2.6 STWO Proving Pipeline (Verified)

**Pipeline:**
```text
Cairo source → Sierra → CASM → Cairo VM execution → Memory + Trace snapshot
  → Adapter (ProverInput) → STWO Prover → STARK Proof → Verifier
```

**Artifacts at each stage:**

| Stage | Artifact | Size (est.) | Deterministic? |
|-------|----------|-------------|----------------|
| Compilation | Sierra JSON + CASM | ~100KB-1MB | Yes |
| VM execution | Memory + state transitions | ~1-10MB | Yes (given same input) |
| Adapter | `ProverInput` struct | ~4MB | Yes |
| Preprocessed trace | Merkle-committed columns | ~4MB | Yes |
| STARK proof | FRI queries + commitments | ~60KB | Yes (given same salt + hash) |

**Key detail:** The adapter converts Cairo VM traces into `ProverInput`:
```rust
pub struct ProverInput {
    pub state_transitions: StateTransitions,
    pub memory: Memory,
    pub pc_count: usize,
    pub public_memory_addresses: Vec<u32>,
    pub builtin_segments: BuiltinSegments,
    pub public_segment_context: PublicSegmentContext,
}
```

Proving is **100-1000x slower than compilation**. Caching STWO artifacts has the highest time-savings ROI in the entire pipeline.

Source: `stwo-cairo/stwo_cairo_prover/crates/prover/src/prover.rs`

---

## 3. Architecture Decision: Why a Persistent Daemon

### 3.1 The Problem (Quantified)

Every `scarb build` today:
1. **Creates new RootDatabase** — loads corelib (~2-3s), initializes all query groups
2. **Loads incremental cache from disk** — deserializes lowering cache blobs
3. **Runs compilation** — even if only 1 file changed, database starts cold
4. **Discards database** — all in-memory Salsa state is thrown away

The language server (`cairo-language-server`) independently:
1. **Creates its own RootDatabase** — same corelib loading, same initialization
2. **Processes the same files** — parses, checks types, resolves symbols
3. **Holds it in memory** — but doesn't share with build

Result: **double the memory, double the parsing, zero shared work.**

### 3.2 The Solution: Shared Salsa Database Daemon

```text
                        ┌─────────────────────┐
                        │     KHEPRI DAEMON    │
                        │                      │
                        │  ┌────────────────┐  │
                        │  │ RootDatabase   │  │
 scarb build ──gRPC────▶│  │ (Salsa 0.26)   │◀─── file watcher (notify crate)
                        │  │                │  │
 cairo-ls ────gRPC─────▶│  │ 100+ cached    │  │
                        │  │ query results  │  │
 scarb prove ──gRPC───▶│  └────────────────┘  │
                        │                      │
                        │  ┌────────────────┐  │
                        │  │ Artifact Cache  │  │
                        │  │ (local + remote)│  │
                        │  └────────────────┘  │
                        └─────────────────────┘
```

**One process, one database, multiple consumers.**

### 3.3 Why This Is Better Than Alternatives

| Alternative | Why Not |
|---|---|
| **Fork Scarb** | Scarb is 78.9% Rust, 50+ crates, active development. Forking = maintenance burden. |
| **Replace Scarb** | PubGrub, plugin system, manifest parsing all work fine. No need to rewrite. |
| **Modify Cairo compiler** | Salsa support is already there. The compiler doesn't need changes. |
| **Just add remote cache to Scarb** | Helps CI but doesn't fix the cold-database problem for local dev. |

**Khepri is a new process, not a fork.** Scarb calls into it instead of directly calling `cairo-lang-compiler`. The compiler code is unchanged. Scarb's CLI, resolver, manifest handling, and plugin system are unchanged.

---

## 4. Detailed Design

### 4.1 Daemon Process

**Language:** Rust (same as Cairo compiler — link `cairo-lang-*` crates directly as libraries)

**IPC:** Unix domain socket with gRPC (via `tonic`). Why:
- Typed API (protobuf schema)
- Streaming support (for watch mode / diagnostics)
- Battle-tested in build tools (Bazel uses gRPC for remote execution)
- Fast on localhost (Unix socket, no TCP overhead)

**Lifecycle:**
```text
khepri start     → starts daemon, creates RootDatabase, loads corelib
                   listens on ~/.khepri/sock (Unix) or localhost:17420 (TCP)
                   starts file watcher on workspace roots

khepri stop      → graceful shutdown, saves database snapshot to disk

khepri status    → reports: uptime, memory, cached queries, watched files

Auto-start:      scarb build detects no daemon → starts one automatically
Auto-stop:       after 30 min idle, daemon exits (configurable)
```

### 4.2 gRPC Service Definition

```protobuf
service Khepri {
  // Compilation
  rpc Compile(CompileRequest) returns (CompileResponse);
  rpc CompileStreaming(CompileRequest) returns (stream CompileEvent);

  // Language server queries
  rpc SemanticQuery(SemanticQueryRequest) returns (SemanticQueryResponse);
  rpc Diagnostics(DiagnosticsRequest) returns (stream Diagnostic);

  // Cache management
  rpc CacheStats(Empty) returns (CacheStatsResponse);
  rpc CacheInvalidate(CacheInvalidateRequest) returns (Empty);
  rpc CachePush(CachePushRequest) returns (Empty);  // upload to remote

  // File system
  rpc FileChanged(FileChangedRequest) returns (Empty);  // manual notification
  rpc WatchStatus(Empty) returns (WatchStatusResponse);

  // STWO pipeline
  rpc Execute(ExecuteRequest) returns (ExecuteResponse);
  rpc Prove(ProveRequest) returns (ProveResponse);

  // Lifecycle
  rpc Shutdown(Empty) returns (Empty);
  rpc Status(Empty) returns (StatusResponse);
}
```

### 4.3 Database Management

```rust
struct KhepriState {
    /// The single Salsa database — shared across all consumers
    db: RwLock<RootDatabase>,

    /// Content-addressed artifact cache
    cache: ArtifactCache,

    /// File watcher state
    watcher: FileWatcher,

    /// Connected clients (scarb instances, LSP instances)
    clients: DashMap<ClientId, ClientState>,
}

impl KhepriState {
    /// Called when a file changes (from watcher or explicit notification)
    fn on_file_changed(&self, path: &Path, content: Arc<str>) {
        let mut db = self.db.write();
        // Update the Salsa input — this is the ONLY mutation needed
        // Salsa automatically marks all dependent queries as stale
        let file_id = self.resolve_file_id(&db, path);
        override_file_content!(*db, file_id, Some(content));
    }

    /// Called by scarb build
    fn compile(&self, crate_ids: Vec<CrateId>, config: CompilerConfig) -> Result<ProgramArtifact> {
        let db = self.db.read();

        // 1. Check content-addressed cache
        let cache_key = self.compute_cache_key(&db, &crate_ids, &config);
        if let Some(artifact) = self.cache.get(&cache_key) {
            return Ok(artifact);
        }

        // 2. Compile using warm Salsa database
        //    Only stale queries are recomputed (red-green algorithm)
        let artifact = compile_prepared_db_program_artifact(
            &*db, crate_ids, config
        )?;

        // 3. Store in cache (local + async push to remote)
        self.cache.put(cache_key, &artifact);

        Ok(artifact)
    }
}
```

### 4.4 Content-Addressed Cache

**Cache key construction:**

```rust
struct CacheKey {
    /// Hash of: source content + dependency hashes (recursive)
    source_hash: [u8; 32],
    /// Exact Cairo compiler version (from cairo-lang-compiler crate version)
    compiler_version: String,
    /// Compilation flags (gas enabled, optimization level, etc.)
    flags_hash: [u8; 32],
    /// Target kind (lib, starknet-contract, executable, test)
    target_kind: String,
}

impl CacheKey {
    fn digest(&self) -> [u8; 32] {
        blake3::hash(&self.to_bytes())
    }
}
```

**Three-tier lookup:**

```text
1. In-memory (Salsa query cache)     → ~0ms    [Salsa handles this automatically]
2. Local disk cache                   → ~5-50ms [content-addressed, ~/.khepri/cache/]
3. Remote cache (S3/R2/GCS)          → ~50-500ms [shared across team/CI]
```

**Cache storage format:**

```text
~/.khepri/cache/
  objects/
    ab/cdef1234...  → Sierra JSON (zstd compressed)
    cd/ef5678...    → CASM bytecode (zstd compressed)
  stwo/
    traces/
      ab/cdef...    → Execution trace (zstd compressed)
    proofs/
      cd/ef56...    → STARK proof
```

### 4.5 File Watching

Use `notify` crate (cross-platform file system events):

```rust
fn start_watcher(state: Arc<KhepriState>, roots: Vec<PathBuf>) {
    let (tx, rx) = channel();
    let mut watcher = RecommendedWatcher::new(tx, Config::default())?;

    for root in &roots {
        watcher.watch(root, RecursiveMode::Recursive)?;
    }

    // Process events, debounced
    for event in rx {
        match event.kind {
            EventKind::Modify(_) | EventKind::Create(_) => {
                if event.path.extension() == Some("cairo") {
                    let content = fs::read_to_string(&event.path)?;
                    state.on_file_changed(&event.path, content.into());
                }
            }
            _ => {}
        }
    }
}
```

### 4.6 Language Server Integration

Two integration paths (Phase 3, choose one):

**Option A: Proxy mode (simpler)**
```text
Editor → cairo-language-server → Khepri daemon (gRPC) → Salsa Database
```
The existing `cairo-language-server` is modified to delegate queries to Khepri instead of maintaining its own database. Minimal changes to the LSP codebase.

**Option B: Native mode (better performance)**
```text
Editor → Khepri daemon (LSP protocol directly) → Salsa Database
```
Khepri implements the LSP protocol itself, serving both build and IDE queries from the same database. More work but eliminates one process and all serialization overhead.

**Recommendation:** Start with Option A (proxy). It's lower risk and proves the shared-database concept. Migrate to Option B later if the gRPC overhead is measurable.

### 4.7 STWO Pipeline Caching

**Cache key for execution traces:**
```rust
struct ExecutionCacheKey {
    casm_hash: [u8; 32],        // Hash of compiled CASM
    input_hash: [u8; 32],       // Hash of program inputs
    vm_version: String,         // Cairo VM version
}
```

**Cache key for proofs:**
```rust
struct ProofCacheKey {
    trace_hash: [u8; 32],       // Hash of execution trace
    prover_config_hash: [u8; 32], // PCS config, security params
    salt: u32,                  // Channel salt
}
```

Since proving is 100-1000x slower than compilation, even a remote cache lookup at 500ms is a massive win vs re-proving at 30-300s.

---

## 5. Integration With Scarb

### 5.1 Zero-Migration Path

Khepri integrates as a **Scarb plugin/wrapper**, not a standalone rewrite:

```toml
# Scarb.toml — opt-in to Khepri acceleration
[tool.khepri]
enabled = true
remote-cache = "s3://my-team/khepri-cache"  # optional
daemon-timeout = "30m"                       # auto-stop after idle
```

**Modified compilation flow:**

```text
scarb build
  → ops::compile()
  → [NEW] check if Khepri daemon is running
    → YES: send CompileRequest via gRPC, receive ProgramArtifact
    → NO: start daemon, then send request
    → FALLBACK: if daemon fails, fall back to current direct compilation
```

### 5.2 What Changes in Scarb

**Minimal changes — ~200 lines:**

Replace in `scarb/src/compiler/db.rs`:
```rust
// BEFORE: create new database every time
fn build_scarb_root_database(...) -> ScarbDatabase {
    let db = RootDatabase::builder()...build();
    ScarbDatabase { db, proc_macros }
}

// AFTER: connect to daemon or fall back
fn build_scarb_root_database(...) -> ScarbDatabase {
    if let Ok(client) = khepri_client::connect() {
        return ScarbDatabase::Remote(client);
    }
    // Fallback: original behavior
    let db = RootDatabase::builder()...build();
    ScarbDatabase::Local { db, proc_macros }
}
```

Replace in `scarb/src/compiler/compilers/lib.rs`:
```rust
// BEFORE: direct compilation
let artifact = compile_prepared_db_program_artifact(db, crate_ids, config)?;

// AFTER: via daemon or direct
match &scarb_db {
    ScarbDatabase::Remote(client) => {
        client.compile(crate_ids, config).await?
    }
    ScarbDatabase::Local { db, .. } => {
        compile_prepared_db_program_artifact(db, crate_ids, config)?
    }
}
```

### 5.3 What Does NOT Change in Scarb

- CLI interface (`scarb build`, `scarb test`, etc.)
- Manifest format (`Scarb.toml`)
- PubGrub dependency resolution
- Plugin system (snforge, sncast)
- Project structure and workspace handling
- Fingerprint-based crate caching (still works as a fallback layer)

---

## 6. Reproducible Builds

Critical for blockchain: two machines must produce identical CASM from identical source.

### 6.1 Sources of Non-Determinism to Eliminate

| Source | Risk | Mitigation |
|--------|------|------------|
| HashMap iteration order | Medium | Cairo compiler uses `OrderedHashMap` throughout — already handled |
| Parallel output ordering | Low | Scarb compiles units on separate threads but collects results deterministically |
| Timestamps in artifacts | Medium | Strip from Sierra JSON / CASM output |
| Compiler version drift | High | Cache key includes exact compiler crate version + git rev |
| Salsa query execution order | None | Salsa queries are pure functions — order doesn't affect output |
| Platform differences (f64, etc.) | Low | Cairo field arithmetic is integer-only (`Felt252`) |

### 6.2 Verification

```bash
# Verify reproducibility
khepri build --verify-reproducible
# Builds twice with different Salsa execution orders
# Compares output byte-for-byte
# Fails if any artifact differs
```

---

## 7. Implementation Roadmap

### Phase 1: Remote Cache Layer (3-4 weeks)

**Goal:** Ship a `khepri cache` command that adds remote caching on top of Scarb's existing local cache. Zero architectural risk.

**What to build:**
- Content-addressed cache store (local: `~/.khepri/cache/`, remote: S3-compatible)
- Cache key computation (source hash + compiler version + flags)
- `khepri cache push` / `khepri cache pull` CLI commands
- Scarb wrapper: `khepri build` = check remote cache → fall back to `scarb build` → push artifacts
- zstd compression for artifacts

**Files:**
```text
khepri/
  Cargo.toml
  src/
    main.rs              # CLI entry
    cache/
      mod.rs             # Cache trait
      local.rs           # Local disk cache (~/.khepri/cache/)
      remote.rs          # S3-compatible remote cache
      key.rs             # Cache key computation
    wrapper.rs           # Wraps scarb build with cache lookup
```

**Verify:** Run on CI, measure cache hit rates, compare build times.

### Phase 2: Persistent Daemon (6-8 weeks)

**Goal:** Long-lived process holding the Salsa `RootDatabase`. Scarb connects to it instead of rebuilding the database per invocation.

**What to build:**
- Daemon process with gRPC API (tonic)
- `RootDatabase` held in `RwLock`, initialized once with corelib
- File watcher (`notify` crate) feeding `file_overrides` into Salsa
- `khepri start` / `khepri stop` / `khepri status` commands
- Auto-start from `scarb build` if daemon not running
- Graceful fallback if daemon crashes

**Files (additions):**
```text
khepri/src/
    daemon/
      mod.rs             # Daemon lifecycle (start, stop, status)
      server.rs          # gRPC server implementation
      state.rs           # KhepriState (RootDatabase + cache + watcher)
      watcher.rs         # File system watcher
    proto/
      khepri.proto       # gRPC service definition
    client.rs            # Client library for Scarb integration
```

**Verify:** Benchmark warm rebuild after single-file edit. Target: <3s.

### Phase 3: Language Server Unification (4-6 weeks)

**Goal:** `cairo-language-server` connects to Khepri daemon instead of maintaining its own database.

**What to build:**
- gRPC endpoints for LSP-relevant queries (diagnostics, hover, go-to-def, completions)
- Adapter layer in `cairo-language-server` to delegate to Khepri
- Shared `RootDatabase` serves both build and IDE queries

**Verify:** Open project in VS Code, edit file, run `scarb build` — the build should be near-instant because the daemon already processed the edit via LSP.

### Phase 4: Parallel Compilation + STWO Caching (4-6 weeks)

**Goal:** Compile independent crates in parallel. Cache STWO execution traces and proofs.

**What to build:**
- DAG analysis of workspace crate dependencies
- Parallel compilation using `RootDatabase::snapshot()` for read-only access across threads
- STWO trace caching (execution trace → content-addressed store)
- STWO proof caching (proof → content-addressed store)
- `khepri prove` with cache lookup

**Verify:** Multi-crate workspace compiles in parallel. STWO proof cache hit skips proving entirely.

### Phase 5: Sub-Crate Granularity (future, 8+ weeks)

**Goal:** Push invalidation below crate level by exposing more Salsa queries.

**What to build:**
- Module-level dependency tracking within a crate
- Per-module cache keys
- Expose Salsa's internal query graph for debugging (`khepri query-graph`)

**Prerequisite:** Requires deeper integration with `cairo-lang-lowering`'s internal queries. May need upstream changes to expose `generate_crate_cache()` at module granularity.

---

## 8. Why This Approach Is Best

### 8.1 Compared to "Rewrite Scarb from Scratch"

| | Rewrite | Khepri daemon |
|---|---|---|
| Effort | 6-12 months | 3-4 months to useful MVP |
| Risk | High (rewriting resolver, plugins, manifest) | Low (Scarb unchanged, daemon is additive) |
| Migration | All users must switch | Opt-in, graceful fallback |
| Maintenance | Fork diverges from upstream Scarb | Independent process, version-independent |

### 8.2 Compared to "Just Contribute to Scarb"

| | Contribute upstream | Khepri daemon |
|---|---|---|
| Speed to ship | Slow (PR review, design alignment) | Fast (independent project) |
| Scope | Limited by Scarb's architecture | Unconstrained (new process) |
| Daemon model | Requires Scarb architecture rethink | Natural fit (new process) |
| Remote cache | Would need Scarb team buy-in | Ships independently |

### 8.3 Compared to "Modify the Cairo Compiler"

| | Modify compiler | Khepri daemon |
|---|---|---|
| Risk | Very high (compiler internals) | Low (consumer of compiler API) |
| Salsa granularity | Could expose module-level queries | Works with existing crate-level API |
| Upstream acceptance | Uncertain | Not needed |

### 8.4 The Bazel/Gradle Precedent

- **Gradle** added a daemon in 2011 — build times dropped 3-10x for warm builds
- **Bazel** uses a persistent server process — holds build graph in memory
- **rust-analyzer** is a persistent compiler daemon — shares Salsa DB with IDE
- **Turborepo** added remote caching — CI times dropped 40-80%

Khepri follows the same proven pattern: **persistent process + shared state + content-addressed cache**.

---

## 9. Technical Risks and Mitigations

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Salsa Database too large for memory | Low | High | Monitor RSS, implement LRU eviction for cold crate queries |
| gRPC serialization overhead | Low | Medium | Use Unix domain sockets (no TCP), flatbuffers for large artifacts |
| Daemon crashes corrupt cache | Medium | Low | Cache is content-addressed — corruption detected on hash mismatch, graceful fallback to direct compilation |
| Cairo compiler API changes break daemon | Medium | Medium | Pin to specific cairo-lang-* crate versions, version-check on connection |
| File watcher misses changes | Low | Medium | Scarb sends explicit `FileChanged` notification as belt-and-suspenders |
| Proc macro plugins need special handling | Medium | Medium | Proc macros compile to native .dylib — daemon needs to load them same as Scarb does (via `libloading`) |

---

## 10. Open Questions for Design Review

1. **Plugin loading:** Scarb loads proc macro plugins as `.dylib` via `libloading`. Should Khepri daemon load these directly, or should Scarb handle plugin compilation and pass the `PluginSuite` to the daemon?

2. **Database snapshots for parallel compilation:** Salsa 0.26 supports `db.snapshot()` for read-only clones. Is the clone cost acceptable for per-crate parallelism, or should we use a single-writer-multiple-reader model?

3. **LSP integration path:** Proxy mode (modify cairo-language-server to delegate to Khepri) vs native mode (Khepri implements LSP directly). Proxy is safer but adds a hop. Native is cleaner but more work.

4. **Remote cache protocol:** Use Bazel's Remote Execution API (industry standard, tooling exists) vs simpler custom S3-based protocol (less overhead, easier to implement)?

5. **Scarb integration depth:** Ship as external wrapper (`khepri build` calls `scarb build` internally) vs Scarb plugin (deeper integration, less friction) vs upstream PR (highest friction, best long-term)?

---

## Appendix A: Repository Structure Reference

```text
<repo-root>/
├── scarb/                          # Build tool (Software Mansion)
│   ├── scarb/src/
│   │   ├── compiler/
│   │   │   ├── db.rs               # ScarbDatabase / RootDatabase construction
│   │   │   ├── mod.rs              # Compiler trait
│   │   │   ├── compilers/          # LibCompiler, StarknetContractCompiler, etc.
│   │   │   ├── incremental/        # Fingerprint caching
│   │   │   └── plugin/             # Cairo plugin loading (proc macros)
│   │   ├── ops/
│   │   │   └── compile.rs          # Main compilation orchestration
│   │   ├── resolver/               # PubGrub dependency resolution
│   │   └── core/                   # Workspace, Package, Config types
│   └── Cargo.toml                  # Pins cairo-lang-* @ rev 661df684
│
├── cairo/                          # Cairo compiler (StarkWare)
│   └── crates/
│       ├── cairo-lang-compiler/
│       │   ├── src/db.rs           # RootDatabase (Salsa 0.26)
│       │   └── src/lib.rs          # compile_prepared_db_program()
│       ├── cairo-lang-filesystem/
│       │   └── src/db.rs           # FilesGroup (INPUT: files, crates)
│       ├── cairo-lang-parser/
│       │   └── src/db.rs           # ParserGroup (TRACKED: AST)
│       ├── cairo-lang-defs/
│       │   └── src/db.rs           # DefsGroup (TRACKED: symbols)
│       ├── cairo-lang-semantic/
│       │   └── src/db.rs           # SemanticGroup (TRACKED: types)
│       ├── cairo-lang-lowering/
│       │   └── src/db.rs           # LoweringGroup (TRACKED: IR)
│       └── cairo-lang-sierra-generator/
│           └── src/db.rs           # SierraGenGroup (TRACKED: codegen)
│
├── stwo-cairo/                     # STWO prover integration
│   └── stwo_cairo_prover/crates/
│       ├── prover/src/prover.rs    # STARK proof generation
│       └── adapter/src/lib.rs      # VM trace → ProverInput
│
├── salsa/                          # Incremental computation framework
│   └── src/
│       ├── database.rs             # Database trait (Send, snapshot)
│       └── storage.rs              # Internal storage management
│
└── cairo-compiler-workshop/        # SWM workshop on compiler internals
    └── README.md                   # 7-stage pipeline walkthrough
```

## Appendix B: Key Functions at the Boundary

```rust
// Scarb creates the database:
// scarb/src/compiler/db.rs:37
pub fn build_scarb_root_database(unit: &CairoCompilationUnit, ws: &Workspace) -> Result<ScarbDatabase>

// Scarb calls the compiler:
// scarb/src/compiler/compilers/lib.rs:76
cairo_lang_compiler::compile_prepared_db_program_artifact(db, main_crate_ids, compiler_config)

// Compiler's entry point that accepts existing database:
// cairo/crates/cairo-lang-compiler/src/lib.rs:73
pub fn compile_prepared_db_program(db: &dyn Database, main_crate_ids: Vec<CrateId>, config: CompilerConfig) -> Result<Program>

// The Salsa input that enables incremental updates:
// cairo/crates/cairo-lang-filesystem/src/db.rs:235
files_group_input(db).set_file_overrides(db).to(Some(overrides));

// Database snapshot for parallel reads:
// cairo/crates/cairo-lang-compiler/src/db.rs:105
pub fn snapshot(&self) -> RootDatabase
```

# uc Next-Generation Review — 2026-07-08

Session artifact: full repo review + empirical testing + July-2026 SOTA research.
Host: Apple Silicon macOS, scarb 2.14.0, uc @ main `e0f3c22` (release build), corelib 2.16 via sparse clone override.

---

## 1. Verified bugs (empirically reproduced unless noted)

### BUG-1 (P0, correctness): stale cache hit after editing a path dependency — BOTH lanes
Repro: workspace `app` with `dep_pkg = { path = "../dep_pkg" }` outside the workspace root.
1. `uc build --engine uc --daemon-mode off` (cold, compiles, caches)
2. edit `../dep_pkg/src/lib.cairo` (42 → 43)
3. `uc build` again → `"cache_hit": true` in 0.3 ms, restored artifact still contains **42**; `scarb build` on the same tree correctly rebuilds to 43.

Reproduced on `scarb_fallback` AND `uc_native` backends: the top-level gate
`run_build_with_uc_cache` (main.rs:10921) validates only
`compute_build_fingerprint_with_scarb_version` (fingerprint.rs:510), whose WalkDir
starts at `workspace_root` only. Path-dep sources are never fingerprinted, and a
cache hit short-circuits before the native session's drift scan (which DOES track
`path_dependency_roots`, main.rs:12779) ever runs.

Fix direction: extend the fingerprint walk to the resolved path-dependency roots
(the project model already knows them — `uc resolve` lists `dep_pkg`), or fold each
path dep's own fingerprint into the context digest. Add a regression test mirroring
`run_build_with_uc_cache_hits_after_initial_compile` but with an external path dep.
Longer term: lockfile v2 with content hashes for path deps.

Related smaller holes in the same file, code-read only (not reproduced):
- `MAX_FINGERPRINT_DEPTH = 32` silently EXCLUDES deeper files (false hit if they change); the file-count limit bails loudly, the depth limit doesn't.
- `try_reuse_hot_fingerprint` trusts dir-mtimes + known-file stats; a tarball extracted with preserved dir mtimes that ADDS a new `.cairo` file is invisible (narrow, tar/rsync -t style flows).
- `should_include_fingerprint_file` requires ext `== "cairo"` exactly while `hash_fingerprint_source_file` accepts case-insensitive — `Foo.CAIRO` is compiled by scarb but never fingerprinted.

### BUG-2 (P1, parity): native lib target emits `app.sierra` (raw text); scarb emits `app.sierra.json`
Downstream tools (foundry, deploy scripts, universal-sierra-compiler) that expect
Scarb's layout break silently on lib targets. Either match Scarb artifact naming and
format exactly or classify lib targets as unsupported-native until parity exists
(the repo's own "fallback is not native success" standard).

### BUG-3 (P0, hygiene): fresh-clone test suite is red
`main_tests::compile_native_casm_contract_rejects_tiny_bytecode_limit` (main_tests.rs:15051)
reads `benchmarks/fixtures/scarb_smoke/target/dev/uc_smoke_token.contract_class.json` —
a gitignored build artifact. 394 pass / 1 fail on a clean clone. Commit the fixture
artifact as test data (rename so it isn't in `target/`), or build it in test setup.
This is also why local-only validation ("GitHub Actions are not the default path")
never caught it: warmed machines have the file.

### BUG-4 (P1, agent contract): `--json` build failures can emit NO JSON
`uc build --daemon-mode require --json` with no daemon running prints a plain-text
anyhow chain to stderr, nothing on stdout, exit 1. The 2026 agent-CLI consensus (and
uc's own Agent Contract) says: structured error envelope on failure — code,
retryable, next_commands — plus a typed exit-code taxonomy. UCN2001 does this
beautifully for fallback; the daemon-unreachable path bypasses it entirely.
Also: `--daemon-mode auto` silently proceeds without the daemon (`daemon_used:false`,
zero diagnostics) and `require` does not attempt auto-start. Recommend: auto-start
under a flock guard, `uc daemon ensure` verb, and a "daemon skipped" diagnostic in auto mode.

### FINDING-5 (process): the cold-path supremacy work is stranded
Issue #18 shows ALL phases 0–6 checked, but PR #19 (`feat/cold-path-supremacy-phases-0-4`,
open since 2026-03-07) is CONFLICTING against main. Only Phase 1 (async persist) and
Phase 4 (postcard) were re-landed. **Not on main: mimalloc (Phase 3), embedded corelib
(Phase 5), pre-compiled corelib Sierra blob (Phase 6 — the projected −300–500 ms
game-changer).** grep confirms: no `mimalloc`, no `include_dir`, no corelib-blob fns
in `crates/`. The issue's own projection: Phases 1–4 ≈ cold parity, Phases 5–6 ≈ 3.3×
faster than Scarb. Reland on current main is the single highest-ROI perf action.
Also fix the #18 checkboxes to reflect main, not the branch.

### FINDING-6 (docs/UX): `support native` answers project shape, not host readiness
On a lockfile pinned to 2.14, `support native` returns `status:"supported"` citing the
2.14 external-helper `binary_path` even when that helper was never built; the
subsequent build silently downgrades to scarb (with a correct UCN2001, to its credit).
Agents following inspect→support→build without `toolchain ensure` get a
"supported"→fallback contradiction. Add `host_ready: bool` (or `helper_built`) to the
support report, or fold a toolchain-presence check into `supported`.

### FINDING-7: `uc mcp serve` is a static catalog, not an MCP server
It prints a JSON catalog and exits (main.rs:7113). Fine as a stub; see §4 for what a
real one should be.

---

## 2. Measured numbers (this host, 2026-07-08)

Toy lib (app + path dep), uc_native lane, daemon off unless noted:

| Scenario | uc | scarb 2.14 |
|---|---:|---:|
| cold | 190 ms | ~900 ms (smoke) |
| warm noop | 0.36 ms (cache hit) | 20 ms |
| comment-only edit | 21 ms (semantic-hash hit — uc-unique win) | 520 ms (recompiles) |
| real edit, no daemon | ~185 ms | ~520 ms |
| real edit, daemon hot | 194 → 132 → **91 ms** (`native_changed_files:1`) | ~520 ms |

Smoke fixture (starknet-contract, scarb_fallback lane): uc cold 832 ms
(797 ms is scarb itself; uc overhead ~35 ms), noop 24 ms.

Real-world pain datapoint: `atomic_lock` (garaga git dep + OZ) cold build via scarb
exceeded **7 minutes** (killed; git-dep fetch through a slow network + garaga
frontend compile). This is the corpus-level cold problem that §3.3 and §3.4 attack.

Their own strict evidence (docs/research, deleted from main by PR #72, recoverable at
branch `codex/perf-native-frontier-20260429`): supported-set cold speedups
1.30×/2.34×/1.78×/3.77×, warm-noop up to 168×, `zcash_relay` warm-noop **0.61× (regression)**,
cold dominated by `native_frontend_compile_ms` (7.0 s of 7.4 s on monero). Consider
restoring these docs — they're the evidence base for every perf claim.

---

## 3. How to accelerate uc deeply (ordered)

### 3.1 Reland PR #19 on main (days, big cold win)
mimalloc → binary-serde already done → embedded corelib → corelib Sierra/lowering
blob. Mechanical conflicts, all design work done, projections in #18.

### 3.2 Kill the frontend wall — the real cold fight (weeks)
Their frontier trace shows repeated `estimate_size` / `dummy_program_for_size_estimation`
churn concentrated in shared corelib helpers during inlining. Three levers:
1. **Memoize/dedupe size estimation upstream** (starkware-libs/cairo): the trace shows the same helpers re-estimated repeatedly. An upstream PR benefits every Scarb user too — good SNF citizenship, and uc keeps the daemon advantage.
2. **Pre-compiled dependency blobs** — extend the Phase-6 corelib-blob mechanism (Scarb's `BlobLongId::Virtual` cache injection) to immutable deps (registry versions, git revs): compile garaga v1.0.1 once per (dep, compiler, profile) key, store in the CAS, inject on cold. Registry/git deps are immutable — perfect cache keys. Nobody in the Cairo ecosystem serves precompiled dep caches yet; this is uv's "warm cache = 100×" moment applied to compilation, and it directly kills the 7-minute garaga cold build.
3. **Parallel compilation units**: Scarb compiles units serially; contracts within a project are independent after the shared frontend DB. The daemon can fan units across cores (Salsa DBs per worker, TS7-style checker parallelism).

### 3.3 Remote/shared action cache (weeks, transforms CI + agent fleets)
`.uc/cache/objects` is already content-addressed with an entry/object split (AC/CAS
shape). Add a read-through remote tier (S3/R2/HTTP; bazel-remote-compatible optional),
deferred materialization (restore only requested targets), and the existing
`returning_cold` scenario as the CI benchmark. Issues #8/#9 already sketch this.
Agents in CI are the population that hits cold constantly — this is where
"cold supremacy" actually gets experienced.

### 3.4 Own the fetch path (imports/downloads)
Today `uc fetch` shells to `scarb fetch`. Next-gen surface, borrowing the winners:
- **uv/pnpm**: global content-addressable source store (`~/.uc/store`) + hardlink/clonefile materialization into workspaces; one copy per (pkg, version) on disk, O(links) per project.
- **uv**: metadata-only resolution (fetch index metadata, not archives, until commit).
- **Bun**: syscall discipline, clonefile on macOS, binary metadata caches; lifecycle scripts disabled by default (supply-chain posture).
- **Parallel range downloads + zstd** for registry archives; sparse index protocol as scarbs.xyz grows (79 pkgs / 2.7 M downloads today — small enough that uc could co-design the protocol with Software Mansion).
- **Integrity/provenance**: checksum pinning in lock, sigstore-style attestation later — matches the AGENTS.md package-manager checklist verbatim.

### 3.5 Daemon as the product (days–weeks)
Auto-start + `daemon ensure`; session prewarm on `project inspect` (agents always
inspect before building — by the time they call build, the Salsa DB is hot);
file-watch journal already exists. The 91 ms hot-edit number is the moat vs
Scarb 2.19 incremental — a one-shot CLI cannot keep a hot DB. Bazel persistent
workers and the TS7 daemon story are the precedents.

---

## 4. July-2026 SOTA scan → what uc should adopt

| Ecosystem | Technique | uc adoption |
|---|---|---|
| uv (Python) | global CAS + hardlink installs; PubGrub; metadata-only fetch; no-interpreter startup | §3.4; PubGrub if/when uc owns resolution (pubgrub-rs is Cargo's chosen next resolver) |
| Bun (JS) | syscall minimization; clonefile; SoA data layouts; binary registry cache; no lifecycle scripts | §3.4; security default worth copying outright |
| pnpm | content-addressable store + links | §3.4 |
| Cargo/rustc | sparse registry index; fingerprint dep-info; `-Zthreads` parallel frontend; sccache | sparse index for scarbs.xyz; parallel units §3.2 |
| Buck2/Bazel | AC/CAS remote cache; persistent workers; deferred materialization; DICE single-graph, no phases | §3.3, §3.5 |
| TypeScript 7 (Go port, RC June 2026) | native + shared-memory parallel checkers = 10× | parallel units; also proof that "rewrite the hot loop native+parallel" is the era's playbook |
| Zig 0.16 | incremental in-place binary patching, 30 ms rebuilds; DAG overhaul | north star for warm edits; uc's session+journal is the same philosophy at coarser grain |
| Go | content-addressed build cache keyed by action inputs (default-on since 1.10) | validates default-on cache; uc already there locally |
| Scarb 2.19 (the competitor) | incremental compilation stable + default-on; parallel `scarb check`; cached warnings; biweekly releases | **re-baseline all benchmarks vs 2.19, not 2.14**; differentiation must come from daemon/session, agent contracts, remote cache, dep blobs — not from "we have incrementality and they don't" (no longer true) |

### Agent-first CLI consensus (Arcjet, Google Workspace CLI writeups, June 2026)
uc is genuinely ahead: JSON-first reports, `what_happened/why/retryable/next_commands`,
replay bundles, plan-only, schema versioning — this matches or exceeds the published
state of the art. Gaps to close:
1. **Typed exit-code taxonomy** (0 ok / 2 validation / 3 toolchain / 4 daemon / 5 unsupported / 6 confirmation-required) — agents branch on `$?` before parsing.
2. **Structured errors ALWAYS in `--json` mode** (BUG-4).
3. **Runtime schema introspection**: `uc schema build --json` returning the JSON Schema (files exist in docs/agent/schemas — expose them from the binary).
4. **Bounded output / `--fields` masks** — context-window discipline for agents.
5. **NDJSON event stream**: `uc watch --json` emitting build results per save; agents subscribe instead of polling (daemon already watches).
6. **Real MCP server** (stdio JSON-RPC over the daemon) alongside the CLI: catalog exists; the 2026 debate says CLI-first is right for token cost, but MCP gets uc into Claude Code/Codex tool registries with zero prompt-engineering. Precedents: Bazel MCP servers, Microsoft's MSBuild binlog MCP with CI agents auto-diagnosing failed builds — uc's failure bundles are BETTER input for that loop than binlogs.
7. **Idempotent verbs** (`toolchain ensure` already is; extend the pattern — `daemon ensure`, `fetch --ensure`).

### Starknet ecosystem context (July 2026)
- Starknet runs an official verifiable-AI-agents push (portal, marketplace, grants); OpenZeppelin ships Contracts MCP; Cairo Coder (Kasar/Ask Starknet monorepo) does RAG codegen; snforge added experimental cairo-native execution, a debugger, test partitions. Agents writing/deploying Cairo is the sanctioned growth path — uc's thesis is aligned with where the foundation is already pointing.
- The slowest agent loop on Starknet today is **edit → build → snforge test**. snforge shells to scarb for builds; if uc becomes snforge's build backend (or ships `uc test` wrapping snforge with the daemon session), the whole ecosystem's agent iteration speed improves. That's the highest-leverage integration, above raw build speed.
- Prove-path integration (issue #8) becomes strategic with S-two on mainnet: `uc build --plan` → `uc prove --plan` with the same session/cache discipline would make uc the control plane for provable apps — no equivalent exists.

---

## 5. Recommended order of work

| # | Item | Size | Payoff |
|---|---|---|---|
| 1 | BUG-1 path-dep fingerprint + regression test | S | correctness (agents silently ship stale artifacts today) |
| 2 | BUG-3 fresh-clone test; BUG-4 JSON error envelope + exit taxonomy; FINDING-6 host_ready | S | trust + agent contract |
| 3 | Reland PR #19 (mimalloc, embedded corelib, Sierra blob) on main | M | cold parity → ~3× vs scarb 2.14 baseline |
| 4 | Re-baseline strict benchmarks vs Scarb 2.19; restore deleted research docs | S | honest competitive position |
| 5 | Daemon ensure/auto-start + prewarm-on-inspect | S–M | 91 ms hot edits become the default experience |
| 6 | Precompiled dependency blobs (garaga-class deps) | M–L | kills the 7-minute cold; unique in ecosystem |
| 7 | Remote AC/CAS read-through cache | M | CI + agent fleets |
| 8 | First-party fetch (CAS store + links + parallel downloads) | M–L | "uv for Cairo" |
| 9 | Agent surfaces: schema introspection, NDJSON watch, real MCP server | M | distribution into agent harnesses |
| 10 | snforge build-backend integration; prove-path plan | L | ecosystem-level win, SNF-strategic |
| 11 | Upstream estimate_size memoization + parallel units to cairo | M | everyone wins, uc keeps daemon moat |

## 5b. Addendum — measured after the fix session (same day)

Helper lane 2.14 built on this host (staged via `build_native_toolchain_helper.sh`;
note: the script needs python ≥3.11 on PATH and pre-extracted registry sources for
the two patched crates — both were missing on a fresh machine, worth a doctor check).

Smoke **contract** fixture (4 contracts), `uc_native_external_helper` lane:

| Scenario | uc | scarb 2.14 |
|---|---:|---:|
| cold | 516 ms (frontend 434) | ~900 ms |
| warm noop | 22.8 ms | 20 ms |
| dead-code semantic edit | **28.8 ms** (impacted-set: 0 contracts recompiled) | ~520 ms |

Corelib/frontend-blob bound (2.16 builtin lane, lib project): virgin cold 237 ms →
returning cold (session image kept) 182 ms. The session image recovers only
`session_prepare` (25.6 → 3.2 ms); `native_frontend_compile_ms` stays ~180–211 ms.
**The persisted session does not capture compiled frontend state — ~97% of a
returning-cold build is re-derivable Salsa work.** That is the measured headroom for
Phase-6-style blobs (corelib first, then immutable deps).

Fix session outcome: PR #73 (stale path-dep cache fix + fresh-clone test fix,
401/401 green, Qodo findings addressed), issues #74 (lib artifact parity),
#75 (JSON error contract + daemon ensure), #76 (fingerprint hardening follow-ups),
status comment on #18 (stranded PR #19).

## 6. Sources (research)
- Scarb releases / incremental: github.com/software-mansion/scarb/releases; docs.swmansion.com/scarb/docs/reference/manifest.html
- Cairo compiler: github.com/starkware-libs/cairo/releases; software-mansion-labs/cairo-compiler-workshop
- uv: nesbitt.io/2025/12/26/how-uv-got-so-fast.html; noos.blog/posts/uv-how-it-works-under-the-hood
- Bun: bun.com/blog/behind-the-scenes-of-bun-install; betterstack.com/community/guides/scaling-nodejs/bun-install-performance
- Buck2/Bazel/Bonanza: buck2.build/docs/about/why; buildbuddy.io/blog/buck2-review; blog.engflow.com/2024/05/13/the-many-caches-of-bazel; blogsystem5.substack.com/p/bazel-next-generation
- TypeScript 7 Go port: devblogs.microsoft.com/typescript/typescript-native-port; visualstudiomagazine.com (7.0 beta/RC coverage)
- Zig incremental: ziglang.org/devlog/2026; ziglang/zig#21165
- Agent-CLI design: blog.arcjet.com/designing-a-cli-for-ai-agents; theundercurrent.dev/p/rewrite-your-cli-for-agents-or-get; gibil.dev/blog/cli-json-pattern
- Build-tool MCP: devblogs.microsoft.com/dotnet/mcp-build-diagnostics-workflows; nacgarg/bazel-mcp-server
- Starknet: starknet.io/verifiable-ai-agents; scarbs.xyz; foundry-rs/starknet-foundry releases; KasarLabs/cairo-coder-mcp

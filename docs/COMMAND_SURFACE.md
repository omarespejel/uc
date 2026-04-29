# Command Surface

## Implemented Commands

1. `uc build`
- Executes build path with selectable engines.
- Supports: `--manifest-path`, `--package`, `--workspace`, `--features`, `--offline`, `--release`, `--profile`, `--daemon-mode`, `--json`.
- Engines:
  - `--engine uc` (default): deterministic fingerprint + native compile/cache path with Scarb fallback only when allowed.
    - daemon policy via `--daemon-mode off|auto|require` (default: `auto`).
  - `--engine scarb`: direct Scarb execution path.
- `--json` emits the same execution report JSON on stdout and suppresses normal build log replay so the payload stays machine-readable.
- Optional `--report-path` writes the execution report JSON to disk; it can be combined with `--json`.
- Optional `--record-failure <path>` writes a redacted, replay-safe failure bundle when the build exits with an error. Sensitive argv values are redacted and `--record-failure` itself is stripped from the recorded replay command.
- Build report JSON now includes:
  - `compile_backend`: `scarb`, `uc_scarb`, `scarb_fallback`, `uc_native`, or `uc_native_external_helper`
  - `native_toolchain`: requested lane, selected source, resolved compiler version, and helper binary path when applicable
  - `diagnostics`: stable machine-readable diagnostic entries with `code`, `category`, `what_happened`, `why`, `how_to_fix`, `retryable`, `fallback_used`, `toolchain_expected`, and `toolchain_found`

2. `uc metadata`
- Executes metadata resolution path.
- Supports: `--manifest-path`, `--format-version`, `--offline`, `--global-cache-dir`.
- Optional `--report-path` writes execution report JSON.
- Behavior note (2026-03-06): in daemon `auto|require` modes, captured metadata `stdout/stderr` is replayed to terminal by default (even without `--report-path`); local fallback keeps streaming behavior unless report capture is requested.

3. `uc compare-build`
- Runs direct Scarb build vs `uc build` wrapper on same manifest.
- Compares artifact hashes and diagnostics lines.
- Writes JSON report to `--output-path <file>` (or `benchmarks/results/compare-build-<epoch>.json` by default) and enforces pass/fail gate.

4. `uc benchmark`
- Runs benchmark matrix harness script.

5. `uc session-key`
- Generates deterministic session key from normalized input fields.

6. `uc project inspect`
- Reads a `Scarb.toml` and optional sibling `Scarb.lock` without mutating files.
- Supports: `--manifest-path`, `--format json`, `--json`, `--report-path`.
- Emits package entries, workspace summary, declared profiles, target summary, dependency/source origins, lockfile state, conservative offline-readiness status, requested toolchain, read-only native support when determinable, and stable diagnostics in one JSON report.
- The report includes `readonly=true` and `mutation_status=none`; use this as the agent pre-build project-state surface.
- The raw report is local evidence and can include absolute paths plus manifest/lockfile hashes; redact or avoid forwarding it when sharing outside the host.

7. `uc support native`
- Probes whether a manifest is eligible for native compile in the current `uc` binary.
- Supports: `--manifest-path`, `--format text|json`, `--json`.
- Returns a structured pre-build decision report so scripts and local benchmark harnesses can classify cases before measuring them.
- Native support JSON includes selected toolchain lane and stable diagnostics for:
  - exact `cairo-version` mismatches
  - unsupported manifest constraints
  - missing or invalid external helper lanes
  - unparseable compiler versions
  - manifest-path, manifest-read, manifest-parse, and helper-probe blocked states
- `status` remains the low-level probe status (`supported`, `unsupported`, `unavailable`) for compatibility, while `decision_status` is the agent-facing state:
  - `native_supported`
  - `native_unsupported`
  - `fallback_likely`
  - `build_blocked`

1. `uc resolve`
- Reads `Scarb.toml` and `Scarb.lock` in a lockfile-first, read-only mode.
- Supports: `--locked`, `--manifest-path`, `--format json`, `--json`, `--report-path`.
- Emits a stable resolution report with:
  - top-level `status` (`ready` or `build_blocked`)
  - top-level `blocked_reason` (`null` when ready; otherwise the authoritative blocked cause)
  - dependency summaries
  - source origins
  - lockfile state
  - lockfile-sync status (`in_sync`, `lockfile_missing`, `lockfile_invalid`, `manifest_drift`, `manifest_invalid`)
  - conservative offline-readiness
  - requested toolchain summary
  - `network_intent=forbidden` and `mutation_status=none`
- `ResolveReport.blocked_reason.is_some()` implies `ResolveReport.status == "build_blocked"`.
- Agents should treat `blocked_reason` as the authoritative reason for stopping before fetch or build.
- `uc resolve` currently requires `--locked`; there is no implicit networked resolution mode yet.

1. `uc fetch`
- Materializes the locked dependency graph into `uc`'s shared source store.
- Supports: `--locked`, `--manifest-path`, `--offline`, `--format json`, `--json`, `--report-path`.
- Behavior:
  - In online mode, `uc` explicitly runs `scarb fetch` first, then replays `scarb metadata --offline` and imports the concrete package roots into the shared `uc` source store.
  - In offline mode, `uc` skips the network fetch step and only imports sources that are already locally available.
- Emits a stable fetch report with:
  - top-level `status` (`ready` or `build_blocked`)
  - top-level `blocked_reason` (`null` when fully hydrated; otherwise the authoritative blocked cause)
  - `network_intent` (`allowed` online, `forbidden` offline)
  - `execution_driver` (`scarb_direct` in the current implementation)
  - `source_store` summary
  - `fetched_entries` and `missing_entries`
  - `offline_readiness_before` and `offline_readiness_after`
- `uc fetch` currently requires `--locked`; there is no implicit mutable resolution mode yet.

1. `uc migrate`
- Analyzes `Scarb.toml` and emits a migration readiness report.
- Optional `--emit-uc-toml <path>` generates a starter `Uc.toml` scaffold.

1. `uc agent eval`
- Probes a manifest and returns an agent decision: proceed to build/benchmark, run a safe action and retry, or stop as native-unsupported.
- Always emits JSON and can also write it with `--report-path`.
- Includes the nested native support report, safe actions, manifest-specific next commands, and fallback/toolchain state.

1. `uc agent safe-action`
- Dry-run-first remediation surface.
- Supports `build-helper-lane`, `rebuild-helper-lane`, `refresh-cache`, `rerun-doctor`, and `regenerate-support-matrix`.
- Does not execute unless `--execute` is supplied.
- Emits a structured safe-action report with command, dry-run status, execution status, exit code, stdout, and stderr.

1. `uc replay <bundle>`
- Reads a `uc build --record-failure` bundle and emits a replay report.
- Dry-run by default; `--execute` replays the recorded command after stripping legacy `--record-failure` arguments so replay cannot overwrite the original evidence bundle.

1. `uc mcp serve`
- Emits the read-only MCP command/resource catalog as JSON.
- Covers `doctor`, `project_inspect`, `support_native`, `explain_diagnostic`, `select_toolchain`, `benchmark_report`, and `profile_native_frontend`.
- This is intentionally read-only: mutable actions stay behind `uc agent safe-action --execute`.

1. `uc daemon`
- `start`: launches local background daemon (`~/.uc/daemon/uc.sock` by default).
- `status`: checks daemon reachability and reports pid/start timestamp.
- `stop`: requests graceful shutdown.

## Target Agent-First Surface

The intended primary surface for agents is:

- `uc project inspect`
  - Status: implemented
  - Versioned JSON description of workspace, packages, targets, source origins, lockfile state, and offline readiness.

- `uc support native`
  - Status: implemented
  - Pre-build native support classification.
  - Reports agent-facing `decision_status` values `native_supported`, `native_unsupported`, `fallback_likely`, or `build_blocked`, with reason codes and remediation diagnostics.

- `uc resolve`
  - Status: implemented for `--locked`
  - Lockfile-first, read-only resolution and graph emission.
  - Reports source origins, lockfile-sync status, and explicit `network_intent=forbidden`.

- `uc fetch`
  - Status: implemented for `--locked`
  - Explicit source acquisition into the shared store.
  - Reports what was materialized, what was reused, and what is still missing.

- `uc toolchain ensure`
  - Status: planned (not yet implemented)
  - Ensures required Cairo/helper lanes exist.
  - Must expose expected/found toolchain details and policy decisions.

- `uc build --plan-only`
  - Status: implemented
  - Emits the execution plan and expected side effects without performing the build.
  - Reports planned backend, execution driver, fallback policy, daemon posture, and session/invalidation keys when they are determinable without running the build.

- `uc explain <id>`
  - Status: planned (not yet implemented)
  - Re-renders a prior failure or fallback decision from stored machine-readable state.

## Agent Contract

- `--json` output is primary for agent-facing commands.
- Versioned schemas should be explicit (`--format-version` or equivalent).
- Locked/offline commands must not mutate lockfiles or touch the network implicitly.
- Fallback must always be classified explicitly.
- Every execution report should include:
  - `what_happened`
  - `why`
  - `retryable`
  - `expected`
  - `found`
  - `fallback_used`
  - `replay_command` or `replay_handle`
  - `artifact_log_path`
  - `schema_version`
- Bump the schema version or explicit format version when these report-contract fields change incompatibly.

## Current Engine Note

`uc` now selects native toolchain lanes before compile starts:
- builtin lane for the compiler version baked into the active binary
- external helper lane via `UC_NATIVE_TOOLCHAIN_<major>_<minor>_BIN` for older Cairo majors/minors such as `2.14`

Native auto mode still falls back to Scarb only when the failure class is explicitly marked fallback-eligible. The fallback path is now surfaced in build reports and benchmark support-matrix output instead of being inferred from logs.

## Source Store Note

`uc fetch` currently uses Scarb as the explicit networked resolver/fetch driver, but it does not stop there: it materializes the resulting locked package roots into `uc`'s own shared source store and exposes:

- `uc cache status --format json`
- `uc cache prune --format json`

Agents should treat the `uc` source store as the authoritative local fetch surface and the current `execution_driver` as part of the rollout evidence, not as hidden behavior.

## Helper Lane Operations

- `./scripts/build_native_toolchain_helper.sh --lane 2.14`
  - Builds a Cairo `2.14` helper binary from the current repo in an isolated staging tree.
  - Produces a binary suitable for `UC_NATIVE_TOOLCHAIN_2_14_BIN`.
- `./scripts/build_native_toolchain_helper.sh --lane 2.14 --check-only`
  - Compiles the helper compatibility feature against the pinned Cairo `2.14` staging tree without producing a release binary.
- `./scripts/doctor.sh --uc-bin /abs/path/to/uc --manifest-path /abs/path/to/Scarb.toml`
  - Probes native support for a real manifest before build time.
  - Fails on missing or invalid helper-lane env vars for that manifest.

## Next Expansion
- Add more native toolchain helper lanes beyond Cairo `2.14`.
- Expand `resolve` beyond `--locked`, add first-party `fetch` and `toolchain ensure`, and keep hardening the implemented `build --plan-only` surface behind stable JSON/report contracts.
- Add native `uc` compile engine implementation behind the existing command interface.
- Keep `compare-build` as mandatory gate while deeper frontend-compile optimizations mature.

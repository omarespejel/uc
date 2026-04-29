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
  - `execution_driver` (`uc_local` for local no-op hydration; otherwise `scarb_direct` when Scarb performed fetch or offline metadata replay)
  - `source_store` summary
  - `fetched_entries` and `missing_entries`
  - `offline_readiness_before` and `offline_readiness_after`
- `uc fetch` currently requires `--locked`; there is no implicit mutable resolution mode yet.

1. `uc cache status`
- Reads the current shared source-store inventory.
- Supports: `--format json`, `--json`, `--report-path`.
- Emits:
  - `schema_version`
  - `generated_at_epoch_ms`
  - `readonly`
  - `mutation_status`
  - source-store root
  - availability / writability
  - entry counts and total bytes
  - invalid entry count
  - configured byte budget
  - `what_happened`, `why`, and `retryable`
  - `expected`, `found`, `fallback_used`, `replay_command`, `artifact_path`, `log_path`

1. `uc cache prune`
- Prunes the shared source store to the configured byte budget.
- Supports: `--max-bytes`, `--format json`, `--json`, `--report-path`.
- Emits:
  - `schema_version`
  - `generated_at_epoch_ms`
  - `readonly`
  - `mutation_status`
  - source-store root
  - availability / writability
  - configured byte budget used for pruning
  - pre/post entry counts
  - pre/post byte totals
  - removed entry count, bytes, and keys
  - `what_happened`, `why`, and `retryable`
  - `expected`, `found`, `fallback_used`, `replay_command`, `artifact_path`, `log_path`

1. `uc toolchain ensure`
- Ensures the native Cairo/helper lane selected from the manifest is locally available.
- Supports: `--manifest-path`, `--format json`, `--json`, `--report-path`.
- Behavior:
  - If the requested lane matches the builtin compiler, the command is a structured no-op.
  - If a usable external helper is already configured or discoverable at the default helper path, the command is a structured no-op.
  - If the lane is productized but missing or invalid, `uc` explicitly runs the checked-in helper builder and then revalidates the lane.
  - If the lane is not productized or the manifest cannot be inspected safely, the command returns `build_blocked` with stable diagnostics.
- Emits a stable ensure report with:
  - top-level `status` (`ready` or `build_blocked`)
  - `execution_driver` (`uc_builtin`, `uc_external_helper`, `helper_builder_script`, or `null` when no driver ran)
  - selected `toolchain`
  - `ensured_now`
  - `mutation_status`
  - `subprocess_commands`
  - `blocked_reason`
  - consumers must treat `execution_driver=null` as valid for blocked pre-driver states and rely on `status` plus `toolchain` for control flow
- Productized helper lanes are now discoverable from their default output path under `~/.uc/toolchain-helpers/...` even when the corresponding `UC_NATIVE_TOOLCHAIN_<major>_<minor>_BIN` env var is unset.

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
- Covers `uc.doctor`, `uc.project_inspect`, `uc.support_native`, `uc.explain_diagnostic`, `uc.select_toolchain`, `uc.fetch`, `uc.cache_status`, `uc.cache_prune`, `uc.toolchain_ensure`, `uc.benchmark_report`, and `uc.profile_native_frontend`.
- The catalog itself is read-only; adapters must honor each tool's `mutates_state` flag before executing mutable surfaces such as `fetch`, `cache prune`, or `toolchain ensure`.

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

- `uc cache status`
  - Status: implemented
  - Read-only inventory of the shared source store.

- `uc cache prune`
  - Status: implemented
  - Explicit source-store budget enforcement.

- `uc toolchain ensure`
  - Status: implemented
  - Ensures required Cairo/helper lanes exist.
  - Reports expected/found toolchain details, helper-builder subprocesses, and whether the lane was ensured during the command.

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

## Toolchain Ensure Note

`uc toolchain ensure` is the explicit mutable toolchain-acquisition surface. It is allowed to build a productized helper lane because the mutation is requested directly by the command, not implied by a support probe or build.

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
- Expand `resolve` beyond `--locked` and keep hardening the implemented `fetch`, `toolchain ensure`, and `build --plan-only` command/report contracts.
- Add native `uc` compile engine implementation behind the existing command interface.
- Keep `compare-build` as mandatory gate while deeper frontend-compile optimizations mature.

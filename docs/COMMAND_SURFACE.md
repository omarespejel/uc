# Command Surface (Current)

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

6. `uc support native`
- Probes whether a manifest is eligible for native compile in the current `uc` binary.
- Supports: `--manifest-path`, `--format text|json`, `--json`.
- Returns a structured reason for ineligible manifests so scripts and local benchmark harnesses can classify cases before measuring them.
- Native support JSON includes selected toolchain lane and stable diagnostics for:
  - exact `cairo-version` mismatches
  - unsupported manifest constraints
  - missing or invalid external helper lanes
  - unparseable compiler versions

7. `uc migrate`
- Analyzes `Scarb.toml` and emits a migration readiness report.
- Optional `--emit-uc-toml <path>` generates a starter `Uc.toml` scaffold.

8. `uc daemon`
- `start`: launches local background daemon (`~/.uc/daemon/uc.sock` by default).
- `status`: checks daemon reachability and reports pid/start timestamp.
- `stop`: requests graceful shutdown.

## Current Engine Note
`uc` now selects native toolchain lanes before compile starts:
- builtin lane for the compiler version baked into the active binary
- external helper lane via `UC_NATIVE_TOOLCHAIN_<major>_<minor>_BIN` for older Cairo majors/minors such as `2.14`

Native auto mode still falls back to Scarb only when the failure class is explicitly marked fallback-eligible. The fallback path is now surfaced in build reports and benchmark support-matrix output instead of being inferred from logs.

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
- Keep `compare-build` as mandatory gate while deeper frontend-compile optimizations mature.

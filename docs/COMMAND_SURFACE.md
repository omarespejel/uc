# Command Surface (Current)

## Implemented Commands

1. `uc build`
- Executes build path with selectable engines.
- Supports: `--manifest-path`, `--package`, `--workspace`, `--features`, `--offline`, `--release`, `--profile`, `--daemon-mode`.
- Engines:
  - `--engine uc` (default): deterministic fingerprint + local artifact cache restore fast-path.
    - daemon policy via `--daemon-mode off|auto|require` (default: `off`).
  - `--engine scarb`: direct Scarb execution path.
- Optional `--report-path` writes execution report JSON.

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

6. `uc migrate`
- Analyzes `Scarb.toml` and emits a migration readiness report.
- Optional `--emit-uc-toml <path>` generates a starter `Uc.toml` scaffold.

7. `uc daemon`
- `start`: launches local background daemon (`~/.uc/daemon/uc.sock` by default).
- `status`: checks daemon reachability and reports pid/start timestamp.
- `stop`: requests graceful shutdown.

## Current Engine Note
`uc` engine currently uses Scarb execution for cache misses and deterministic local cache restore for hits. This is a bootstrap native path to capture warm speedups while keeping parity gates.

## Next Expansion
- Add native `uc` compile engine implementation behind the existing command interface.
- Keep `compare-build` as mandatory gate while native path matures.

# Command Surface (Current)

## Implemented Commands

1. `uc build`
- Executes build path with Scarb-backed engine.
- Supports: `--manifest-path`, `--package`, `--workspace`, `--features`, `--offline`, `--release`, `--profile`.
- Optional `--report-path` writes execution report JSON.

2. `uc metadata`
- Executes metadata resolution path.
- Supports: `--manifest-path`, `--format-version`, `--offline`, `--global-cache-dir`.
- Optional `--report-path` writes execution report JSON.

3. `uc compare-build`
- Runs direct Scarb build vs `uc build` wrapper on same manifest.
- Compares artifact hashes and diagnostics lines.
- Writes JSON report and enforces pass/fail gate.

4. `uc benchmark`
- Runs benchmark matrix harness script.

5. `uc session-key`
- Generates deterministic session key from normalized input fields.

## Current Engine Note
`uc build` currently uses `engine=scarb` as the bootstrap execution backend. This is intentional for phased correctness gating before native engine swap.

## Next Expansion
- Add native `uc` compile engine implementation behind the existing command interface.
- Keep `compare-build` as mandatory gate while native path matures.

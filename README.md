# Ultra Cairo (`uc`)

`uc` is an agent-first Cairo project control plane. It combines project inspection, dependency planning, source fetching, toolchain state, build planning, native execution, and structured diagnostics behind one deterministic interface.

The goal is not just a faster terminal command. The goal is a Cairo tool that agents can use directly: inspect the project, understand support before build, prepare dependencies, plan execution, run the native path when available, and act on machine-readable failures when something is missing.

## Current Status

`uc` currently provides:

- read-only project inspection,
- native support classification before build,
- locked dependency resolution reports,
- source fetch and source-store reports,
- explicit toolchain/helper checks,
- build plan reports,
- native build execution for supported Cairo lanes,
- compatibility fallback classification,
- replayable failure bundles,
- strict benchmark harnesses for supported workloads.

The native path is intentionally scoped. Supported projects can use the accelerated path; unsupported projects are classified explicitly and can route through the compatibility backend. Fallback is reported as fallback, not native success.

## Agent-First Design

Every important phase should be separately inspectable:

- `project inspect`
- `support native`
- `resolve`
- `fetch`
- `toolchain ensure`
- `build --plan-only`
- `build`
- `replay`

Machine-readable reports are the source of truth. Human-readable output is only a rendering of the same state.

## Quick Start

```bash
make bootstrap
make doctor
make agent-validate
cargo run -p uc-cli -- project inspect --manifest-path /abs/path/to/project.toml --format json
cargo run -p uc-cli -- support native --manifest-path /abs/path/to/project.toml --format json
cargo run -p uc-cli -- resolve --locked --manifest-path /abs/path/to/project.toml --format json
cargo run -p uc-cli -- fetch --locked --manifest-path /abs/path/to/project.toml --format json
cargo run -p uc-cli -- toolchain ensure --manifest-path /abs/path/to/project.toml --format json
cargo run -p uc-cli -- build --engine uc --daemon-mode off --manifest-path /abs/path/to/project.toml --plan-only --json
cargo run -p uc-cli -- build --engine uc --daemon-mode off --manifest-path /abs/path/to/project.toml --json
```

## Repository Structure

- `crates/uc-cli`: CLI, daemon, project model, reports, native build orchestration, and cache behavior.
- `crates/uc-core`: shared artifact/session primitives.
- `docs/agent`: stable agent-facing docs and JSON schemas.
- `benchmarks`: benchmark harnesses, corpora helpers, gates, and result outputs.
- `scripts`: local validation, bootstrap, helper-lane, and repo-maintenance scripts.
- `toolchains`: helper-lane metadata and patches.
- `third_party`: small vendored Cairo crates used by the native path.

## Validation

Use local validation before publishing changes:

```bash
cargo fmt --all
make agent-validate
git diff --check
make local-ci
```

For performance work, only quote numbers from strict same-window benchmark artifacts and include the exact lane, host, sample counts, and daemon mode.

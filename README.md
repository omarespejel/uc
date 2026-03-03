# uc

`uc` is a next-generation Cairo package manager and build/proving engine that will replace Scarb for our workflows.

## Mission
- Deliver materially faster developer and CI loops than Scarb.
- Keep deterministic artifacts and reproducible builds as hard requirements.
- Provide a modern 2026 architecture: sessionized compiler state, content-addressed cache, and measurable performance gates.

## First Product Proof
The first thing we must prove is that `uc` beats Scarb on warm rebuild latency while preserving correctness.

- Hypothesis: `uc` can reduce warm `edit -> build` p95 by at least 40%.
- Guardrails: zero artifact hash mismatches and diagnostics parity >= 99.5%.
- Decision gate: continue full replacement only if proof passes.

See:
- `docs/FIRST_PROOF.md`
- `docs/BENCHMARK_PLAN.md`
- `docs/ARCHITECTURE_BLUEPRINT.md`
- `docs/COMMAND_SURFACE.md`

## Repository Structure
- `docs/`: product, architecture, roadmap, KPIs, operating model, cutover.
- `docs/research/`: imported and synthesized research from the codebase exploration.
- `docs/adr/`: architecture decision records.
- `benchmarks/`: scenario matrix, harness, fixtures, results, and baselines.
- `scripts/github/`: GitHub milestones/labels/issues bootstrap.
- `.github/`: issue templates, PR template, and benchmark CI workflow.

## Quick Start
```bash
make bootstrap
make benchmark-local
make compare-local
cargo run -p uc-cli -- build --manifest-path /path/to/Scarb.toml
cargo run -p uc-cli -- compare-build --manifest-path /path/to/Scarb.toml
```

## Tooling
- `scarb`
- `jq`
- `git`
- `cargo`

## Current Status
- Program foundations are set.
- Baseline benchmarking against Scarb is automated and committed.
- `uc` now has executable commands for build, metadata, benchmark orchestration, and dual-run comparator reports.

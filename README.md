# uc

Future-first Cairo build and proving platform.

## Mission
- Replace legacy project tooling with a faster, deterministic, cloud-ready developer and CI workflow.
- Prioritize measurable performance, reproducibility, and migration at scale.

## Program Targets
- p95 warm edit->build under 3 seconds on representative large workspaces.
- 70%+ CI cache reuse across branches.
- Deterministic artifacts and auditable build provenance.

## Repository Structure
- `docs/`: product, roadmap, KPIs, benchmark strategy, cutover plan.
- `.github/`: issue/PR templates and automation workflows.
- `benchmarks/`: scenario definitions and benchmark outputs.
- `scripts/github/`: scripts to bootstrap labels, milestones, issues, and project setup.

## Operating Model
- Work tracked by Milestones + Project board + seeded issues.
- Weekly metric review from benchmark and KPI scorecard.
- Go/No-Go gates based on latency, correctness, and reliability.

## Quick Start
```bash
make bootstrap
make benchmark-local
make gh-bootstrap
```

## Required Tooling
- `gh` (authenticated with `repo` scope)
- `jq`
- `hyperfine`
- `git`

## Notes
- Project name is `uc`.
- Migration from existing tooling is explicit and staged; this repo is the source of truth for execution.

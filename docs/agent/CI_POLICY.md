# CI Policy

## The split

**CI runs correctness. Local runs performance.**

| Lane | Where | Why |
|---|---|---|
| Format, lint (clippy), unit tests, native smoke tests, artifact parity | GitHub Actions, on push to `main` and on every PR | Deterministic, cheap, and must never regress unnoticed |
| Benchmarks (`make benchmark-strict-*`, `make perf-fast`) | Local only, pinned host | Shared runners have unstable CPU allocation; a number from a noisy runner is worse than no number |

## Why this changed

This repo previously ran **no** automatic GitHub Actions: validation was local-first through
`make local-ci` and the repo-managed pre-push hook, and the single workflow was
`workflow_dispatch`-only. That policy was adopted to bound GitHub spend, and it worked while the
repo was under daily development.

It failed in a specific way worth recording. Between 2026-05-02 and 2026-08-01 `main` received no
commits, and during that window nothing verified that `main` still built or that its tests still
passed. Local-first gates only protect branches that a human is actively pushing. A dormant branch
under local-only validation is an unverified branch.

The correctness lanes are therefore now automatic. The cost is bounded: the jobs are the same ones
`make validate-fast` already runs, plus a parity check, on two platforms.

## What CI must not become

- **No benchmark lanes.** Timing on shared runners is not evidence. Performance claims come from
  `benchmarks/scripts/run_stability_benchmarks.sh` on a pinned local host, same-window, per
  `benchmarks/README.md`.
- **No lane that cannot fail deterministically.** A flaky required check trains everyone to ignore
  required checks.
- **Not a replacement for the local gate.** `make local-ci` and the pre-push hook stay authoritative
  for the fast inner loop; CI is the backstop, not the first line.

## Changing this policy

CI scope changes require updating this file in the same PR. The compliance checklist item
"Validation split is respected" enforces that.

## Current workflows

| Workflow | Trigger | Contents |
|---|---|---|
| `.github/workflows/ci.yml` | push to `main`, pull_request | fmt, clippy, `validate-fast`, native tests, artifact parity smoke |
| `.github/workflows/agent-surface.yml` | `workflow_dispatch` | agent surface validation, run on demand |

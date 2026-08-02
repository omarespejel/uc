# Agent Surface

This directory is the checked-in handoff layer for coding agents, humans, and PR review bots.

## Files

- `AGENT_FIRST_COMPILER.md`: product philosophy and command flow.
- `AGENT_DIAGNOSTICS.md`: stable diagnostic-code contract for JSON consumers.
- `AGENT_QUICKSTART.md`: command sequence agents should prefer before build, fix, or benchmark work.
- `HUMAN_QUICKSTART.md`: compact human command sequence for the same surfaces.
- `PR_LIFECYCLE.md`: how to take a change to `main` — branching, gating, bot-finding triage, merge criteria. Read before your first commit.
- `PR_BOT_POLICY.md`: how CodeRabbit and Qodo are configured and how their output should be interpreted.
- `CI_POLICY.md`: what runs in GitHub Actions versus locally, and why.
- `REPO_MAP.md`: generated map of current repo entrypoints and hot files.
- `schemas/`: JSON schemas for diagnostic, project inspect, support, resolve, fetch, toolchain, source-store, build-plan, and build outputs.

## Rules

- Machine-readable reports are the source of truth.
- Every command that can mutate state must expose what it will do before it does it.
- Fallback is compatibility behavior and must be reported explicitly.
- Locked and offline flows must not touch network, toolchain, or lockfile state silently.
- Benchmark evidence must state lane, host, sample counts, and daemon mode.

## Validation

```bash
make agent-map
make agent-validate
```

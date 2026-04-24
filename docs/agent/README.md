# Agent Surface

This directory is the checked-in handoff layer for humans, coding agents, and PR review bots.

## Files

- `AGENTS.md`: root operating rules and high-risk areas.
- `.codex/START_HERE.md`: exact bootstrap sequence for a fresh clone.
- `PR_BOT_POLICY.md`: how CodeRabbit and Qodo should be configured and interpreted.
- `REPO_MAP.md`: generated map of the current repo entrypoints and hot files.

## Why This Exists

Modern repo-agent workflows work best when review instructions live with the code, not in thread history. Both CodeRabbit and Qodo support checked-in guidance:
- CodeRabbit recommends repo-root YAML config and can read code-guideline files like `AGENTS.md` automatically.
- Qodo supports repo-root `.pr_agent.toml`, `best_practices.md`, `pr_compliance_checklist.yaml`, and auto-detects `AGENTS.md`.

This repo also uses a checked-in local validation contract:
- repo-managed Git hooks under `.githooks/`
- a path-aware local push gate in `scripts/local_ci_gate.sh`
- manual-only GitHub Actions workflows for exceptional remote runs

That split keeps PR bot review on GitHub while moving routine tests and benchmarks back to the developer machine.

## Sources

- CodeRabbit configuration overview: <https://docs.coderabbit.ai/guides/configuration-overview>
- CodeRabbit path instructions: <https://docs.coderabbit.ai/configuration/path-instructions>
- CodeRabbit custom checks: <https://docs.coderabbit.ai/pr-reviews/custom-checks>
- Qodo `.pr_agent.toml`: <https://docs.qodo.ai/code-review/get-started/configuration-overview/configuration-file>
- Qodo `AGENTS.md`: <https://docs.qodo.ai/qodo-gen/agent/agents.md-support>
- Qodo best practices: <https://docs.qodo.ai/v1/features/best-practices>
- Qodo compliance checklist: <https://docs.qodo.ai/qodo-documentation/qodo-merge/pr-agent/tools/compliance>
- GitHub manual workflows (`workflow_dispatch`): <https://docs.github.com/en/actions/how-tos/managing-workflow-runs-and-deployments/managing-workflow-runs/manually-running-a-workflow>
- Git hooks and `core.hooksPath`: <https://git-scm.com/docs/githooks>

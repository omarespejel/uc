# uc PR Review Loop

Use this skill when addressing PR comments or preparing a branch for review.

## Steps
1. Read `AGENTS.md` and `.codex/START_HERE.md`.
2. Run `make doctor`.
3. Inspect bot config files: `.coderabbit.yaml`, `.pr_agent.toml`, `best_practices.md`, `pr_compliance_checklist.yaml`.
4. Implement the smallest coherent fix.
5. Run focused validation first, then broader validation if the risk justifies it.
6. Run `make agent-validate` if repo entrypoints or docs changed.
7. Before merge, ensure the PR is quiet for at least 3 minutes with no new actionable AI bot feedback.

## Review Priorities
- correctness regressions
- invalidation/cache safety
- daemon safety
- deterministic outputs
- benchmark rigor
- missing regression coverage

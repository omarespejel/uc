# PR Bot Policy

## Goals

- Keep AI reviews focused on production bugs, invalidation errors, daemon safety, artifact drift, missing tests, and benchmark methodology.
- Minimize style-only noise that humans or formatters already cover.
- Make the repository easy for both review bots and coding agents to bootstrap correctly.

## CodeRabbit

- Use repo-local `.coderabbit.yaml` as the primary configuration source.
- Keep path instructions narrow and file-type specific.
- Prefer checked-in code-guideline files (`AGENTS.md`, `.codex/START_HERE.md`) for broad repo rules.
- Do not overuse custom checks; they should be reserved for crisp pass/fail rules because they run in a read-only sandbox and cannot execute the full test suite.

## Qodo

- Keep `.pr_agent.toml` small and repo-specific.
- Put durable coding standards in `best_practices.md`.
- Put hard business or engineering gates in `pr_compliance_checklist.yaml`.
- Keep repo-level standards concise so the agent actually applies them.

## Human Triage Rules

- Every agent should read `AGENTS.md` and `.codex/START_HERE.md` before changing code, then follow this PR loop by default.
- Open a normal ready-for-review PR early for any non-trivial change so review bots have a real diff to inspect.
- Do not use draft PRs for normal engineering work in this repo; CodeRabbit and Qodo need a reviewable PR state.
- Treat the PR as the working unit: push small coherent slices, let bots review, then address relevant findings before expanding scope.
- Treat CodeRabbit and Qodo as review accelerators, not as merge authority.
- Fix real correctness or regression findings first.
- If two bots disagree, verify in code and tests rather than following either blindly.
- Resolve comments only when the code or rationale is clearly complete.
- Merge only after a 3-minute quiet period with no new useful AI feedback.

## Repo-Specific Focus

- Native compile state reuse must stay conservative under uncertainty.
- Cache/session invalidation bugs are higher priority than small latency wins.
- Benchmarks are only valid when lane conditions remain pinned and repeatable.
- Docs, commands, and repo-map entrypoints must stay in sync with code changes.

## Sources

- CodeRabbit path instructions and code guidelines: <https://docs.coderabbit.ai/configuration/path-instructions>
- CodeRabbit custom checks limits: <https://docs.coderabbit.ai/pr-reviews/custom-checks>
- Qodo `.pr_agent.toml`: <https://docs.qodo.ai/code-review/get-started/configuration-overview/configuration-file>
- Qodo `best_practices.md`: <https://docs.qodo.ai/v1/features/best-practices>
- Qodo `pr_compliance_checklist.yaml`: <https://docs.qodo.ai/qodo-documentation/qodo-merge/pr-agent/tools/compliance>

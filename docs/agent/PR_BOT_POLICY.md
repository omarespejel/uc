# PR Bot Policy

## Goals

- Keep AI reviews focused on production bugs, invalidation errors, daemon safety, artifact drift, missing tests, and benchmark methodology.
- Minimize style-only noise that humans or formatters already cover.
- Make the repository easy for both review bots and coding agents to bootstrap correctly.
- Keep GitHub spend bounded by using PR bots for review, while running routine tests and benchmarks locally.

> Full end-to-end procedure, including bot-finding triage rules and the standing gotchas:
> **`docs/agent/PR_LIFECYCLE.md`**. This file covers bot *configuration*; that one covers
> how to *work* with them.

## Agent review loop

Agents must not scrape the PR web UI. Read bot feedback as JSON:

```sh
make pr-feedback            # everything, for the current branch's PR
make pr-feedback-open       # only unresolved threads
make pr-feedback PR=73      # a specific PR
```

The output contains `counts.unresolved_bot_threads`, `counts.failing_checks`, per-thread
`path`/`line`/`body`/`resolved`, and the check rollup. The loop is:

1. Push a coherent slice, open a ready-for-review PR.
2. Poll `make pr-feedback-open` until the bots have posted.
3. Fix real findings; reply or resolve with rationale where a finding is wrong.
4. Re-run until `unresolved_bot_threads` is 0 and `failing_checks` is 0.
5. Merge after the quiet period below.

CodeRabbit inline comments include a "Prompt for AI Agents" block
(`reviews.enable_prompt_for_ai_agents`) — a ready-made codegen instruction for the finding.
Use it as input, not as authority: verify the claim against the code before applying.

## Pre-merge checks

`.coderabbit.yaml` defines pre-merge checks that gate the PR when they fail in `error` mode
(paired with `request_changes_workflow: true`):

| Check | Mode | Gate |
|---|---|---|
| Title | error | imperative mood, names the surface, under ~72 chars |
| Description | error | follows the PR template |
| Invalidation regression coverage | error | reuse/restore changes ship with a test or a justification |
| No silent degradation | error | every degradation path records a diagnostic |
| Agent contract stability | error | schema changes additive or `schema_version` bumped |
| Benchmark claim integrity | warning | perf numbers state harness, lane, samples, comparison build |
| Linked issue assessment | warning | advisory |
| Docstring coverage | off | deliberately disabled; see the comment in `.coderabbit.yaml` |

Do not silence a failing check by weakening it. Either fix the PR or, if the check is wrong for
this change, say so in the PR and let a human override.

## CodeRabbit

- Use repo-local `.coderabbit.yaml` as the primary configuration source.
- Keep path instructions narrow and file-type specific.
- Prefer checked-in code-guideline files (`AGENTS.md`, `.codex/START_HERE.md`) for broad repo rules.
- Do not overuse custom checks; they should be reserved for crisp pass/fail rules because they run in a read-only sandbox and cannot execute the full test suite.
- Every CodeRabbit tool defaults to **enabled**. Only write an entry under `reviews.tools` to turn
  something off, or to keep a security-relevant tool explicitly visible. Do not add `enabled: true`
  lines that merely restate the default.
- `ast-grep` has no `enabled` key — it is configured with `rule_dirs`/`packages`. Writing
  `ast-grep: {enabled: false}` is a schema error that silently invalidates that block.
- Validate config changes against the published schema before pushing:
  `curl -sL https://coderabbit.ai/integrations/schema.v2.json` and check with any JSON-Schema validator.

## Qodo

- Keep `.pr_agent.toml` small and repo-specific.
- Put durable coding standards in `best_practices.md`.
- Put hard business or engineering gates in `pr_compliance_checklist.yaml`.
- Keep repo-level standards concise so the agent actually applies them.

## Human Triage Rules

- Open a normal ready-for-review PR early for any non-trivial change so review bots have a real diff to inspect.
- Do not use draft PRs for normal engineering work in this repo; CodeRabbit and Qodo need a reviewable PR state.
- Treat the PR as the working unit: push small coherent slices, let bots review, then address relevant findings before expanding scope.
- GitHub Actions are manual-only in this repo by policy. Do not wait for automatic CI that should not exist; use the local validation lanes and pre-push hook instead.
- Treat CodeRabbit and Qodo as review accelerators, not as merge authority.
- Fix real correctness or regression findings first.
- If two bots disagree, verify in code and tests rather than following either blindly.
- Resolve comments only when the code or rationale is clearly complete.
- Merge only after a 6-minute quiet period with no new useful AI feedback, matching `AGENTS.md`.

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

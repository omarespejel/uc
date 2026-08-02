# PR Lifecycle

How a change gets from an idea to `main` in this repo. Applies to humans and coding
agents alike; agents should treat it as executable procedure.

Bots (CodeRabbit, Qodo) review every PR. **Treat their findings as an adversarial
reviewer whose claims you must verify — not as ground truth to obey, and not as noise to
ignore.**

## 1. Branch

- Never commit to `main`.
- One feature branch per logical change; one logical change per commit.
- Imperative commit subjects, no trailing period, ideally under 72 characters.
- Prefer a fresh worktree for parallel work.

## 2. Gate locally, and read the real exit code

Run the gate on your branch head **before** opening a PR:

```sh
make local-ci
```

Scoped lanes while iterating: `make validate-fast`, `make validate-native`,
`cargo test -p uc-cli --bin uc`, `make agent-validate`.

**Read the gate's own status, not a pipeline's tail.** This is the single most common way
an agent convinces itself a red gate is green:

```sh
# WRONG — reports grep's status, and prints nothing useful when the build failed
cargo test -p uc-cli 2>&1 | grep -E "^error" | head -20

# RIGHT — capture the command's own exit code
cargo test -p uc-cli > /tmp/uc_test.log 2>&1; echo "EXIT=$?"
grep -E "^error" -A 8 /tmp/uc_test.log | head -40
```

`${PIPESTATUS[0]}` also works in bash, but only immediately after the pipeline and only
under bash — it silently yields nothing under `sh`. Redirect to a file and check `$?`.

Backgrounded commands report the status of the *last* command in the chain, not your
gate. Record the gate's status explicitly if you background it.

## 3. Open the PR

Fill the template: Summary, Linked Issue, Change Type, Validation (what you ran and that
it passed), Risks, Rollback Plan. Open it ready-for-review, not draft — the bots need a
reviewable state.

Pre-merge checks gate the title and description (see `PR_BOT_POLICY.md`). Write the title
in imperative mood naming the surface.

## 4. Triage every bot finding

This is the core of the job. For **each** finding:

1. **Read it in full** — body, code location, suggested fix. Not just the title or the
   severity badge. Severity labels are frequently miscalibrated.
2. **Verify it independently against the actual code.** Reproduce the claimed failure:
   construct the exact input or tamper the bot describes and confirm it really breaks.
   Bots produce false positives, misattributions ("you introduced X" when X predates the
   PR), and findings that are true in general but not in this codebase.
3. **Check the history** when a bot claims your PR introduced something:
   `git log -S '<snippet>' -- <path>` or `git blame`.
4. **If real:** fix it minimally and add a regression test that reproduces the exact issue
   described. Re-verify the fix closes it and that the honest path still passes.
5. **If invalid or out of scope:** decline it explicitly, as a PR reply, with specific
   verified reasoning. Never silently ignore a finding. Good declines name the reason:
   "pre-existing, not introduced here (see `<sha>`)"; "this bound is deliberately loud —
   see the comment above it"; "the flagged line is a test fixture, not production".
6. **Recognize recurring false-positive patterns.** Some rules fire on every PR of a given
   shape. Decline consistently with the same reasoning rather than re-litigating.
7. **A security or critical badge on a security-sensitive change deserves extra rigor, not
   reflexive dismissal.** Those are exactly where a real gap hides. Prove it is wrong
   before declining it.

Read findings as JSON rather than scraping the UI:

```sh
make pr-feedback-open    # unresolved threads only
make pr-feedback         # everything, plus the check rollup
```

### Repo-specific priorities when triaging

A finding that touches these is presumed real until you prove otherwise:

- Cache or fingerprint reuse that could serve a stale artifact (**a false hit is always
  worse than a false miss**).
- A degradation path — fallback, daemon-unavailable, offline downgrade — that does not
  record a diagnostic.
- A JSON report or schema field removed, renamed, or retyped without a `schema_version`
  bump, or a failure path that exits without emitting JSON in `--json` mode.
- A performance claim not backed by a same-window strict-harness run.

## 5. Re-gate, dispose, merge

- After pushing fixes, **re-run the gate on the new head** and confirm green. Do not rely
  on the previous run.
- Bot inline comments on lines you changed become "outdated" automatically. A thread still
  live is usually one you declined — fine, if it is dispositioned with reasoning.
- Merge only when **all** hold:
  - gate PASS on the fixed head,
  - no unresolved substantive threads (declined ones answered),
  - `mergeable` is clean,
  - `counts.failing_checks` is 0,
  - the PR has been quiet for at least 6 minutes (bots review on push and may lag).
- Post a final comment summarizing dispositions: "Finding A fixed in `<sha>` + regression;
  Finding B declined because …; gate PASS."
- Merge, then delete the branch.

## 6. Standing gotchas

- **A PR-diff gate run after merge sees an empty diff** (HEAD == base) and can pass or
  fail benignly. Validate on the PR head, not the merged tip.
- **Pre-commit and pre-push hooks may reject compound commands** (`git push … && <reply>`).
  Split them.
- **Schema and fixture pins cascade.** If a change makes a checked-in digest, schema, or
  golden fixture stale, regenerate it, then diff and confirm *only* what you intended
  changed. An unexpected flip means unintended drift — investigate it rather than
  accepting the new value. After updating, grep for the old value to confirm no stale
  copies remain. Never edit a pin to make a check pass without understanding what the
  check protects.
- **Never fabricate.** No invented benchmark numbers, commit hashes, issue numbers, or
  citations. If a value is unconfirmable, say so.
- **Confirm before irreversible or outward-facing steps** — merging to a shared branch,
  force-pushing, publishing. Approval for one action does not extend to the next.

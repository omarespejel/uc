# uc Start Here

## 5-Minute Bootstrap

1. `make bootstrap`
2. `make doctor`
3. `make agent-validate`
4. Read `docs/agent/REPO_MAP.md`
5. Read the subsystem doc you are changing:
   - architecture: `docs/ARCHITECTURE_BLUEPRINT.md`
   - roadmap: `docs/ROADMAP.md`
   - benchmarks: `docs/BENCHMARK_PLAN.md`, `benchmarks/README.md`
   - supremacy/perf research: `docs/SUPREMACY_RESEARCH_2026-03-06.md`
6. If the task is larger than a trivial one-line fix, create or reuse a scoped branch and plan to open a PR before broadening the change.
7. Open normal PRs, not draft PRs, so CodeRabbit and Qodo review the branch immediately.

## Common Commands

- Install repo hooks: `make install-hooks`
- Local push gate: `make local-ci`
- Format: `cargo fmt --all`
- Fast validation: `make validate-fast`
- Native validation: `make validate-native`
- Refresh repo map: `make agent-map`
- Strict smoke benchmark: `make benchmark-strict-smoke`
- Strict research benchmark: `make benchmark-strict-research`

## Key Files

- `crates/uc-cli/src/main.rs`: build path, daemon, native compile session, persisted state.
- `crates/uc-cli/src/fingerprint.rs`: semantic hashing and fingerprint cache.
- `crates/uc-cli/src/main_tests.rs`: regression-heavy unit coverage.
- `benchmarks/scripts/`: harnesses and gates.
- `.coderabbit.yaml`, `.pr_agent.toml`, `best_practices.md`, `pr_compliance_checklist.yaml`: PR bot behavior.

## Expected Workflow

- Start in a fresh clone or worktree.
- Install repo-managed hooks immediately; local validation is the primary gate in this repo.
- Make the smallest coherent change that can be tested.
- Add tests before or with risky code changes.
- Re-run focused validation before broader benchmarks.
- Update `docs/agent/REPO_MAP.md` with `make agent-map` when repo entrypoints change.
- Push coherent slices to a PR instead of holding large local diffs.
- Assume GitHub Actions are disabled or manual-only. If you need a remote workflow run, trigger it deliberately with `workflow_dispatch`; do not expect automatic CI on push or PR.
- Keep the PR in ready-for-review state; do not switch to draft unless a human explicitly asks for it.
- After each meaningful push, run the review loop: check CodeRabbit and Qodo, fix relevant findings, and only merge after a 3-minute quiet window with no new useful bot feedback.

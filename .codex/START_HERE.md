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
- Helper-lane validation: `make validate-helper-lane`
- Refresh repo map: `make agent-map`
- Strict smoke benchmark: `make benchmark-strict-smoke`
- Strict research benchmark: `make benchmark-strict-research`
- Build deployed-contract source index: `benchmarks/scripts/build_deployed_contract_source_index.sh --inventory /abs/path/to/source-inventory.json --out /abs/path/to/pinned-deployed-contract-source-index.json`
- Generate deployed-contract corpus: `benchmarks/scripts/generate_deployed_contract_corpus.sh --source-index /abs/path/to/source-index.json --out /abs/path/to/generated-corpus.json`
- Run deployed-contract corpus evidence: `benchmarks/scripts/run_deployed_contract_corpus.sh --corpus /abs/path/to/generated-corpus.json`

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
- For older Cairo native repos, build the helper with `./scripts/build_native_toolchain_helper.sh --lane 2.14` and export the printed `UC_NATIVE_TOOLCHAIN_2_14_BIN`.
- For helper-lane patch experiments, use lane metadata `patch-dir`; keep patch files in `toolchains/cairo-2.14/patches/*.patch`. The helper applies them only in staging and honors `UC_HELPER_CARGO_REGISTRY_SRC` for an alternate Cargo registry source cache.
- Before measuring a real manifest, run `./scripts/doctor.sh --uc-bin /abs/path/to/uc --manifest-path /abs/path/to/Scarb.toml` to catch missing helper lanes early.
- For deployed-contract corpus claims, build the source index from a reviewed source inventory with `benchmarks/scripts/build_deployed_contract_source_index.sh`, generate the run corpus with `benchmarks/scripts/generate_deployed_contract_corpus.sh`, then run `benchmarks/scripts/run_deployed_contract_corpus.sh` and only quote `.claim_guard.compiled_all_claim_text` when the guard is true.
- Update `docs/agent/REPO_MAP.md` with `make agent-map` when repo entrypoints change.
- Push coherent slices to a PR instead of holding large local diffs.
- Assume GitHub Actions are disabled or manual-only. If you need a remote workflow run, trigger it deliberately with `workflow_dispatch`; do not expect automatic CI on push or PR.
- Keep the PR in ready-for-review state; do not switch to draft unless a human explicitly asks for it.
- After each meaningful push, run the review loop: check CodeRabbit and Qodo, fix relevant findings, and only merge after a 3-minute quiet window with no new useful bot feedback.

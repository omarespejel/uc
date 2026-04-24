# uc Agent Guide

## Start Here
- Read `.codex/START_HERE.md` before making changes.
- Treat `AGENTS.md`, `.codex/START_HERE.md`, and `docs/agent/*.md` as the checked-in source of truth for agents and review bots.
- Run `make bootstrap && make doctor` before the first edit in a fresh clone.
- Run `make agent-validate` before pushing changes that touch docs, bot config, scripts, or repo structure.
- GitHub Actions are not the default validation path in this repo. Run local gates first and treat GitHub workflows as manual-only escape hatches.
- Never rely on automatic GitHub CI for routine PR validation here. Keep CodeRabbit and Qodo on PRs, but run tests and benchmarks locally.
- Do not use `git push --no-verify` in normal repo work. The checked-in pre-push hook is part of the required validation contract.

## PR-First Rule
- Do all non-trivial work in a fresh branch and open a normal ready-for-review PR early; do not use draft PRs because AI review bots do not fully engage on drafts.
- Do not accumulate substantial local-only changes without a PR review surface.
- After pushing a coherent slice, start the review loop immediately: wait for CodeRabbit and Qodo, fix all relevant feedback, and push follow-up commits.
- Merge only after all actionable human and AI feedback is addressed and the PR has been quiet for at least 3 minutes with no new useful bot comments.
- If more work is needed after a merge, start a new branch and a new PR; do not keep stacking unrelated work into a merged PR.

## Working Model
- Always prefer a fresh clone or worktree for new PR work. Do not edit in a dirty checkout.
- Keep changes scoped. Do not fold unrelated cleanup into performance or review-fix PRs.
- For perf-sensitive work, preserve determinism first, then optimize.
- If a change affects native compile, cache restore, daemon behavior, benchmark harnesses, or artifact format, add or update regression tests in `crates/uc-cli/src/main_tests.rs` or `crates/uc-cli/tests/`.

## Commands
- Bootstrap hooks: `make bootstrap` or `make install-hooks`
- Fast repo check: `make doctor && make agent-validate`
- Local push gate: `make local-ci`
- Format: `cargo fmt --all`
- Fast Rust validation: `make validate-fast`
- Native-focused validation: `make validate-native`
- Validate Cairo 2.14 helper compatibility: `make validate-helper-lane`
- Strict smoke benchmark: `make benchmark-strict-smoke`
- Strict research benchmark: `make benchmark-strict-research`
- Build deployed-contract source index: `benchmarks/scripts/build_deployed_contract_source_index.sh --inventory /abs/path/to/source-inventory.json --out /abs/path/to/pinned-deployed-contract-source-index.json`
- Generate deployed-contract corpus: `benchmarks/scripts/generate_deployed_contract_corpus.sh --source-index /abs/path/to/source-index.json --out /abs/path/to/generated-corpus.json`
- Run deployed-contract corpus evidence: `benchmarks/scripts/run_deployed_contract_corpus.sh --corpus /abs/path/to/generated-corpus.json`

## High-Risk Areas
- `crates/uc-cli/src/main.rs`: native compile session state, cache restore, daemon, build execution.
- `crates/uc-cli/src/fingerprint.rs`: semantic file hashing and fingerprint reuse.
- `third_party/cairo-lang-filesystem/`: keyed file invalidation patch surface.
- `benchmarks/scripts/`: measurement methodology; keep sample counts, pinning, and lane conditions explicit.

## Review Expectations
- Prioritize bugs, invalidation mistakes, silent fallback paths, daemon safety, artifact drift, and benchmark bias.
- Prefer actionable findings over style nits.
- If CI or bot feedback is stale, re-check the exact thread or check run before acting on it.
- Keep PRs reviewable by bots at all times: open normal PRs, not drafts, unless a human explicitly asks for a draft and accepts the loss of bot review.
- Merge only after the PR is quiet for at least 3 minutes with no new useful AI bot feedback and all actionable comments are addressed or explicitly rejected.

## Performance Rules
- Do not claim speedups without stating the exact lane and conditions.
- Keep pinned-host benchmark settings strict by default: offline, explicit daemon mode, CPU pinning when supported, and stable sample counts.
- Do not loosen gate thresholds or sample counts to “make green”.
- Do not re-enable automatic GitHub benchmark workflows to compensate for missing local discipline. Fix the local validation lane instead.

## Native Debugging
- Use `UC_PHASE_TIMING=1` for phase telemetry.
- Use `RUST_LOG=uc=debug` for detailed trace output.
- When debugging hard native stalls, prefer `--engine uc --daemon-mode off --offline` first to remove daemon noise.
- Build older native lanes with `./scripts/build_native_toolchain_helper.sh --lane 2.14`, then export the printed `UC_NATIVE_TOOLCHAIN_2_14_BIN` value.
- Probe helper-lane readiness before measuring a repo with `./scripts/doctor.sh --uc-bin /abs/path/to/uc --manifest-path /abs/path/to/Scarb.toml`.
- For deployed-contract launch claims, build the source index from a reviewed source inventory with `benchmarks/scripts/build_deployed_contract_source_index.sh`, generate the run corpus with `benchmarks/scripts/generate_deployed_contract_corpus.sh`, then use `benchmarks/scripts/run_deployed_contract_corpus.sh`; only quote generated claim text when the artifact's `claim_guard` marks it safe.

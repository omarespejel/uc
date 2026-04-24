# uc Agent Guide

## Start Here
- Read `.codex/START_HERE.md` before making changes.
- Treat `AGENTS.md`, `.codex/START_HERE.md`, and `docs/agent/*.md` as the checked-in source of truth for agents and review bots.
- Run `make doctor` before the first edit in a fresh clone.
- Run `make agent-validate` before pushing changes that touch docs, bot config, scripts, or repo structure.

## PR-First Rule
- Do all non-trivial work in a fresh branch and open a PR early; do not accumulate substantial local-only changes without a PR review surface.
- After pushing a coherent slice, start the review loop immediately: wait for CodeRabbit and Qodo, fix all relevant feedback, and push follow-up commits.
- Merge only after all actionable human and AI feedback is addressed and the PR has been quiet for at least 3 minutes with no new useful bot comments.
- If more work is needed after a merge, start a new branch and a new PR; do not keep stacking unrelated work into a merged PR.

## Working Model
- Always prefer a fresh clone or worktree for new PR work. Do not edit in a dirty checkout.
- Keep changes scoped. Do not fold unrelated cleanup into performance or review-fix PRs.
- For perf-sensitive work, preserve determinism first, then optimize.
- If a change affects native compile, cache restore, daemon behavior, benchmark harnesses, or artifact format, add or update regression tests in `crates/uc-cli/src/main_tests.rs` or `crates/uc-cli/tests/`.

## Commands
- Fast repo check: `make doctor && make agent-validate`
- Format: `cargo fmt --all`
- Fast Rust validation: `make validate-fast`
- Native-focused validation: `make validate-native`
- Strict smoke benchmark: `make benchmark-strict-smoke`
- Strict research benchmark: `make benchmark-strict-research`

## High-Risk Areas
- `crates/uc-cli/src/main.rs`: native compile session state, cache restore, daemon, build execution.
- `crates/uc-cli/src/fingerprint.rs`: semantic file hashing and fingerprint reuse.
- `third_party/cairo-lang-filesystem/`: keyed file invalidation patch surface.
- `benchmarks/scripts/`: measurement methodology; keep sample counts, pinning, and lane conditions explicit.

## Review Expectations
- Prioritize bugs, invalidation mistakes, silent fallback paths, daemon safety, artifact drift, and benchmark bias.
- Prefer actionable findings over style nits.
- If CI or bot feedback is stale, re-check the exact thread or check run before acting on it.
- Merge only after the PR is quiet for at least 3 minutes with no new useful AI bot feedback and all actionable comments are addressed or explicitly rejected.

## Performance Rules
- Do not claim speedups without stating the exact lane and conditions.
- Keep pinned-host benchmark settings strict by default: offline, explicit daemon mode, CPU pinning when supported, and stable sample counts.
- Do not loosen gate thresholds or sample counts to “make green”.

## Native Debugging
- Use `UC_PHASE_TIMING=1` for phase telemetry.
- Use `RUST_LOG=uc=debug` for detailed trace output.
- When debugging hard native stalls, prefer `--engine uc --daemon-mode off --offline` first to remove daemon noise.

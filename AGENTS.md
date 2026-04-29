# uc Agent Guide

## Mission
`uc` is an agent-first Cairo project control plane.

Treat it as:
- a first-party project model,
- a lockfile-first resolver and fetcher,
- a toolchain manager,
- a compiler/build orchestrator,
- and a deterministic machine-readable execution surface.

Do not treat it as a human-only CLI wrapper around Scarb.

## Start Here
- Read `.codex/START_HERE.md` before making changes.
- Treat `AGENTS.md`, `.codex/START_HERE.md`, and `docs/agent/*.md` as the checked-in source of truth for agents and review bots.
- Run `make bootstrap && make doctor` before the first edit in a fresh clone.
- Run `make agent-validate` before pushing changes that touch docs, bot config, scripts, or repo structure.
- GitHub Actions are not the default validation path in this repo. Run local gates first and treat GitHub workflows as manual-only escape hatches.
- Never rely on automatic GitHub CI for routine PR validation here. Keep CodeRabbit and Qodo on PRs, but run tests and benchmarks locally.
- Do not use `git push --no-verify` in normal repo work. The checked-in pre-push hook is part of the required validation contract.

## Read First
1. `.codex/START_HERE.md`
2. `docs/PRODUCT_CHARTER.md`
3. `docs/ARCHITECTURE_BLUEPRINT.md`
4. `docs/COMMAND_SURFACE.md`
5. `docs/ROADMAP.md`
6. `docs/PROJECT_MODEL_STRATEGY.md`
7. `docs/research/AGENT_FIRST_PACKAGE_MANAGER_RESEARCH_2026-04-28.md`

## PR-First Rule
- Do all non-trivial work in a fresh branch and open a normal ready-for-review PR early; do not use draft PRs because AI review bots do not fully engage on drafts.
- Do not accumulate substantial local-only changes without a PR review surface.
- After pushing a coherent slice, start the review loop immediately: wait for CodeRabbit and Qodo, fix all relevant feedback, and push follow-up commits.
- Merge only after all actionable human and AI feedback is addressed and the PR has been quiet for at least 6 minutes with no new useful bot comments.
- If more work is needed after a merge, start a new branch and a new PR; do not keep stacking unrelated work into a merged PR.

## Current Priorities
1. Stable JSON and schema versioning for agent-facing commands.
2. Explicit project inspection, support detection, resolve, fetch, toolchain, and build planning phases.
3. Lockfile-first and offline-capable package/source behavior.
4. Explicit fallback classification. Fallback is compatibility behavior, not native success.
5. Native-first acceleration on modern Cairo lanes. Older lanes are compatibility work unless explicitly promoted.
6. Benchmark claims only from truly native-supported workloads under strict same-window reruns.

## Working Model
- Always prefer a fresh clone or worktree for new PR work. Do not edit in a dirty checkout.
- Keep changes scoped. Do not fold unrelated cleanup into performance or review-fix PRs.
- For perf-sensitive work, preserve determinism first, then optimize.
- If a change affects native compile, cache restore, daemon behavior, benchmark harnesses, artifact format, project inspect, support-native, resolve, fetch, or toolchain surfaces, add or update regression tests in `crates/uc-cli/src/main_tests.rs` or `crates/uc-cli/tests/`.
- No silent fallback.
- No silent network access in locked/offline lanes.
- No silent toolchain download.
- No silent lockfile mutation.
- Machine-readable output is the source of truth; human-readable output is a rendering.
- If the active checkout is dirty in unexpected ways, use an isolated worktree before editing.

## Commands
- Bootstrap hooks: `make bootstrap` or `make install-hooks`
- Fast repo check: `make doctor && make agent-validate`
- Local push gate: `make local-ci`
- Format: `cargo fmt --all`
- Fast Rust validation: `make validate-fast`
- Native-focused validation: `make validate-native`
- Validate Cairo 2.14 helper compatibility: `make validate-helper-lane`
- Refresh repo map: `make agent-map`
- Read-only project inspection: `uc project inspect --manifest-path /abs/path/to/Scarb.toml --format json`
- Agent support decision: `uc support native --manifest-path /abs/path/to/Scarb.toml --format json`
- Locked resolve report: `uc resolve --locked --manifest-path /abs/path/to/Scarb.toml --format json`
- Explicit toolchain ensure: `uc toolchain ensure --manifest-path /abs/path/to/Scarb.toml --format json`
- Dry-run safe remediation: `uc agent safe-action build-helper-lane --lane 2.14`
- Record replayable build failure: `uc build --engine uc --daemon-mode off --manifest-path /abs/path/to/Scarb.toml --record-failure /abs/path/to/uc-failure.json`
- Replay failure bundle: `uc replay /abs/path/to/uc-failure.json`
- Read-only MCP catalog: `uc mcp serve`
- Strict smoke benchmark: `make benchmark-strict-smoke`
- Strict research benchmark: `make benchmark-strict-research`
- Build deployed-contract source index: `benchmarks/scripts/build_deployed_contract_source_index.sh --inventory /abs/path/to/source-inventory.json --out /abs/path/to/pinned-deployed-contract-source-index.json`
- Generate deployed-contract corpus: `benchmarks/scripts/generate_deployed_contract_corpus.sh --source-index /abs/path/to/source-index.json --out /abs/path/to/generated-corpus.json`
- Run deployed-contract corpus evidence: `benchmarks/scripts/run_deployed_contract_corpus.sh --corpus /abs/path/to/generated-corpus.json`
- Summarize corpus opportunities: `benchmarks/scripts/summarize_corpus_opportunities.py --benchmark-json /abs/path/to/benchmark.json --out-json /abs/path/to/opportunities.json --out-md /abs/path/to/opportunities.md`

## Package-Manager Direction
When working on package management, do not reduce the task to "download packages".
The required surface is:
- project model,
- lockfile semantics,
- resolution,
- source fetch,
- source store/cache,
- offline/frozen behavior,
- toolchain acquisition,
- integrity/provenance checks,
- and remediation-grade diagnostics.

## High-Risk Areas
- `crates/uc-cli/src/main.rs`: build path, daemon, native compile session, persisted state, and command dispatch.
- `crates/uc-cli/src/fingerprint.rs`: semantic hashing and fingerprint cache.
- `crates/uc-cli/src/main_tests.rs`: regression-heavy unit coverage.
- `third_party/cairo-lang-filesystem/`: keyed file invalidation patch surface.
- `benchmarks/scripts/`: harnesses and gates.
- `.coderabbit.yaml`, `.pr_agent.toml`, `best_practices.md`, `pr_compliance_checklist.yaml`: PR bot behavior.

## Review Expectations
- Prioritize bugs, invalidation mistakes, silent fallback paths, daemon safety, artifact drift, benchmark bias, and wrong machine-readable state transitions.
- Prefer actionable findings over style nits.
- If CI or bot feedback is stale, re-check the exact thread or check run before acting on it.
- Keep PRs reviewable by bots at all times: open normal PRs, not drafts, unless a human explicitly asks for a draft and accepts the loss of bot review.
- Merge only after the PR is quiet for at least 6 minutes with no new useful AI bot feedback and all actionable comments are addressed or explicitly rejected.

## Performance Rules
- Do not claim speedups without stating the exact lane and conditions.
- Keep pinned-host benchmark settings strict by default: offline, explicit daemon mode, CPU pinning when supported, and stable sample counts.
- Do not loosen gate thresholds or sample counts to "make green".
- Do not re-enable automatic GitHub benchmark workflows to compensate for missing local discipline. Fix the local validation lane instead.

## Native Debugging
- Use `UC_PHASE_TIMING=1` for phase telemetry.
- Use `RUST_LOG=uc=debug` for detailed trace output.
- When debugging hard native stalls, prefer `--engine uc --daemon-mode off --offline` first to remove daemon noise.
- Build older native lanes with `./scripts/build_native_toolchain_helper.sh --lane 2.14`, then export the printed `UC_NATIVE_TOOLCHAIN_2_14_BIN` value.
- Helper-lane patch experiments are configured by lane metadata `patch-dir`; patches must live under `toolchains/cairo-2.14/patches/*.patch`, are applied only in the staging tree, and can read registry sources from `UC_HELPER_CARGO_REGISTRY_SRC` when the default Cargo registry cache is not suitable.
- For Cairo `2.14` frontend hot-path diagnosis, run the helper with `UC_CAIRO214_SIZE_TRACE=/abs/path/to/trace.tsv`; the trace records bounded size-estimation and dummy-Sierra TSV counter samples and is for local diagnostics only.
- Probe helper-lane readiness before measuring a repo with `./scripts/doctor.sh --uc-bin /abs/path/to/uc --manifest-path /abs/path/to/Scarb.toml`.
- For deployed-contract launch claims, build the source index from a reviewed source inventory with `benchmarks/scripts/build_deployed_contract_source_index.sh`, generate the run corpus with `benchmarks/scripts/generate_deployed_contract_corpus.sh`, then use `benchmarks/scripts/run_deployed_contract_corpus.sh`; only quote generated claim text when the artifact's `claim_guard` marks it safe.
- After corpus or real-repo benchmark runs, use `benchmarks/scripts/summarize_corpus_opportunities.py` to turn support gaps, fallback use, unstable lanes, diagnostics gaps, and phase hotspots into UCO-coded follow-up work.

## Agent Contract
Prefer commands and reports that make each phase separately inspectable:
- `project inspect`
- `support native`
- `resolve`
- `fetch`
- `toolchain ensure`
- `build --plan-only`
- `build`
- `explain`

Every new agent-facing operation should answer:
- what happened,
- why,
- retryable or not,
- expected vs found,
- fallback used or not,
- replay command,
- artifact/log path,
- schema version.

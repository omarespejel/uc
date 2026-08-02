# uc Agent Guide

## Mission

`uc` is an agent-first Cairo project control plane.

Treat it as:

- a first-party project model,
- a lockfile-first resolver and fetcher,
- a toolchain manager,
- a compiler/build orchestrator,
- and a deterministic machine-readable execution surface.

## Start Here

- Read `.codex/START_HERE.md` before making changes.
- Treat `AGENTS.md`, `.codex/START_HERE.md`, and `docs/agent/*.md` as the checked-in source of truth for agents and review bots.
- Run `make bootstrap && make doctor` before the first edit in a fresh clone.
- Run `make agent-validate` before pushing changes that touch docs, bot config, scripts, or repo structure.
- GitHub Actions are not the default validation path in this repo. Run local gates first.
- Do not use `git push --no-verify` in normal repo work.

## Read First

1. `.codex/START_HERE.md`
2. `docs/agent/PR_LIFECYCLE.md` — how to take a change to `main`; read before your first commit
3. `README.md`
4. `docs/agent/README.md`
5. `docs/agent/AGENT_FIRST_COMPILER.md`
6. `docs/agent/AGENT_QUICKSTART.md`
7. `docs/agent/AGENT_DIAGNOSTICS.md`
8. `docs/NATIVE_TOOLCHAIN_HELPERS.md`

## PR-First Rule

Full procedure: `docs/agent/PR_LIFECYCLE.md`. The short version:

- Do non-trivial work in a fresh branch or clean worktree. Never commit to `main`.
- Gate locally before opening the PR, and read the gate's own exit code — not the exit
  code of a `grep`/`tail` at the end of a pipeline. Redirect to a file and check `$?`.
- Open a normal ready-for-review PR early; avoid draft PRs unless explicitly requested.
- After pushing a coherent slice, start the review loop immediately.
- Read bot feedback as JSON with `make pr-feedback-open`, not from the web UI.
- Triage **every** bot finding: verify it against the code before acting. If real, fix it
  minimally and add a regression test. If invalid, decline it explicitly with verified
  reasoning in a reply — never silently ignore one. Bots are adversarial reviewers to be
  checked, not authorities to obey.
- Re-run the gate on the fixed head; the previous green run does not carry over.
- Merge only after all actionable feedback is addressed, `counts.unresolved_bot_threads` and
  `counts.failing_checks` are both 0, and the PR has been quiet for at least 6 minutes.

## Current Priorities

1. Stable JSON and schema versioning for agent-facing commands.
2. Explicit project inspection, support detection, resolve, fetch, toolchain, and build planning phases.
3. Lockfile-first and offline-capable source behavior.
4. Explicit fallback classification. Fallback is compatibility behavior, not native success.
5. Native acceleration on supported modern Cairo lanes.
6. Benchmark claims only from truly native-supported workloads under strict same-window reruns.

## Working Model

- Always prefer a fresh worktree for new PR work.
- Keep changes scoped.
- Preserve correctness and determinism before optimizing.
- If a change affects native compile, cache restore, daemon behavior, benchmark harnesses, artifact format, project inspection, support reports, resolve, fetch, or toolchain surfaces, add or update regression tests.
- No silent fallback.
- No silent network access in locked/offline lanes.
- No silent toolchain download.
- No silent lockfile mutation.
- Machine-readable output is the source of truth.

## Commands

- Bootstrap hooks: `make bootstrap` or `make install-hooks`
- Fast repo check: `make doctor && make agent-validate`
- Local push gate: `make local-ci`
- Review-bot feedback as JSON: `make pr-feedback` (or `make pr-feedback-open`, `make pr-feedback PR=<n>`)
- Format: `cargo fmt --all`
- Fast Rust validation: `make validate-fast`
- Native-focused validation: `make validate-native`
- Validate helper compatibility: `make validate-helper-lane`
- Refresh repo map: `make agent-map`
- Read-only project inspection: `uc project inspect --manifest-path /abs/path/to/project.toml --format json`
- Agent support decision: `uc support native --manifest-path /abs/path/to/project.toml --format json`
- Locked resolve report: `uc resolve --locked --manifest-path /abs/path/to/project.toml --format json`
- Locked fetch report: `uc fetch --locked --manifest-path /abs/path/to/project.toml --format json`
- Source-store inventory: `uc cache status --format json`
- Source-store prune: `uc cache prune --format json`
- Explicit toolchain ensure: `uc toolchain ensure --manifest-path /abs/path/to/project.toml --format json`
- Dry-run safe remediation: `uc agent safe-action build-helper-lane --lane 2.14`
- Record replayable build failure: `uc build --engine uc --daemon-mode off --manifest-path /abs/path/to/project.toml --record-failure /abs/path/to/uc-failure.json`
- Replay failure bundle: `uc replay /abs/path/to/uc-failure.json`
- Read-only MCP catalog: `uc mcp serve`
- Strict smoke benchmark: `make benchmark-strict-smoke`
- Strict research benchmark: `make benchmark-strict-research`

## Package-Manager Direction

Do not reduce package management to downloading files. The required surface is:

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

## Agent Contract

Every new agent-facing operation should answer:

- what happened,
- why,
- retryable or not,
- expected vs found,
- fallback used or not,
- replay command,
- artifact/log path,
- schema version.

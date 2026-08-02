# uc best practices

## Rust changes
- Favor deterministic behavior over cleverness.
- In non-test code, do not add `unwrap()` or `expect()` unless the panic is explicitly part of a fail-fast invariant.
- Recover from mutex poisoning (`unwrap_or_else(|p| p.into_inner())`) instead of propagating a panic across a lock.
- Native compile, cache, daemon, and benchmark code should log enough context to debug production failures without rerunning under a debugger.

## Native compile and cache changes
- If a persisted-state format changes, keep a migration or legacy-read path unless there is a deliberate breaking decision documented in the PR.
- Invalidation logic must be conservative under uncertainty. A false miss is acceptable; a false hit is not.
- Bounds and caps (depth, file count, size, timeout) must fail loudly and disable caching for that build. A cap that silently drops inputs from a fingerprint is a false-hit bug.
- Fingerprint inputs must cover every path that can change compiled output, including sources outside the workspace root reached through path dependencies.
- New restore fast paths must prove correctness with targeted regression tests.

## Agent surface changes
- Machine-readable output is the contract. In `--json` mode every exit path emits exactly one JSON document on stdout, including failures.
- Schema changes are additive by default. Removing, renaming, or retyping a field requires a `schema_version` bump and a note naming affected consumers.
- Exit codes carry meaning; see `docs/agent/AGENT_DIAGNOSTICS.md`. Do not collapse a retryable infrastructure failure and a genuine correctness failure into the same code.
- Every degradation is recorded. Scarb fallback, daemon-unavailable, cache-disabled, and offline downgrades must surface a diagnostic, not just a quieter result.
- Diagnostics should answer: what happened, why, retryable or not, expected vs found, fallback used, replay command, schema version.

## Verification and artifacts
- Build receipts and class hashes are correctness surfaces, not telemetry. Treat a wrong `class_hash` like a wrong artifact.
- Artifact bytes must not depend on whether a build was cold, warm, daemon-backed, or fallback-backed. Any setting that changes emitted bytes belongs in the fingerprint and the receipt.
- Never mutate a user's manifest or sources as part of producing something to verify. If an input is unsuitable, fail loudly and say why.

## Benchmark changes
- Keep lane conditions explicit: daemon mode, offline/online, CPU pinning, sample counts, and gate file.
- Do not reduce samples or loosen thresholds to hide noise.
- Do not change benchmark fixtures or baselines without documenting why.
- A performance claim must name the harness, lane, sample count, and comparison build, and must come from a same-window run. Runs where uc fell back to scarb are not native wins.

## Docs and commands
- If a command surface or repo bootstrap step changes, update `AGENTS.md`, `.codex/START_HERE.md`, and `docs/agent/REPO_MAP.md` in the same PR.
- Local validation remains the fast path. Keep the repo-managed hook and `scripts/local_ci_gate.sh` aligned with the documented commands whenever workflows, tests, or benchmark lanes change.
- CI covers correctness lanes only (see `docs/agent/CI_POLICY.md`). Benchmarks stay local and pinned.

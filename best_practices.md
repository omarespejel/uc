# uc best practices

## Rust changes
- Favor deterministic behavior over cleverness.
- In non-test code, do not add `unwrap()` or `expect()` unless the panic is explicitly part of a fail-fast invariant.
- Native compile, cache, daemon, and benchmark code should log enough context to debug production failures without rerunning under a debugger.

## Native compile and cache changes
- If a persisted-state format changes, keep a migration or legacy-read path unless there is a deliberate breaking decision documented in the PR.
- Invalidation logic must be conservative under uncertainty. A false miss is acceptable; a false hit is not.
- New restore fast paths must prove correctness with targeted regression tests.

## Benchmark changes
- Keep lane conditions explicit: daemon mode, offline/online, CPU pinning, sample counts, and gate file.
- Do not reduce samples or loosen thresholds to hide noise.
- Do not change benchmark fixtures or baselines without documenting why.

## Docs and commands
- If a command surface or repo bootstrap step changes, update `AGENTS.md`, `.codex/START_HERE.md`, and `docs/agent/REPO_MAP.md` in the same PR.

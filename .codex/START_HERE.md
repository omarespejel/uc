# uc Start Here

## Product Statement
`uc` is an agent-first Cairo project control plane with package-management, toolchain, and compiler/build responsibilities.

## The Direction
The product is no longer just "make build faster".
It must let agents:
1. inspect a Cairo project,
2. determine native support before build,
3. resolve and fetch dependencies in a lockfile-first way,
4. ensure the right toolchain/helper lane exists,
5. plan and execute builds with explicit fallback behavior,
6. consume stable structured results without scraping terminal prose.

## 5-Minute Bootstrap
1. `make bootstrap`
2. `make doctor`
3. `make agent-validate`
4. Read `docs/agent/README.md`
5. Read `docs/agent/REPO_MAP.md`
6. Read the subsystem doc you are changing:
   - product: `docs/PRODUCT_CHARTER.md`
   - architecture: `docs/ARCHITECTURE_BLUEPRINT.md`
   - command surface: `docs/COMMAND_SURFACE.md`
   - roadmap: `docs/ROADMAP.md`
   - project model: `docs/PROJECT_MODEL_STRATEGY.md`
   - benchmarks: `docs/BENCHMARK_PLAN.md`, `benchmarks/README.md`
   - agent direction: `docs/agent/AGENT_FIRST_COMPILER.md`
   - package-manager research: `docs/research/AGENT_FIRST_PACKAGE_MANAGER_RESEARCH_2026-04-28.md`
7. If the task is larger than a trivial one-line fix, create or reuse a scoped branch and plan to open a ready-for-review PR before broadening the change.

## Immediate Priorities
1. Keep agent-visible behavior explicit and versioned.
2. Build first-party project-model, resolve, fetch, and toolchain surfaces.
3. Preserve correctness and fallback classification.
4. Keep acceleration work focused on native-supported modern Cairo lanes.

## What Not To Do
- Do not count fallback as native support.
- Do not make launch claims from unsupported or fallback-backed cases.
- Do not add implicit network, toolchain, or lockfile behavior to locked flows.
- Do not optimize terminal UX ahead of machine-readable correctness.
- Do not keep substantial local-only changes without a PR review surface.

## Common Commands
- Install repo hooks: `make install-hooks`
- Local push gate: `make local-ci`
- Format: `cargo fmt --all`
- Fast validation: `make validate-fast`
- Native validation: `make validate-native`
- Helper-lane validation: `make validate-helper-lane`
- Refresh repo map: `make agent-map`
- Read-only project inspection: `uc project inspect --manifest-path /abs/path/to/Scarb.toml --format json`
- Agent support decision: `uc support native --manifest-path /abs/path/to/Scarb.toml --format json`
- Locked resolve report: `uc resolve --locked --manifest-path /abs/path/to/Scarb.toml --format json`
- Dry-run safe remediation: `uc agent safe-action build-helper-lane --lane 2.14`
- Replayable build failure capture: `uc build --engine uc --daemon-mode off --manifest-path /abs/path/to/Scarb.toml --record-failure /abs/path/to/uc-failure.json`
- Failure replay: `uc replay /abs/path/to/uc-failure.json`
- Read-only MCP catalog: `uc mcp serve`
- Strict smoke benchmark: `make benchmark-strict-smoke`
- Strict research benchmark: `make benchmark-strict-research`

## Documents
- Product: `docs/PRODUCT_CHARTER.md`
- Architecture: `docs/ARCHITECTURE_BLUEPRINT.md`
- Command surface: `docs/COMMAND_SURFACE.md`
- Roadmap: `docs/ROADMAP.md`
- Project model: `docs/PROJECT_MODEL_STRATEGY.md`
- Agent docs: `docs/agent/README.md`
- Research: `docs/research/AGENT_FIRST_PACKAGE_MANAGER_RESEARCH_2026-04-28.md`
- ADR: `docs/adr/ADR-003-agent-first-control-plane.md`

## Edit Discipline
If you change the product direction, update all of:
- `docs/PRODUCT_CHARTER.md`
- `docs/ARCHITECTURE_BLUEPRINT.md`
- `docs/COMMAND_SURFACE.md`
- `docs/ROADMAP.md`
- `docs/PROJECT_MODEL_STRATEGY.md`
- `docs/agent/README.md`
- `AGENTS.md`
- this file

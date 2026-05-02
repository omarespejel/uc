# uc Start Here

## Product Statement

`uc` is an agent-first Cairo project control plane with package-management, toolchain, and compiler/build responsibilities.

## Direction

The product is not only "make build faster". It should let agents:

1. inspect a Cairo project,
2. determine native support before build,
3. resolve and fetch dependencies in a lockfile-first way,
4. ensure the right toolchain/helper lane exists,
5. plan and execute builds with explicit fallback behavior,
6. consume stable structured results without scraping terminal prose.

## Bootstrap

1. `make bootstrap`
2. `make doctor`
3. `make agent-validate`
4. Read `README.md`
5. Read `docs/agent/README.md`
6. Read the subsystem doc you are changing.

## Immediate Priorities

1. Keep agent-visible behavior explicit and versioned.
2. Build first-party project-model, resolve, fetch, and toolchain surfaces.
3. Preserve correctness and fallback classification.
4. Keep acceleration work focused on native-supported modern Cairo lanes.

## Common Commands

- Install repo hooks: `make install-hooks`
- Local push gate: `make local-ci`
- Format: `cargo fmt --all`
- Fast validation: `make validate-fast`
- Native validation: `make validate-native`
- Helper-lane validation: `make validate-helper-lane`
- Refresh repo map: `make agent-map`
- Read-only project inspection: `uc project inspect --manifest-path /abs/path/to/project.toml --format json`
- Agent support decision: `uc support native --manifest-path /abs/path/to/project.toml --format json`
- Locked resolve report: `uc resolve --locked --manifest-path /abs/path/to/project.toml --format json`
- Locked fetch report: `uc fetch --locked --manifest-path /abs/path/to/project.toml --format json`
- Source-store inventory: `uc cache status --format json`
- Source-store prune: `uc cache prune --format json`
- Explicit toolchain ensure: `uc toolchain ensure --manifest-path /abs/path/to/project.toml --format json`
- Dry-run safe remediation: `uc agent safe-action build-helper-lane --lane 2.14`
- Replayable build failure capture: `uc build --engine uc --daemon-mode off --manifest-path /abs/path/to/project.toml --record-failure /abs/path/to/uc-failure.json`
- Failure replay: `uc replay /abs/path/to/uc-failure.json`
- Read-only MCP catalog: `uc mcp serve`
- Strict smoke benchmark: `make benchmark-strict-smoke`
- Strict research benchmark: `make benchmark-strict-research`

## Documents

- Product overview: `README.md`
- Agent docs: `docs/agent/README.md`
- Agent diagnostics: `docs/agent/AGENT_DIAGNOSTICS.md`
- Agent quickstart: `docs/agent/AGENT_QUICKSTART.md`
- Human quickstart: `docs/agent/HUMAN_QUICKSTART.md`
- Helper lanes: `docs/NATIVE_TOOLCHAIN_HELPERS.md`

## Edit Discipline

If you change the product direction, update:

- `README.md`
- `AGENTS.md`
- `.codex/START_HERE.md`
- `docs/agent/README.md`
- the affected agent docs or schemas

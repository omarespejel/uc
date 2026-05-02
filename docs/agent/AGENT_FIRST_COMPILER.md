# Agent-First Compiler

`uc` is designed for agents first and terminal users second.

That changes the architecture. Agents need structured state, explicit plans, replayable failures, and stable schemas. They should not infer behavior from logs or terminal prose.

## Core Flow

```bash
uc project inspect --manifest-path /abs/path/to/project.toml --format json
uc support native --manifest-path /abs/path/to/project.toml --format json
uc resolve --locked --manifest-path /abs/path/to/project.toml --format json
uc fetch --locked --manifest-path /abs/path/to/project.toml --format json
uc toolchain ensure --manifest-path /abs/path/to/project.toml --format json
uc build --engine uc --daemon-mode off --manifest-path /abs/path/to/project.toml --plan-only --json
uc build --engine uc --daemon-mode off --manifest-path /abs/path/to/project.toml --json
```

## Design Consequences

- Project state must be inspectable before build.
- Native support must be classified before expensive execution.
- Dependency and source readiness must be explicit.
- Toolchain/helper state must be explicit.
- Build planning must report side effects before execution.
- Fallback must be reported as fallback, not native success.
- Diagnostics must include stable codes, retryability, expected/found state, and next commands.

## Speed Direction

The performance strategy is to avoid unnecessary work:

- keep reusable state alive where safe,
- avoid recomputing frontend work on unchanged inputs,
- use content-addressed cache keys,
- split planning from execution,
- benchmark only supported workloads under same-window conditions.

Speed claims require the exact benchmark lane, host, sample counts, daemon mode, and source artifact.

# Agent Quickstart

Agents should use `uc` as a phased control plane, not as one opaque build command.

## 1. Inspect

```bash
uc project inspect --manifest-path /abs/path/to/project.toml --format json
```

Use this to collect workspace root, package metadata, targets, lockfile state, source origins, and read-only mutation status.

## 2. Classify Native Support

```bash
uc support native --manifest-path /abs/path/to/project.toml --format json
```

Use `decision_status` for routing:

- `native_supported`: native path can proceed.
- `native_unsupported`: do not retry native until the reported gap changes.
- `fallback_likely`: compatibility backend may be used if policy allows it.
- `build_blocked`: fix manifest, source, lockfile, or toolchain state first.

## 3. Resolve And Fetch

```bash
uc resolve --locked --manifest-path /abs/path/to/project.toml --format json
uc fetch --locked --manifest-path /abs/path/to/project.toml --format json
```

Locked mode must not mutate dependency intent. Offline-readiness must be reported explicitly.

## 4. Ensure Toolchain

```bash
uc toolchain ensure --manifest-path /abs/path/to/project.toml --format json
```

If a helper lane is missing, follow only the reported safe action. Do not build or download a toolchain silently.

## 5. Plan Before Build

```bash
uc build --engine uc --daemon-mode off --manifest-path /abs/path/to/project.toml --plan-only --json
```

Read the planned backend, fallback policy, daemon mode, side effects, session key, invalidation key, and diagnostics before execution.

## 6. Execute

```bash
uc build --engine uc --daemon-mode off --manifest-path /abs/path/to/project.toml --json
```

A successful fallback-backed build is not native success. Preserve `fallback_used`, diagnostics, artifact paths, and replay commands.

## 7. Capture Failures

```bash
uc build --engine uc --daemon-mode off \
  --manifest-path /abs/path/to/project.toml \
  --record-failure /abs/path/to/uc-failure.json

uc replay /abs/path/to/uc-failure.json
```

Failure bundles should be treated as the durable reproduction handle.

## 8. Benchmark Discipline

Use strict same-window runs for claims. A benchmark result is not quotable unless it states lane, host, sample counts, daemon mode, selected cases, fallback state, and claim guard.

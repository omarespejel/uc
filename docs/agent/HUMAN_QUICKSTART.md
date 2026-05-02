# Human Quickstart

Bootstrap the repo:

```bash
make bootstrap
make doctor
```

Inspect a project:

```bash
uc project inspect --manifest-path /abs/path/to/project.toml --format json | jq
```

Check native support:

```bash
uc support native --manifest-path /abs/path/to/project.toml
uc support native --manifest-path /abs/path/to/project.toml --format json | jq
```

Prepare dependencies and toolchain:

```bash
uc resolve --locked --manifest-path /abs/path/to/project.toml --format json | jq
uc fetch --locked --manifest-path /abs/path/to/project.toml --format json | jq
uc toolchain ensure --manifest-path /abs/path/to/project.toml --format json | jq
```

Plan and build:

```bash
uc build --engine uc --daemon-mode off --manifest-path /abs/path/to/project.toml --plan-only --json | jq
uc build --engine uc --daemon-mode off --manifest-path /abs/path/to/project.toml --json
```

Capture a replayable failure:

```bash
uc build --engine uc --daemon-mode off \
  --manifest-path /abs/path/to/project.toml \
  --record-failure /tmp/uc-failure.json

uc replay /tmp/uc-failure.json
```

Validate before pushing:

```bash
cargo fmt --all
make agent-validate
git diff --check
make local-ci
```

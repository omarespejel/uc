# Agent Quickstart

Agents should prefer structured commands and stop guessing from terminal prose.

## Native Support Probe

```sh
uc support native --manifest-path /abs/path/to/Scarb.toml --format json
```

Read:

- `.supported`
- `.status`
- `.issue_kind`
- `.toolchain.requested_version`
- `.toolchain.requested_major_minor`
- `.toolchain.source`
- `.diagnostics[].code`
- `.diagnostics[].next_commands`
- `.diagnostics[].safe_automated_action`

## Doctor Probe

```sh
./scripts/doctor.sh --uc-bin /abs/path/to/uc --manifest-path /abs/path/to/Scarb.toml
```

If `jq` is missing, install or provide it before interpreting manifest support probes.

## Safe Remediation

If diagnostic `UCN1004` has `safe_automated_action=build_helper_lane`, run the helper builder for the requested lane:

```sh
./scripts/build_native_toolchain_helper.sh --lane 2.14
```

Then export the printed env var and rerun support probing.

## Build Report

```sh
uc build --engine uc --daemon-mode off --manifest-path /abs/path/to/Scarb.toml --json
```

If `.diagnostics[].fallback_used` is true, classify the result as fallback-used even when the command exits successfully.

## Stop Conditions

Stop and ask for human permission before:

- editing Cairo source files
- rewriting dependency ranges
- deleting caches outside the repo workspace
- publishing benchmark numbers from uncommitted artifacts
- merging PRs before the AI review quiet window

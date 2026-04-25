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

If diagnostic `UCN1006` has `safe_automated_action=manual_legacy_adapter_required`, do not run the helper builder for that lane. Report the workload as `native_unsupported`, or use an explicitly reviewed helper binary via the reported `UC_NATIVE_TOOLCHAIN_<major>_<minor>_BIN` env var.

## Build Report

```sh
uc build --engine uc --daemon-mode off --manifest-path /abs/path/to/Scarb.toml --json
```

If `.diagnostics[].fallback_used` is true, classify the result as fallback-used even when the command exits successfully.

## Deployed-Contract Corpus Evidence

```sh
./benchmarks/scripts/build_deployed_contract_source_index.sh \
  --inventory /abs/path/to/source-root/reviewed-deployed-contract-source-inventory.json \
  --out /abs/path/to/source-root/pinned-deployed-contract-source-index.json

./benchmarks/scripts/generate_deployed_contract_corpus.sh \
  --source-index /abs/path/to/source-root/pinned-deployed-contract-source-index.json \
  --out /abs/path/to/generated-deployed-contract-corpus.json

./benchmarks/scripts/run_deployed_contract_corpus.sh \
  --uc-bin /abs/path/to/uc \
  --corpus /abs/path/to/generated-deployed-contract-corpus.json \
  --results-dir benchmarks/results \
  --runs 5 \
  --cold-runs 5
```

Read:

- `.summary.support_matrix`
- `.summary.cairo_version_min`
- `.summary.cairo_version_max`
- `.claim_guard.safe_to_say_compiled_all_deployed_contracts_in_corpus`
- `.claim_guard.compiled_all_claim_text`

Only use `.claim_guard.compiled_all_claim_text` when the guard is true. If the
guard is false, report `.claim_guard.reason` and keep the artifact as support
matrix evidence, not launch copy.

Treat the source inventory as the durable raw evidence input. The source index
and generated corpus JSON are deterministic artifacts and should be regenerated
from the reviewed inventory instead of edited by hand.

## Stop Conditions

Stop and ask for human permission before:

- editing Cairo source files
- rewriting dependency ranges
- deleting caches outside the repo workspace
- publishing benchmark numbers from uncommitted artifacts
- merging PRs before the AI review quiet window

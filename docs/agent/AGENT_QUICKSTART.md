# Agent Quickstart

Agents should prefer structured commands and stop guessing from terminal prose.

## Project Inspect

```sh
uc project inspect --manifest-path /abs/path/to/Scarb.toml --format json
```

Read:

- `.readonly`
- `.mutation_status`
- `.package`
- `.workspace.members`
- `.targets`
- `.dependencies`
- `.lockfile`
- `.toolchain.requested_version`
- `.toolchain.requested_major_minor`
- `.toolchain.native_status`
- `.diagnostics[].code`

Treat the raw report as local evidence. It can include absolute paths and
BLAKE3 hashes for `Scarb.toml` / `Scarb.lock`; do not forward it to telemetry,
public logs, or issue comments unless that sharing is intentional.

## Native Support Probe

```sh
uc support native --manifest-path /abs/path/to/Scarb.toml --format json
```

Read:

- `.supported`
- `.decision_status`
- `.status`
- `.issue_kind`
- `.toolchain.requested_version`
- `.toolchain.requested_major_minor`
- `.toolchain.source`
- `.diagnostics[].code`
- `.diagnostics[].next_commands`
- `.diagnostics[].safe_automated_action`

## Locked Resolve

```sh
uc resolve --locked --manifest-path /abs/path/to/Scarb.toml --format json
```

Read:

- `.status`
- `.mode`
- `.network_intent`
- `.lockfile.present`
- `.lockfile.valid`
- `.lockfile_sync.status`
- `.lockfile_sync.missing_dependencies`
- `.offline_readiness.status`
- `.toolchain.requested_version`
- `.diagnostics[].code`

If `.status == "build_blocked"`, stop before fetch or build and fix the lockfile
or manifest state first. Read `.blocked_reason` directly because the schema
requires it as a nullable field and `uc` uses it as the authoritative blocked
cause. Do not infer locked-resolve safety from diagnostics, lockfile prose, or
terminal output.

## Fetch

```sh
uc fetch --locked --manifest-path /abs/path/to/Scarb.toml --format json
```

Read:

- `.status`
- `.network_intent`
- `.execution_driver`
- `.source_store.entry_count`
- `.source_store.total_bytes`
- `.offline_readiness_before.status`
- `.offline_readiness_after.status`
- `.fetched_entries[].status`
- `.missing_entries[].name`
- `.blocked_reason`
- `.diagnostics[].code`

If `.status == "build_blocked"`, stop before build and fix the blocked fetch
state first. Use `.missing_entries` and `.source_store` directly instead of
guessing from terminal output whether the local dependency graph is actually
hydrated.

## Source Store Inventory

```sh
uc cache status --format json
uc cache prune --format json
```

Read from `uc cache status`:

- `.root`
- `.available`
- `.writable`
- `.entry_count`
- `.total_bytes`
- `.max_bytes`
- `.invalid_entry_count`

Read from `uc cache prune`:

- `.entry_count_before`
- `.entry_count_after`
- `.total_bytes_before`
- `.total_bytes_after`
- `.removed_entry_count`
- `.removed_bytes`

Use `uc cache status` before forcing offline fetch/build flows. Use
`uc cache prune` only as an explicit maintenance step; do not assume pruning is
safe in the middle of a benchmark or active build investigation.

## Toolchain Ensure

```sh
uc toolchain ensure --manifest-path /abs/path/to/Scarb.toml --format json
```

Read:

- `.status`
- `.execution_driver`
- `.toolchain.requested_version`
- `.toolchain.requested_major_minor`
- `.toolchain.source`
- `.ensured_now`
- `.mutation_status`
- `.artifact_path`
- `.blocked_reason`
- `.subprocess_commands`
- `.diagnostics[].code`

If `.status == "build_blocked"`, stop before build and fix the toolchain issue
first. If `.ensured_now == true`, persist `.artifact_path` as local evidence for
the ensured helper lane and rerun `uc support native` only if you need the full
probe payload.

## Build Plan

```sh
uc build --engine uc --daemon-mode off --manifest-path /abs/path/to/Scarb.toml --plan-only --json
```

Read:

- `.status`
- `.planned_compile_backend`
- `.execution_driver`
- `.fallback_allowed`
- `.daemon_planned`
- `.daemon_autostart_allowed`
- `.session_key`
- `.strict_invalidation_key`
- `.side_effects[].kind`
- `.native_support.decision_status`
- `.diagnostics[].code`

If `.status == "build_blocked"`, stop before execution and fix the reported input or
toolchain problem. Do not infer build readiness from terminal text alone.

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

If diagnostic `UCN1006` has `safe_automated_action=manual_legacy_adapter_required`, do not run the helper builder for that lane. Report the workload as `native_unsupported`, or use an explicitly reviewed helper binary via the reported `UC_NATIVE_TOOLCHAIN_<major>_<minor>_BIN` env var. After setting a reviewed helper env var, rerun `UC_NATIVE_TOOLCHAIN_<major>_<minor>_BIN=/abs/path/to/helper-uc uc support native --manifest-path /abs/path/to/Scarb.toml --format json` before final classification; the rerun validates the helper and lets the lane proceed past `manual_legacy_adapter_required`.

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
- `.summary.source_kind_counts`
- `.summary.cairo_version_min`
- `.summary.cairo_version_max`
- `.claim_guard.safe_to_say_compiled_all_deployed_contracts_in_corpus`
- `.claim_guard.compiled_all_claim_text`

Only use `.claim_guard.compiled_all_claim_text` when the guard is true. If the
guard is false, report `.claim_guard.reason` and keep the artifact as support
matrix evidence, not launch copy.

For launch copy that says "deployed contracts", every row must be
`source_kind=deployed_contract`. `declared_class` rows are valid support
evidence for class source compatibility, but they intentionally block the
deployed-contract claim guard.

Legacy rows that omit `source_kind` are normalized as `deployed_contract` so old
corpus artifacts remain readable. New reviewed inventories should set
`source_kind` explicitly.

Treat the source inventory as the durable raw evidence input. The source index
and generated corpus JSON are deterministic artifacts and should be regenerated
from the reviewed inventory instead of edited by hand.

After a corpus run, generate the experiment backlog before optimizing or
drafting launch copy:

```sh
./benchmarks/scripts/summarize_corpus_opportunities.py \
  --benchmark-json /abs/path/to/deployed-contract-corpus-bench.json \
  --out-json /abs/path/to/corpus-opportunities.json \
  --out-md /abs/path/to/corpus-opportunities.md
```

Agents should treat `UCO1001`, `UCO1002`, `UCO1003`, and `UCO2001` as blockers
before speed work. `UCO3001` marks `native_frontend_compile_ms` as the likely
next acceleration target. `UCO5001` means the diagnostic payload is not yet
safe enough for automated remediation.

For strict launch-speed evidence from a mixed real-repo artifact:

```sh
./benchmarks/scripts/run_strict_supported_set_benchmarks.sh \
  --benchmark-json /abs/path/to/real-repo-bench.json \
  --results-dir benchmarks/results \
  --runs 12 \
  --cold-runs 12
```

Read:

- `.selection.source_benchmark_json`
- `.selection.selected_case_count`
- `.claim_guard.safe_to_say_native_supported_speed_claim`
- `.claim_guard.native_supported_speed_claim_text`

Only use `.claim_guard.native_supported_speed_claim_text` when the guard is
true. If the guard is false, report `.claim_guard.reason` and keep the rerun
artifact as diagnostic support evidence only.

## Stop Conditions

Stop and ask for human permission before:

- editing Cairo source files
- rewriting dependency ranges
- deleting caches outside the repo workspace
- publishing benchmark numbers from uncommitted artifacts
- merging PRs before the AI review quiet window

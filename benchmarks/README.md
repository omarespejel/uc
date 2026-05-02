# Benchmarks

This directory contains benchmark harnesses, gates, corpora helpers, fixtures, and result output locations for `uc`.

## Reproducibility Rules

- Use same-window runs for comparisons.
- State lane, host, sample counts, daemon mode, selected cases, and fallback state.
- Record exact pins for the `uc` binary, helper/toolchain lane, and case manifest revisions. Use commit hashes or immutable tags.
- Run claimable builds through the offline harness path. `run_real_repo_benchmarks.sh` and the strict supported-set wrapper invoke offline builds internally; do not switch to online mode for claim artifacts.
- Record the path and commit for any benchmark gate file used to approve a claimable run.
- Do not quote unsupported or fallback-backed cases as native speed evidence.
- Keep generated result files under `benchmarks/results/` unless the artifact is intentionally external.
- Treat sample corpora as diagnostic evidence, not public claims.

## Execution Policy

Benchmark execution is local/manual by default. The old remote benchmark workflow was removed with the dated baseline artifacts because benchmark results must be produced from an explicit local lane with known host conditions. Use the commands below and commit only the artifacts that are intentionally part of a reviewed evidence set.

## Common Commands

```bash
make benchmark-strict-smoke
make benchmark-strict-research

UC_BIN=/abs/path/to/uc-<git-sha> \
UC_NATIVE_TOOLCHAIN_2_14_BIN=/abs/path/to/helper-2.14-<git-sha> \
benchmarks/scripts/run_real_repo_benchmarks.sh \
  --uc-bin /abs/path/to/uc-<git-sha> \
  --cases-file /abs/path/to/cases-pinned-at-<git-sha>.tsv \
  --results-dir benchmarks/results \
  --runs 12 \
  --cold-runs 12 \
  --stamp same-window-<git-sha>

UC_BIN=/abs/path/to/uc-<git-sha> \
UC_NATIVE_TOOLCHAIN_2_14_BIN=/abs/path/to/helper-2.14-<git-sha> \
benchmarks/scripts/run_strict_supported_set_benchmarks.sh \
  --benchmark-json /abs/path/to/real-repo-bench-<git-sha>.json \
  --uc-bin /abs/path/to/uc-<git-sha> \
  --results-dir benchmarks/results \
  --runs 12 \
  --cold-runs 12 \
  --stamp strict-supported-<git-sha>

benchmarks/scripts/gate_benchmark_summary.sh \
  --summary /abs/path/to/stability-summary-<git-sha>.json \
  --config /abs/path/to/benchmark-gate-config-at-<git-sha>.json

benchmarks/scripts/summarize_corpus_opportunities.py \
  --benchmark-json /abs/path/to/benchmark.json \
  --out-json /abs/path/to/opportunities.json \
  --out-md /abs/path/to/opportunities.md
```

## Deployed-Contract Corpus Helpers

```bash
benchmarks/scripts/build_deployed_contract_source_index.sh \
  --inventory /abs/path/to/source-inventory.json \
  --out /abs/path/to/source-index.json

benchmarks/scripts/generate_deployed_contract_corpus.sh \
  --source-index /abs/path/to/source-index.json \
  --out /abs/path/to/generated-corpus.json

benchmarks/scripts/run_deployed_contract_corpus.sh \
  --corpus /abs/path/to/generated-corpus.json
```

Only quote generated claim text when the artifact's `claim_guard` marks it safe.

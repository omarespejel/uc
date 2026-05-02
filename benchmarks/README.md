# Benchmarks

This directory contains benchmark harnesses, gates, corpora helpers, fixtures, and result output locations for `uc`.

## Rules

- Use same-window runs for comparisons.
- State lane, host, sample counts, daemon mode, selected cases, and fallback state.
- Do not quote unsupported or fallback-backed cases as native speed evidence.
- Keep generated result files under `benchmarks/results/` unless the artifact is intentionally external.
- Treat sample corpora as diagnostic evidence, not public claims.

## Common Commands

```bash
make benchmark-strict-smoke
make benchmark-strict-research

benchmarks/scripts/run_real_repo_benchmarks.sh \
  --case /abs/path/to/repo-a/project.toml repo-a \
  --case /abs/path/to/repo-b/project.toml repo-b

benchmarks/scripts/run_strict_supported_set_benchmarks.sh \
  --benchmark-json /abs/path/to/real-repo-bench.json \
  --results-dir benchmarks/results \
  --runs 12 \
  --cold-runs 12

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

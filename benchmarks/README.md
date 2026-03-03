# Benchmarks

Benchmark harness and baseline artifacts for Scarb vs `uc` performance and parity tracking.

## Folders
- `scenarios.md`: scenario definitions.
- `scripts/`: benchmark and comparator runners.
- `gates/`: performance gate rule sets.
- `fixtures/`: local fixture projects for CI smoke runs.
- `results/`: transient benchmark and comparator outputs.
- `baselines/`: committed baseline snapshots.

## Run Baseline Matrix
```bash
./benchmarks/scripts/run_local_benchmarks.sh --matrix research --tool scarb --workspace-root /path/to/compiler-starknet
./benchmarks/scripts/run_local_benchmarks.sh --matrix research --tool uc --workspace-root /path/to/compiler-starknet
```

## Run Dual-Run Comparator
```bash
WORKSPACE_ROOT=/path/to/compiler-starknet ./benchmarks/scripts/run_dual_run_comparator.sh
```

## Run Stability Cycles + Gate
```bash
./benchmarks/scripts/run_stability_benchmarks.sh \
  --matrix research \
  --workspace-root /path/to/compiler-starknet \
  --runs 10 \
  --cold-runs 5 \
  --cycles 5 \
  --gate-config benchmarks/gates/perf-gate-research.json
```

## Compare Two Benchmark Runs
```bash
./benchmarks/scripts/compare_benchmark_results.sh --baseline <scarb.json> --candidate <uc.json> --out <delta.md>
```

## Modes
- `research` (default): uses external sibling repos (`scarb/examples/*`) under `--workspace-root` or `WORKSPACE_ROOT`.
- default fallback for `research` is the parent directory of this repo; if manifests are not found, pass `--workspace-root` explicitly.
- `smoke`: uses fixture project in this repo for CI portability.

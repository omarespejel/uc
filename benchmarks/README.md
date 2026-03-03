# Benchmarks

Benchmark harness and baseline artifacts for Scarb vs `uc` performance and parity tracking.

## Folders
- `scenarios.md`: scenario definitions.
- `scripts/`: benchmark and comparator runners.
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

## Compare Two Benchmark Runs
```bash
./benchmarks/scripts/compare_benchmark_results.sh --baseline <scarb.json> --candidate <uc.json> --out <delta.md>
```

## Modes
- `research` (default): uses external sibling repos (`scarb/examples/*`) under `--workspace-root` or `WORKSPACE_ROOT`.
- default fallback for `research` is the parent directory of this repo; if manifests are not found, pass `--workspace-root` explicitly.
- `smoke`: uses fixture project in this repo for CI portability.

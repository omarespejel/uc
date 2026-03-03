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
./benchmarks/scripts/run_local_benchmarks.sh --matrix research --tool scarb
./benchmarks/scripts/run_local_benchmarks.sh --matrix research --tool uc
```

## Run Dual-Run Comparator
```bash
./benchmarks/scripts/run_dual_run_comparator.sh
```

## Compare Two Benchmark Runs
```bash
./benchmarks/scripts/compare_benchmark_results.sh --baseline <scarb.json> --candidate <uc.json> --out <delta.md>
```

## Modes
- `research` (default): uses local repos in `/Users/espejelomar/StarkNet/compiler-starknet`.
- `smoke`: uses fixture project in this repo for CI portability.

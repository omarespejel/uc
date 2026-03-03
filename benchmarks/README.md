# Benchmarks

Benchmark harness and baseline artifacts for Scarb vs `uc` performance tracking.

## Folders
- `scenarios.md`: scenario definitions.
- `scripts/`: benchmark runner scripts.
- `fixtures/`: local fixture projects for CI smoke runs.
- `results/`: transient benchmark outputs.
- `baselines/`: committed baseline snapshots.

## Run
```bash
./benchmarks/scripts/run_local_benchmarks.sh
```

## Modes
- `research` (default): uses local repos in `/Users/espejelomar/StarkNet/compiler-starknet`.
- `smoke`: uses fixture project in this repo for CI portability.

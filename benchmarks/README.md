# Benchmarks

Benchmark harness and baseline artifacts for Scarb vs `uc` performance and parity tracking.

`run_local_benchmarks.sh` runs on `bash` and supports CPU affinity backends (`taskset` or `hwloc-bind`) plus optional pinning flags for lower variance (`--cpu-set`, `--nice-level`, `--strict-pinning`).
By default, UC benchmarks use the release binary (`target/release/uc`) to reflect production startup/runtime behavior. Override with `UC_BUILD_PROFILE=debug` or an explicit `UC_BIN=/abs/path/to/uc`.
Build scenarios are measured in offline mode by default for stability (`--build-online` to opt out). The default UC benchmark mode is `--uc-daemon-mode off` for lower run-to-run jitter (`require` is still supported). `uc` runs emit per-sample phase telemetry (`phase_samples` + `phase_stats`) in benchmark JSON.

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
  --runs 12 \
  --cold-runs 12 \
  --uc-daemon-mode off \
  --cycles 5 \
  --cpu-set 0 \
  --strict-pinning \
  --nice-level 5 \
  --warm-settle-seconds 2.2 \
  --gate-config benchmarks/gates/perf-gate-research.json \
  --lock-baseline
```

`run_stability_benchmarks.sh` enforces a locked lane (`--runs 12`, `--cold-runs 12`, pinned CPU + strict pinning). Use `--allow-unpinned` only when affinity APIs are unavailable on the host.

## Compare Two Benchmark Runs
```bash
./benchmarks/scripts/compare_benchmark_results.sh --baseline <scarb.json> --candidate <uc.json> --out <delta.md>
```

## Modes
- `research` (default): uses external sibling repos (`scarb/examples/*`) under `--workspace-root` or `WORKSPACE_ROOT`.
- default fallback for `research` is the parent directory of this repo; if manifests are not found, pass `--workspace-root` explicitly.
- `smoke`: uses fixture project in this repo for CI portability.

# Benchmarks

Benchmark harness and baseline artifacts for Scarb vs `uc` performance and parity tracking.

`run_local_benchmarks.sh` runs on `bash` and supports CPU affinity backends (`taskset` or `hwloc-bind`) plus optional pinning flags for lower variance (`--cpu-set`, `--nice-level`, `--strict-pinning`).
It also supports host-noise preflight controls (`--host-preflight off|warn|require`, `--allow-noisy-host`) to catch background language/proc-macro servers that can skew samples.
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

`run_stability_benchmarks.sh` enforces a locked lane (`--runs 12`, `--cold-runs 12`, pinned CPU + strict pinning) and always evaluates the matrix gate config (`benchmarks/gates/perf-gate-<matrix>.json` by default).
The current stability gate requires warm-noop median p95 improvement of at least +20% and blocks catastrophic single-cycle warm-noop outliers (< -20%).
Use `--allow-unpinned` only when affinity APIs are unavailable on the host.
Stability runs default to `--host-preflight require` and fail fast if noisy host processes are detected; use `--allow-noisy-host` only for debugging or environments where process isolation is not possible.

## Compare Two Benchmark Runs
```bash
./benchmarks/scripts/compare_benchmark_results.sh --baseline <scarb.json> --candidate <uc.json> --out <delta.md>
```

## CI Native Gates
```bash
./benchmarks/scripts/run_native_only_gate.sh \
  --uc-bin ./target/release/uc \
  --results-dir benchmarks/results \
  --case benchmarks/fixtures/scarb_smoke/Scarb.toml smoke 0

./benchmarks/scripts/run_native_real_repo_smoke.sh \
  --uc-bin ./target/release/uc \
  --results-dir benchmarks/results \
  --strict-case /abs/path/to/project/Scarb.toml sample  \
  --backend-case /abs/path/to/project/Scarb.toml sample-fallback scarb,uc-native
```

These checked-in scripts back the GitHub Actions native-only and real-repo smoke gates.
Keep CI gate logic in scripts instead of workflow heredocs so it stays testable and reviewable.

## Fast Iteration Loop (Developer Lane)
```bash
./benchmarks/scripts/run_fast_perf_check.sh
# or:
make perf-fast

# target only one hotspot scenario for faster iteration:
./benchmarks/scripts/run_fast_perf_check.sh --scenario build.warm_edit_semantic
```

This lane is optimized for iteration speed (default `--runs 4 --cold-runs 4`, smoke matrix) and applies lightweight p95 gates for early signal. Use it while developing and keep the full stability lane (`12/12`, paired cycles, pinned host) as the final merge/nightly proof.

## Modes
- `research` (default): uses external sibling repos (`scarb/examples/*`) under `--workspace-root` or `WORKSPACE_ROOT`.
- default fallback for `research` is the parent directory of this repo; if manifests are not found, pass `--workspace-root` explicitly.
- `smoke`: uses fixture project in this repo for CI portability.

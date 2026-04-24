# Benchmark Plan

## Objective
Measure and prove that `uc` outperforms Scarb on real workflows while maintaining correctness.

## Matrix
1. `build.cold`: remove workspace build artifacts and build.
2. `build.warm_noop`: build again without changes.
3. `build.warm_edit`: edit one Cairo file and rebuild.
4. `metadata.online_cold`: metadata with empty global cache.
5. `metadata.offline_warm`: metadata with warm cache and `--offline`.

## Workloads
- `scarb/examples/hello_world`
- `scarb/examples/workspaces`
- `scarb/examples/dependencies` (metadata path)
- Optional heavy profile: `stwo_cairo_verifier` with required features.

## Output Artifacts
- JSON per run under `benchmarks/results/`.
- Markdown summary per run under `benchmarks/results/`.
- Reviewed baseline snapshots under `benchmarks/baselines/`.
- Deployed-contract corpus plan/benchmark artifacts under `benchmarks/results/`,
  generated from `benchmarks/corpora/deployed-contract-corpus.schema.json`.

## Execution Policy
- Benchmarks are local-first. Reproduction must work from checked-in scripts plus pinned manifest paths; do not require GitHub Actions or hosted CI to verify the numbers.
- Before comparing before/after performance claims, rerun both sides in the same binary/toolchain window on the same machine.
- Deployed-contract claims must go through `run_deployed_contract_corpus.sh`; do
  not manually aggregate real-repo benchmark output into launch copy.

## Baseline Rule
Before changing `uc` engine behavior, rerun baseline against current Scarb and snapshot results.

## Eligibility Rule
Do not mix native-ineligible manifests into `uc` vs Scarb speedup claims.
Benchmark reports must separate:
- `native_supported`: auto-build stayed on native and strict native benchmarks were executed
- `fallback_used`: auto-build downgraded to Scarb, along with the structured fallback reason
- `native_unsupported`: native support probe rejected the repo before timed execution
- `build_failed`: auto-build failed before backend classification completed

Examples of native-ineligible reasons that should be reported explicitly:
- exact `cairo-version` mismatch against the native compiler
- legacy package editions (`2023_01`, `2023_10`, `2023_11`) without an exact `[package].cairo-version`
- missing or invalid `UC_NATIVE_TOOLCHAIN_<major>_<minor>_BIN` helper lanes

## Stability Rule
Do not rely on aggregate medians alone.
Real-repo benchmark reports must surface repo-level instability when a lane shows a materially noisy sample window, including outlier-heavy cases where:
- `p95 / p50 >= 1.20`, or
- `max / p50 >= 1.25`

These warnings are not automatic failures, but they must be called out explicitly before drawing “faster/slower” conclusions from the aggregate summary.

## Comparator Rule
Every build-path engine change must run dual-run comparison (`scarb-direct` vs `uc build`) and record:
- artifact mismatch count,
- diagnostics similarity,
- candidate vs baseline elapsed time.

## Gate Thresholds
- Gate A (Performance): warm rebuild p95 >= 40% faster than Scarb baseline.
- Gate B (Correctness): zero artifact hash mismatches and diagnostics parity >= 99.5%.

## Execution
```bash
./benchmarks/scripts/run_local_benchmarks.sh --matrix research --tool scarb
./benchmarks/scripts/run_local_benchmarks.sh --matrix research --tool uc
./benchmarks/scripts/run_dual_run_comparator.sh

# Real repo support-matrix and strict-native benchmark sweep
./scripts/build_native_toolchain_helper.sh --lane 2.14
UC_NATIVE_TOOLCHAIN_2_14_BIN=/abs/path/to/uc-cairo214-helper \
./benchmarks/scripts/run_real_repo_benchmarks.sh \
  --uc-bin /abs/path/to/uc \
  --case /abs/path/to/repo/Scarb.toml repo-tag

# Pinned deployed-contract corpus support matrix and guarded claim artifact
./benchmarks/scripts/run_deployed_contract_corpus.sh \
  --uc-bin /abs/path/to/uc \
  --corpus /abs/path/to/pinned-deployed-contract-corpus.json \
  --results-dir benchmarks/results \
  --runs 5 \
  --cold-runs 5
```

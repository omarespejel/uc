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

## Benchmark Real Repos With Eligibility Split
```bash
./benchmarks/scripts/run_real_repo_benchmarks.sh \
  --uc-bin ./target/release/uc \
  --results-dir benchmarks/results \
  --runs 5 \
  --cold-runs 5 \
  --case /abs/path/to/repo-a/Scarb.toml repo-a \
  --case /abs/path/to/repo-b/Scarb.toml repo-b

# For larger case sets, avoid argv fanout and use a tab-separated cases file:
#   /abs/path/to/repo-a/Scarb.toml<TAB>repo-a
./benchmarks/scripts/run_real_repo_benchmarks.sh \
  --uc-bin ./target/release/uc \
  --results-dir benchmarks/results \
  --runs 5 \
  --cold-runs 5 \
  --cases-file /abs/path/to/manifest-tag-cases.tsv
```

This local-only harness uses `uc support native --format json` to classify each
manifest before any timed run. Native-eligible cases are benchmarked against
Scarb on `build.cold` and `build.warm_noop`; native-ineligible cases are listed
separately with the exact unsupported reason and are excluded from speedup claims.
If a native-eligible case fails during `scarb` or `uc` execution, the harness
records that build failure in a separate section with exit code and log path
instead of aborting the whole benchmark run.

## Build Deployed-Contract Source Index From Inventory

```bash
# Shape-only sample: validates reviewed source inventory and writes a source index.
./benchmarks/scripts/build_deployed_contract_source_index.sh \
  --inventory benchmarks/corpora/deployed-contract-source-inventory.example.json \
  --out benchmarks/corpora/generated-deployed-contract-source-index.sample.json
```

The inventory is the durable raw evidence layer for deployed-contract claims. It
records the chain/snapshot/block selection, deduplication policy, license
policy, source availability, and every reviewed source record before
deduplication. The builder enforces the constraints mirrored in
`benchmarks/corpora/deployed-contract-source-inventory.schema.json`, validates
that every manifest path points at a local `Scarb.toml`, deduplicates by the
configured key, and writes source-index JSON that conforms to
`benchmarks/corpora/deployed-contract-source-index.schema.json`.

Write the generated source index next to the reviewed inventory. Source paths
are intentionally confined under the inventory/source-index directory so a
reviewed artifact cannot escape into unrelated local files via `..` traversal.

Do not hand-author launch source indexes. Keep the inventory as the reviewed
input, generate the source index from it, and commit or immutably archive both
the exact inventory and generated source index used for any public claim.

## Generate Deployed-Contract Corpus From Source Index

```bash
# Shape-only sample: validates generated source-index input and writes a generated corpus.
./benchmarks/scripts/generate_deployed_contract_corpus.sh \
  --source-index benchmarks/corpora/generated-deployed-contract-source-index.sample.json \
  --out benchmarks/results/generated-deployed-contract-corpus.sample.json
```

The source index is the deterministic selection artifact for deployed-contract
evidence. It records chain/snapshot/block selection, deduplication counts,
license policy, source availability, and the local `Scarb.toml` chosen for each
contract or deduped class. The generator enforces the constraints mirrored in
`benchmarks/corpora/deployed-contract-source-index.schema.json`, resolves all
relative manifest paths to absolute paths, and writes corpus JSON that conforms
to `benchmarks/corpora/deployed-contract-corpus.schema.json` for the benchmark
runner.

Do not hand-author launch corpus JSON. Keep the source inventory and generated
source index as the reviewed inputs, generate the corpus from them, and commit
or immutably archive the exact artifacts used for any public claim.

## Run Deployed-Contract Corpus Evidence

```bash
# Validate and normalize a corpus without running builds.
./benchmarks/scripts/run_deployed_contract_corpus.sh \
  --corpus benchmarks/corpora/deployed-contract-corpus.example.json \
  --results-dir benchmarks/results \
  --plan-only

# Run the corpus through the real-repo support matrix and benchmark harness.
./benchmarks/scripts/run_deployed_contract_corpus.sh \
  --uc-bin ./target/release/uc \
  --corpus /abs/path/to/pinned-deployed-contract-corpus.json \
  --results-dir benchmarks/results \
  --runs 5 \
  --cold-runs 5
```

The corpus wrapper is the launch-evidence path for deployed-contract claims. It
validates `benchmarks/corpora/deployed-contract-corpus.schema.json`, resolves
each item to a local `Scarb.toml`, runs `run_real_repo_benchmarks.sh`, and emits
a combined JSON/Markdown artifact with:

- the pinned chain/snapshot/block-range selection,
- a per-item `source_kind`:
  - `deployed_contract` rows must include `contract_address`
  - `declared_class` rows benchmark class source evidence but do not count as
    deployed-contract coverage
  - legacy rows without `source_kind` are read as `deployed_contract` for
    backward compatibility, but new reviewed inventories should set it
    explicitly
- deduplication and source/license policy metadata,
- Cairo version min/max across the corpus,
- a support matrix for `native_supported`, `native_unsupported`,
  `fallback_used`, and `build_failed`,
- guarded claim text via explicit guards:
  - `.claim_guard.compiled_all_claim_text` only when
    `.claim_guard.safe_to_say_compiled_all_deployed_contracts_in_corpus=true`;
    this requires `coverage=complete_deployed_contracts`, every item classified
    as `source_kind=deployed_contract` and `native_supported`, no fallback, no
    unsupported rows, no build failures, no failed native benchmark cases, and
    `deduplication.key=none` with `deduplication.input_count == item_count`.
    Only use `.claim_guard.compiled_all_claim_text` when that equality holds.
  - `.claim_guard.selected_units_claim_text` only when
    `.claim_guard.safe_to_say_compiled_all_selected_deployed_units_in_corpus=true`;
    this is the safe wording for corpora deduplicated by `class_hash` or
    `source_package`
  - `.claim_guard.native_supported_claim_text` only when
    `.claim_guard.safe_to_say_all_items_native_supported=true`

Do not turn sample-corpus output into launch copy. A `coverage=sample` corpus is
valid for smoke testing the artifact path, but the generated claim guard will
mark the “compiled every deployed contract” sentence unsafe.

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

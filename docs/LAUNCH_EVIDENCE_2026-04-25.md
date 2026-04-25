# Launch Evidence Checkpoint 2026-04-25

This checkpoint records the current local evidence for the agent-first Cairo compiler launch path. It is support evidence and diagnostic benchmark output for a reviewed two-item sample, not a deployed-contract universe claim and not a launch-grade speed claim.

## Evidence Bundle

- Evidence root: `$EVIDENCE_ROOT`
- Source inventory: `$EVIDENCE_ROOT/reviewed-source-inventory.json`
- Source index: `$EVIDENCE_ROOT/pinned-source-index.json`
- Generated corpus: `$EVIDENCE_ROOT/generated-corpus.json`
- Corpus benchmark JSON: `$EVIDENCE_ROOT/results-current-final/deployed-contract-corpus-bench-20260425-201850.json`
- Corpus benchmark Markdown: `$EVIDENCE_ROOT/results-current-final/deployed-contract-corpus-bench-20260425-201850.md`
- Real-repo benchmark JSON: `$EVIDENCE_ROOT/results-current-final/real-repo-bench-20260425-201850.json`
- Real-repo benchmark Markdown: `$EVIDENCE_ROOT/results-current-final/real-repo-bench-20260425-201850.md`

## Generation Commands

```bash
export UC_REPO_ROOT="$(pwd)"
export EVIDENCE_ROOT="/path/to/external/uc-launch-evidence-20260425-pr48"
export UC_NATIVE_TOOLCHAIN_2_14_BIN="$HOME/.uc/toolchain-helpers/uc-cairo214-helper/bin/uc"

cd "$UC_REPO_ROOT"
cargo build -p uc-cli --release

./benchmarks/scripts/build_deployed_contract_source_index.sh \
  --inventory "$EVIDENCE_ROOT/reviewed-source-inventory.json" \
  --out "$EVIDENCE_ROOT/pinned-source-index.json"

./benchmarks/scripts/generate_deployed_contract_corpus.sh \
  --source-index "$EVIDENCE_ROOT/pinned-source-index.json" \
  --out "$EVIDENCE_ROOT/generated-corpus.json"

./benchmarks/scripts/run_deployed_contract_corpus.sh \
  --uc-bin "$UC_REPO_ROOT/target/release/uc" \
  --corpus "$EVIDENCE_ROOT/generated-corpus.json" \
  --results-dir "$EVIDENCE_ROOT/results-current-final" \
  --runs 3 \
  --cold-runs 3 \
  --warm-settle-seconds 2.2
```

## Corpus Scope

- Corpus id: `starknet-reviewed-local-source-sample-2026-04-25`
- Chain label: `starknet-reviewed-local-source-sample`
- Coverage: `sample`
- Snapshot id: `local-reviewed-source-sample-2026-04-25-pr48`
- Items: `2`
- Source kinds: `deployed_contract=1`, `declared_class=1`
- Cairo versions: `2.14.0` through `2.14.0`
- Deduplication: `class_hash`, `input_count=2`, `deduped_count=2`

## Support Matrix

| Item | Source kind | Cairo version | Classification | Backend | Fallback used |
|---|---|---:|---|---|---|
| `monero_atomic_lock_sepolia` | `deployed_contract` | `2.14.0` | `native_supported` | `uc_native_external_helper` | `false` |
| `braavos_account_v1_2_0_class` | `declared_class` | `2.14.0` | `native_supported` | `uc_native_external_helper` | `false` |

Summary: `native_supported=2`, `fallback_used=0`, `native_unsupported=0`, `build_failed=0`, `unstable_lane_count=1`.

## Same-Window Benchmark Output

This run is not launch-grade speed evidence because `unstable_lane_count=1`. Keep it as diagnostic support evidence only until a same-window rerun passes the stability guard.

Lane and conditions:

- Benchmark lane: `run_deployed_contract_corpus.sh` wrapping `run_real_repo_benchmarks.sh`.
- Benchmark stages: `build.cold` and `build.warm_noop`.
- Native lane: Cairo `2.14.0` external helper via `UC_NATIVE_TOOLCHAIN_2_14_BIN`.
- uc backend: `uc_native_external_helper`.
- Baseline: `scarb 2.14.0 (682b29e13 2025-11-25)`.
- Host: Apple M3 Pro, `Mac15,7`, macOS `26.4.1` build `25E253`, `aarch64-apple-darwin`.
- Runs: warm runs `3`, cold runs `3`, warm settle `2.2s`.
- CPU pinning: not set on this macOS host.
- uc binary SHA-256: `3b4557707f3681ba3ebc34012adb2fe5a861ccb73f79f8e7b4948a8e4ef0bed0`.
- Cairo 2.14 helper SHA-256: `b184a564ebd1b9761cc20ef8a11594aba2e4b355b348cfc5875a26d040ea27a3`.

| Item | Cold Scarb p95 (ms) | Cold uc p95 (ms) | Cold speedup | Warm Scarb p95 (ms) | Warm uc p95 (ms) | Warm speedup |
|---|---:|---:|---:|---:|---:|---:|
| `monero_atomic_lock_sepolia` | 10069.652 | 6244.161 | 1.613x | 5939.758 | 36.852 | 161.179x |
| `braavos_account_v1_2_0_class` | 4291.033 | 13619.110 | 0.315x | 15965.993 | 75.151 | 212.452x |

Stability warning:

| Item | Tool | Stage | p50 (ms) | p95 (ms) | max (ms) | p95/p50 | max/p50 |
|---|---|---|---:|---:|---:|---:|---:|
| `braavos_account_v1_2_0_class` | `scarb` | `build.warm_noop` | 4691.155 | 15965.993 | 17218.753 | 3.403 | 3.670 |

## Native Frontend Profile

The build reports include phase telemetry for the native cold classification builds:

| Item | `native_frontend_compile_ms` | `native_casm_ms` | Contracts compiled |
|---|---:|---:|---:|
| `monero_atomic_lock_sepolia` | 5547.938 | 259.500 | 1 |
| `braavos_account_v1_2_0_class` | 2533.680 | 401.005 | 3 |

Monero remains the right next profiling target: most cold `uc` time is still in `native_frontend_compile`.

## Artifact Hashes

```text
1fe7c1a777dc565cb918c4c72bb6aaf63f2a79427871ff1e126e710bd4cbfb42  reviewed-source-inventory.json
4b55caa829353f6b0c5fb56a0f2175db975716774652614f86a0940c6f4b17e3  pinned-source-index.json
2eb32ce9c2d1c57a279388bf3eae1982b74decf1c329c991c741ba7aa9f3d921  generated-corpus.json
3b4557707f3681ba3ebc34012adb2fe5a861ccb73f79f8e7b4948a8e4ef0bed0  target/release/uc
d67bcf16d83a8b5140188f405b42d5e68d3cd4a40d3d6565eeea1d9f234a810a  results-current-final/deployed-contract-corpus-bench-20260425-201850.json
26982507475eb5ba1c544963f106f3ddd890a88f311bce4961408f8c454392ab  results-current-final/real-repo-bench-20260425-201850.json
cec9bfbdbdd2ee8a74f2b622882b9eb04e159a6bfd67e4a3a1811f2ff9e345e3  results-current-final/real-repo-braavos_account_v1_2_0_class-uc-auto-build-report.json
f9b76b840f44efcd89259a4671ad6f62b8a70b0034526bff950feb9faf64624f  results-current-final/real-repo-monero_atomic_lock_sepolia-uc-auto-build-report.json
99aceb3776cd871412972f086596f30095dace9e07871b541f2d560d8c25c2e7  results-current-final/deployed-contract-corpus-bench-20260425-201850.md
aacc93c4e12ecff5e8c2cd7e2e2ce9926dee3fb853471b1102648a520bcdb7fb  results-current-final/real-repo-bench-20260425-201850.md
```

To verify the artifact hashes, set `EVIDENCE_ROOT` to the evidence directory or
run from that directory, copy the lines above into `checksums.txt`, then run:

```bash
cd "$EVIDENCE_ROOT"
shasum -a 256 -c checksums.txt
```

For `target/release/uc`, run `shasum -a 256 "$UC_REPO_ROOT/target/release/uc"`.

## Claim Boundary

Safe to say internally:

> Every item in the pinned `starknet-reviewed-local-source-sample` corpus was native-supported in this run.

Also safe to say internally:

> The current two-item Cairo `2.14.0` sample classifies monero and Braavos as native-supported with no fallback and no build failures.

Not safe to say yet:

> We compiled every deployed contract in a Starknet corpus.

Also not safe to say from this exact artifact:

> The current same-window benchmark artifact supports a launch-grade speedup claim.

Reasons:

- `selection.coverage` is `sample`, not `complete_deployed_contracts`.
- The corpus has one `deployed_contract` row and one `declared_class` row.
- `claim_guard.safe_to_say_compiled_all_deployed_contracts_in_corpus=false`.
- `claim_guard.safe_to_say_compiled_all_selected_deployed_units_in_corpus=false`.
- `summary.unstable_lane_count=1`, so speed numbers are diagnostic only.
- Launch-grade deployed-contract evidence still needs a complete deployed-contract snapshot or an explicitly named bounded complete deployed-contract corpus with immutable hosting.

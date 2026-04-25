# Launch Evidence Checkpoint 2026-04-25

This checkpoint records the current local evidence for the agent-first Cairo compiler launch path. It is support and benchmark evidence for a reviewed two-item sample, not a deployed-contract universe claim.

## Evidence Bundle

- Evidence root: `$EVIDENCE_ROOT`
- Source inventory: `$EVIDENCE_ROOT/reviewed-source-inventory.json`
- Source index: `$EVIDENCE_ROOT/pinned-source-index.json`
- Generated corpus: `$EVIDENCE_ROOT/generated-corpus.json`
- Corpus benchmark JSON: `$EVIDENCE_ROOT/results/deployed-contract-corpus-bench-20260425-180015.json`
- Corpus benchmark Markdown: `$EVIDENCE_ROOT/results/deployed-contract-corpus-bench-20260425-180015.md`
- Real-repo benchmark JSON: `$EVIDENCE_ROOT/results/real-repo-bench-20260425-180015.json`
- Real-repo benchmark Markdown: `$EVIDENCE_ROOT/results/real-repo-bench-20260425-180015.md`

## Generation Commands

```bash
export UC_REPO_ROOT="$(pwd)"
export EVIDENCE_ROOT="/path/to/external/uc-launch-evidence-20260425"
export UC_NATIVE_TOOLCHAIN_2_14_BIN="$HOME/.uc/toolchain-helpers/uc-cairo214-helper/bin/uc"

cd "$UC_REPO_ROOT"
cargo build -p uc-cli --release
./scripts/build_native_toolchain_helper.sh --lane 2.14 --output "$UC_NATIVE_TOOLCHAIN_2_14_BIN"

./benchmarks/scripts/build_deployed_contract_source_index.sh \
  --inventory "$EVIDENCE_ROOT/reviewed-source-inventory.json" \
  --out "$EVIDENCE_ROOT/pinned-source-index.json"

./benchmarks/scripts/generate_deployed_contract_corpus.sh \
  --source-index "$EVIDENCE_ROOT/pinned-source-index.json" \
  --out "$EVIDENCE_ROOT/generated-corpus.json"

./benchmarks/scripts/run_deployed_contract_corpus.sh \
  --uc-bin "$UC_REPO_ROOT/target/release/uc" \
  --corpus "$EVIDENCE_ROOT/generated-corpus.json" \
  --results-dir "$EVIDENCE_ROOT/results" \
  --runs 3 \
  --cold-runs 3 \
  --warm-settle-seconds 2.2
```

## Support Matrix

| Item | Source kind | Cairo version | Classification | Backend | Fallback used |
|---|---|---:|---|---|---|
| `monero_atomic_lock_sepolia` | `deployed_contract` | `2.14.0` | `native_supported` | `uc_native_external_helper` | `false` |
| `braavos_account_v1_2_0_class` | `declared_class` | `2.14.0` | `native_supported` | `uc_native_external_helper` | `false` |

Summary: `native_supported=2`, `fallback_used=0`, `native_unsupported=0`, `build_failed=0`.

## Same-Window Benchmark Results

Lane and conditions:

- Benchmark lane: `run_deployed_contract_corpus.sh` wrapping `run_real_repo_benchmarks.sh`.
- Benchmark stages: `build.cold` and `build.warm_noop`.
- Native lane: Cairo `2.14.0` external helper via `UC_NATIVE_TOOLCHAIN_2_14_BIN`.
- uc backend: `uc_native_external_helper`.
- Baseline: `scarb 2.14.0 (682b29e13 2025-11-25)`.
- Host: Apple M3 Pro, `Mac15,7`, macOS `26.4.1` build `25E253`, `aarch64-apple-darwin`.
- Runs: warm runs `3`, cold runs `3`, warm settle `2.2s`.
- CPU pinning: not set on this macOS host.
- uc binary SHA-256: `22da63681dfcd6a5429cbcf096638ac08618f71cb2987f3616b59c2b7ce5cb99`.
- Cairo 2.14 helper SHA-256: `b184a564ebd1b9761cc20ef8a11594aba2e4b355b348cfc5875a26d040ea27a3`.

| Item | Cold Scarb p95 (ms) | Cold uc p95 (ms) | Cold speedup | Warm Scarb p95 (ms) | Warm uc p95 (ms) | Warm speedup |
|---|---:|---:|---:|---:|---:|---:|
| `monero_atomic_lock_sepolia` | 14365.929 | 7173.073 | 2.003x | 8027.459 | 33.547 | 239.288x |
| `braavos_account_v1_2_0_class` | 5121.310 | 4225.745 | 1.212x | 5465.849 | 56.151 | 97.343x |

The monero Scarb warm-noop lane was marked unstable by the harness: p50 `6586.095ms`, p95 `8027.459ms`, max `8187.611ms`, p95/p50 ratio `1.219`.

## Native Frontend Profile

The build reports include phase telemetry for the native cold classification builds:

| Item | `native_frontend_compile_ms` | `native_casm_ms` | Contracts compiled |
|---|---:|---:|---:|
| `monero_atomic_lock_sepolia` | 6620.065 | 276.651 | 1 |
| `braavos_account_v1_2_0_class` | 2698.329 | 429.709 | 3 |

Monero remains the right next profiling target: most cold `uc` time is still in `native_frontend_compile`.

## Artifact Hashes

```text
b326e0a9cbcaeeef78073328933b81c133378b66d32c0d6e828d4c8df86bb5b5  deployed-contract-corpus-bench-20260425-180015.json
061159118082bb5ad43db28f54a75d7e2a4b45747223a9adf8c472cb20f8a36b  deployed-contract-corpus-bench-20260425-180015.md
b2a470abe9ed1082e39527848f426ebb476367ef0492524733df533121f0a0cb  real-repo-bench-20260425-180015.json
0ac8179b1a4b80031ba5edb3439f7b9ff59c37c796f401879bf391225d24ebfb  real-repo-bench-20260425-180015.md
9b4f86866dc9d896bc8848d84174632478b28565cdf46d6e85e0a15f8c7c0f42  generated-corpus.json
fbb75e2284a08cc2b6e96d4e7be4abc739af85506354490da66097a9170e9205  pinned-source-index.json
b62d46619e475bb5ca3a6f7d45a551bc93d10fe3cdebfb819823959611479493  reviewed-source-inventory.json
```

To verify the artifact hashes, set `EVIDENCE_ROOT` to the evidence directory or
run from that directory, copy the lines above into `checksums.txt`, then run:

```bash
cd "$EVIDENCE_ROOT"
shasum -a 256 -c checksums.txt
```

## Claim Boundary

Safe to say internally:

> The current two-item Cairo `2.14.0` sample classifies monero and Braavos as native-supported with no fallback and no build failures, and the same-window benchmark artifact shows material warm-noop speedups plus positive cold p95 speedups.

Not safe to say yet:

> We compiled every deployed contract in a Starknet corpus.

Reasons:

- `selection.coverage` is `sample`, not `complete_deployed_contracts`.
- The corpus has one `deployed_contract` row and one `declared_class` row.
- `claim_guard.safe_to_say_compiled_all_deployed_contracts_in_corpus=false`.
- Launch-grade evidence still needs a complete deployed-contract snapshot or an explicitly named bounded corpus with immutable hosting.

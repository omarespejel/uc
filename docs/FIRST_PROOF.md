# First Proof (TPM Gate)

## First Thing We Need To Prove
A sessionized `uc` build path can reduce warm rebuild p95 by at least 40% versus Scarb baseline without correctness regressions.

## Hypothesis H1
For repeated builds in the same workspace:
- `uc` warm `edit -> build` p95 improves by >= 40%.
- Artifact parity stays exact.
- Diagnostics parity is >= 99.5%.

## Why This First
- It validates the core product value quickly.
- It derisks architecture before broad command implementation.
- It creates a hard Go/No-Go gate for platform investment.

## Required Evidence
1. Benchmark report across matrix workloads.
2. Artifact hash comparator report.
3. Diagnostics comparator report.
4. Reliability report proving:
   - 100/100 consecutive `uc build` runs complete without crash.
   - on forced cache read failure, build falls back to fresh build with correct artifacts.
   - process restart preserves correctness gate outcomes (no artifact mismatches, diagnostics >= 99.5%).

## Current Execution Hooks
- Baseline benchmark harness: `benchmarks/scripts/run_local_benchmarks.sh`
- Dual-run comparator harness: `benchmarks/scripts/run_dual_run_comparator.sh`
- CLI compare command: `uc compare-build --manifest-path <Scarb.toml>`

## Current Status (2026-03-03)
- Comparator gate: passing on sampled workloads (0 artifact mismatches, diagnostics similarity 100%).
- Performance signal: smoke stability gate passing with wins on warm no-op and warm-edit (including semantic edits); cold path remains near parity and is still being tuned.
- Hardening in progress: integration coverage for cache hit/miss, corruption recovery, and concurrent access continues to expand.
- Gate A status: passing for smoke fixture lane; research-matrix sign-off still in progress.

## Gate Outcome
- Go: all criteria pass.
- No-Go: criteria miss after one stabilization cycle.

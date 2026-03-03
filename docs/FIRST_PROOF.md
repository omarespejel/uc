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
- It creates a hard Go/No-Go gate for replacement investment.

## Required Evidence
1. Benchmark report across matrix workloads.
2. Artifact hash comparator report.
3. Diagnostics comparator report.
4. Reliability report for fallback and daemon restart behavior.

## Gate Outcome
- Go: all criteria pass.
- No-Go: criteria miss after one stabilization cycle.

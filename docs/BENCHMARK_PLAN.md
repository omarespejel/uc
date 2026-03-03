# Benchmark Plan

## Objective
Prove measurable value and protect against regressions with reproducible benchmarks.

## Workload Matrix
1. Warm no-op rebuild.
2. Warm single-file edit rebuild.
3. Cold build.
4. Profile/feature change rebuild.
5. CI-like clean environment run.

## Reference Repositories
- Large workspace representative.
- Medium workspace representative.
- Proof-heavy representative.

## Outputs
- Raw JSON per run.
- Summary Markdown in `benchmarks/results/`.
- Weekly trend comparison report.

## Tooling
- `hyperfine` for timing stats.
- Structured output JSON.
- Optional system-level stats (`/usr/bin/time -lp`).

## Acceptance Thresholds
- Warm p95 improvement >=40% by Gate A.
- CI cache-hit >=70% by Gate C.
- Flake rate <=5%.


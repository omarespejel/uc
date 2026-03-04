# Comparator Report (2026-03-03)

## Scope
Dual-run comparison between:
- baseline: direct `scarb build`
- candidate: `uc build --engine uc`

## Command
```bash
./benchmarks/scripts/run_dual_run_comparator.sh
```

## Results
- `hello_world`
  - pass: `true`
  - artifact mismatches: `0`
  - diagnostics similarity: `100%`
- `workspaces`
  - pass: `true`
  - artifact mismatches: `0`
  - diagnostics similarity: `100%`

## Notes
- This establishes comparator infrastructure and correctness gating.
- Candidate engine now runs through `uc` cache path and falls back to Scarb on cache miss.
- Comparator gate remains mandatory while native engine behavior is tuned.

## Artifacts
- `benchmarks/baselines/2026-03-03-compare-summary.md`
- `benchmarks/baselines/2026-03-03-compare-hello_world.json`
- `benchmarks/baselines/2026-03-03-compare-workspaces.json`

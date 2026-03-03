# Comparator Report (2026-03-03)

## Scope
Dual-run comparison between:
- baseline: direct `scarb build`
- candidate: `uc build` wrapper path

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
- Current candidate engine is Scarb-backed by design for initial rollout.
- Next step is swapping candidate execution path from Scarb-backed wrapper to native `uc` engine and enforcing the same gate.

## Artifacts
- `benchmarks/baselines/2026-03-03-compare-summary.md`
- `benchmarks/baselines/2026-03-03-compare-hello_world.json`
- `benchmarks/baselines/2026-03-03-compare-workspaces.json`

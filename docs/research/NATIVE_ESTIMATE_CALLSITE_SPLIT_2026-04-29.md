# Native Estimate Callsite Split (2026-04-29)

## Goal

Determine how much of Cairo `2.14` exact size-estimation traffic is driven by inline decisions, instead of treating all `estimate_size` activity as one undifferentiated hotspot.

## Patch Shape

Local helper-lane instrumentation only:

- existing bounded `estimate_size` / `dummy_program_for_size_estimation` trace remained in place
- added one new inline caller event:
  - `estimate_size_callsite:inline_should_inline`
- attempted specialization caller instrumentation too, but dropped it for this pass after the patch landed incorrectly in the helper lane and did not justify more time before the next product slice

The helper patch remained local to the Cairo `2.14` helper lane staging tree.

## Validation

Helper-lane check passed with the inline-only instrumentation patch:

```bash
./scripts/build_native_toolchain_helper.sh --lane 2.14 --check-only
./scripts/build_native_toolchain_helper.sh --lane 2.14
```

Release-helper trace runs were killed with exit code `137`, so the usable instrumentation evidence came from the debug helper binary directly:

- helper: `.uc/toolchain-helper-targets/cairo-2.14/debug/uc`
- flags: `build --engine uc --daemon-mode off --offline`
- env:
  - `UC_NATIVE_DISALLOW_SCARB_FALLBACK=1`
  - `UC_CAIRO214_SIZE_TRACE=/tmp/<trace>.tsv`

For braavos, the first direct debug run cache-hit and wrote no trace. The successful trace run used a fresh temp copy with `target`, `.uc`, and `.scarb` removed.

## Artifacts

Monero debug-helper trace:

- `/tmp/uc-monero-inline-callsite-trace-debug-20260429.tsv`

Braavos debug-helper trace:

- `/tmp/uc-braavos-inline-callsite-trace-debug-20260429-rerun.tsv`

Braavos trace contains some malformed interleaved rows from the cheap append-only tracing mechanism; those rows were skipped during aggregation.

## Result

### Monero

Aggregated event counts:

- `estimate_size`: `2741`
- `dummy_program_for_size_estimation`: `2741`
- `estimate_size_callsite:inline_should_inline`: `1348`

Inline share of exact size-estimation traffic:

- `1348 / 2741 = 0.492`
- about `49.2%`

### Braavos

Aggregated event counts after skipping `69` malformed rows:

- `estimate_size`: `2912`
- `dummy_program_for_size_estimation`: `2900`
- `estimate_size_callsite:inline_should_inline`: `1119`

Inline share of exact size-estimation traffic:

- `1119 / 2912 = 0.384`
- about `38.4%`

## Conclusion

Inline decisions are a major, but not total, contributor to exact size-estimation churn:

- roughly half on monero
- roughly two-fifths on braavos

This is enough to rule out a simplistic story like “all remaining frontend cost is inline-only”.

It also supports the earlier rejection of broad inline heuristics:

- inlining is important enough to matter,
- but not dominant enough to justify shipping unstable global heuristics that regress other workloads.

## Recommendation

The next frontend speed work should target one of these two narrower directions:

1. improve exact size-estimation cost for the repeated hot helper families without changing inline policy broadly, or
2. add safer caller-specific instrumentation for the remaining non-inline share before changing behavior.

For now, the strongest production decision remains:

- keep the helper lane on the baseline trace patch,
- keep the inline fast-path experiment rejected,
- and do not ship a perf PR from this pass.

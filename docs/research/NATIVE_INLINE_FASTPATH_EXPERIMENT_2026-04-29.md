# Native Inline Fast-Path Experiment (2026-04-29)

## Goal

Test whether Cairo `2.14` helper cold-build time can improve by skipping exact CASM size estimation for obviously tiny lowered functions during inlining decisions.

This was motivated by the repeated `estimate_size` / `dummy_program_for_size_estimation` traffic measured on helper-backed native builds for:

- `monero_atomic_swap`
- `braavos_account`
- `glint_contracts`
- `zcash_relay`

## Hypothesis

The hottest repeated size-estimation targets are tiny core helpers (`core::assert`, `core::Felt252PartialEq::eq`, storage reads, array append, byte-array accessors). A cheap lowered-body approximation might be good enough to early-accept these cases and avoid exact dummy Sierra plus CASM generation.

## Patch Shape

Local-only helper-lane experiment in Cairo `2.14`:

- patch file: `toolchains/cairo-2.14/patches/cairo-lang-lowering.patch`
- touched function: `cairo_lang_lowering::inline::should_inline_lowered`
- strategy:
  - compute `ApproxCasmInlineWeight` from `LoweringStage::PostBaseline`
  - return `true` early for very small functions
  - keep the existing exact `db.estimate_size(function_id)` path for the rest

Two thresholds were tested:

1. `APPROX_INLINE_FAST_PATH_THRESHOLD = 24`
2. `APPROX_INLINE_FAST_PATH_THRESHOLD = 12`

This experiment was not kept in the checked-in helper patch after measurement.

## Helper Validation

The experimental lane patch applied cleanly and passed helper-lane validation before measurement:

```bash
./scripts/build_native_toolchain_helper.sh --lane 2.14 --check-only
./scripts/build_native_toolchain_helper.sh --lane 2.14
```

Reference helper was rebuilt from the unmodified lane in a separate clean worktree and copied aside for same-window comparison.

## Same-Window Artifact Set

### Threshold 24

Reference artifact:

- `benchmarks/results/real-repo-bench-inlinefast-reference-20260429.json`

Patched artifact:

- `benchmarks/results/real-repo-bench-inlinefast-patched-20260429.json`

Measured `uc` p95 deltas versus the same-window reference helper:

| Contract | Cold | Warm no-op | Result |
| --- | ---: | ---: | --- |
| `braavos_account` | `1.309x` faster | `0.997x` | cold win, warm neutral |
| `glint_contracts` | `2.083x` faster | `1.795x` faster | strong win |
| `monero_atomic_swap` | `0.899x` | `0.565x` | bad regression |
| `zcash_relay` | `1.101x` faster | `0.989x` | small cold win, warm neutral |

Decision:

- reject threshold `24`
- monero regression is too large to treat this as a real speed improvement

### Threshold 12

Reference artifact:

- `benchmarks/results/real-repo-bench-inlinefast12-reference-20260429.json`

Patched artifact:

- `benchmarks/results/real-repo-bench-inlinefast12-patched-20260429.json`

Measured `uc` p95 deltas versus the same-window reference helper:

| Contract | Cold | Warm no-op | Result |
| --- | ---: | ---: | --- |
| `braavos_account` | `1.045x` faster | `0.987x` | small cold win, warm neutral |
| `glint_contracts` | `1.082x` faster | `0.615x` | warm regression |
| `monero_atomic_swap` | `1.011x` faster | `1.008x` | neutral |
| `zcash_relay` | `1.054x` faster | `0.603x` | warm regression |

Decision:

- reject threshold `12`
- narrowing the fast path removed the monero regression but still produced clear warm regressions on smaller helper-backed projects

## Conclusion

Do not ship the inline fast-path heuristic as tested here.

What this experiment proved:

- the repeated hot path is real
- a coarse early-inline fast path can help some cold builds
- but the tradeoff is unstable across workloads and degrades warm behavior enough to fail the production bar

## Recommendation

The next speed work should stay focused on measurement, not heuristic widening:

1. keep daemon routing work separate from frontend cold-build work
2. keep the helper lane on the baseline trace-only patch after this experiment
3. instrument where `estimate_size` is invoked, not just what it returns
4. look for narrower reductions in repeated size-estimation work around the specific hot helper families:
   - `core::assert`
   - `core::Felt252PartialEq::eq`
   - `core::byte_array::ByteArrayImpl::at`
   - `core::bytes_31::Bytes31Impl::at`
   - `core::starknet::storage::StorablePointerReadAccessImpl::read`

A credible next experiment would need to preserve warm-noop behavior while producing a clean same-window cold improvement on both monero and braavos.

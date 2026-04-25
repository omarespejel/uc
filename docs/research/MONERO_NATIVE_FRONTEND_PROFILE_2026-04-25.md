# Monero Native Frontend Profile (2026-04-25)

## Decision

Do not ship a `uc` wrapper-level perf PR from this pass.

The monero Cairo `2.14` lane is now native-supported and fast in the measured harness, but the remaining cold-build cost is dominated by Cairo `2.14` frontend lowering/inlining work inside the native helper. The profiler did not identify cache glue, wrapper setup, artifact restore, or Rayon thread scheduling as the next material bottleneck.

## Scope

- Repository under test: `monero-starknet-atomic-swap/cairo`
- Manifest: `Scarb.toml`
- Cairo lane: `2.14.0`
- Native helper source: external Cairo `2.14` helper via `UC_NATIVE_TOOLCHAIN_2_14_BIN`
- Run mode: local-first, offline, daemon off, same-window Scarb vs `uc` harness rerun
- Host class: Apple M3 Pro, arm64, 18 GiB RAM, macOS 26.4.1
- Sample counts: 3 cold, 3 warm-noop

This is a local engineering profile note, not a public launch benchmark claim. The raw benchmark artifacts were written under `benchmarks/results/`, which is ignored by git. Before quoting these numbers externally, publish or commit an immutable benchmark artifact that records the host, binary, repo revision, manifest, helper path, flags, and sample counts.

## Local Benchmark Result

Local artifact directory:

```text
benchmarks/results/monero-hotspot-baseline-20260425-083622/
```

Summary from `real-repo-bench-20260425-083623.json`:

| Lane | Conditions | Scarb p95 | uc p95 | p95 speedup | Stability |
| --- | --- | ---: | ---: | ---: | --- |
| Cold build | Apple M3 Pro, arm64, 18 GiB RAM, macOS 26.4.1, offline, daemon off, 3 cold runs | 23423.016 ms | 5762.864 ms | 4.06x | stable |
| Warm no-op | Apple M3 Pro, arm64, 18 GiB RAM, macOS 26.4.1, offline, daemon off, 3 warm-noop runs | 6427.190 ms | 37.462 ms | 171.57x | stable |

Support matrix:

| Classification | Count |
| --- | ---: |
| native supported | 1 |
| fallback used | 0 |
| native unsupported | 0 |
| build failed | 0 |

Native support probe resolved the lane as:

```text
status=supported
compiler_version=2.14.0
package_cairo_version=2.14.0
toolchain.source=external_helper
compile_backend=uc_native_external_helper
```

## Phase Telemetry

Cold `uc` build phase timings from the three timed runs:

| Phase | Mean | Min | Max |
| --- | ---: | ---: | ---: |
| compile | 4848.889 ms | 4511.927 ms | 5181.126 ms |
| native_frontend_compile | 4451.073 ms | 4168.393 ms | 4669.697 ms |
| native_session_prepare | 124.436 ms | 67.246 ms | 238.139 ms |
| native_casm | 254.928 ms | 252.589 ms | 257.711 ms |
| native_artifact_write | 18.195 ms | 16.490 ms | 20.441 ms |

Interpretation:

- `native_frontend_compile` accounts for roughly 92% of the timed cold compile phase.
- `native_total_contracts=1`, so more Rayon work splitting across contract batches is not the lever for monero.
- Warm no-op runs hit the `uc` cache and do no frontend compile work.

## Sampled Hot Path

A macOS `sample` run against the Cairo `2.14` helper showed the hot path inside Cairo lowering/inlining and size estimation:

```text
cairo_lang_starknet::compile::compile_prepared_db
cairo_lang_sierra_generator::program_generator::get_sierra_program_for_functions
cairo_lang_lowering::db::lowered_body
cairo_lang_lowering::optimizations::strategy::OptimizationPhase::apply
cairo_lang_lowering::inline::apply_inlining
cairo_lang_lowering::db::estimate_size
cairo_lang_sierra_generator::program_generator::get_dummy_program_for_size_estimation
```

This points to repeated lowering/optimization work during inlining size estimation, not to wrapper I/O or benchmark harness overhead.

## Rejected Experiment

A temporary local-only experiment exposed `UC_NATIVE_INLINING_STRATEGY` and rebuilt an isolated Cairo `2.14` helper. It was reverted and not committed.

Result:

- `InliningStrategy::Default`: normal monero cold builds around the existing 5 second range.
- `InliningStrategy::Avoid`: first run was still compiling after more than 175 seconds and was killed.

Decision:

- Do not continue the `Avoid` path.
- Do not ship a user-facing inlining strategy knob without artifact-equivalence tests and a real clean harness win.

## Next Engineering Target

The next credible speed work is inside the Cairo `2.14` helper path, not in more cache glue:

1. Profile whether `estimate_size` / dummy Sierra generation recomputes lowered bodies that should be query-reused.
2. Test a narrowly scoped helper-lane patch that reduces duplicate size-estimation work without changing generated Sierra/CASM.
3. Validate by comparing artifacts against the reference helper and rerunning the same-window monero harness.
4. Ship only if the real harness improves cleanly and artifact equivalence holds.

If that patch is too invasive for local ownership, the production-grade alternative is to open an upstream Cairo issue with this profile and keep `uc`'s launch messaging focused on support coverage, diagnostics, and warm-cache behavior.

## Reproduction Shape

Use the standard local-first harness pattern:

```bash
export UC_NATIVE_TOOLCHAIN_2_14_BIN=/path/to/uc-cairo214-helper/bin/uc
export UC_PHASE_TIMING=1

./scripts/doctor.sh \
  --uc-bin /path/to/uc \
  --manifest-path /path/to/monero-starknet-atomic-swap/cairo/Scarb.toml

./benchmarks/scripts/run_real_repo_benchmarks.sh \
  --uc-bin /path/to/uc \
  --results-dir /path/to/results \
  --runs 3 \
  --cold-runs 3 \
  --warm-settle-seconds 1.0 \
  --case /path/to/monero-starknet-atomic-swap/cairo/Scarb.toml \
  monero
```

Keep same-window reruns for before/after comparisons. Do not compare a new helper patch against older local artifacts from a different time window.

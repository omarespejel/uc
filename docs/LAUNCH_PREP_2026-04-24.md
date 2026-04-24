# Launch Prep 2026-04-24

## Current Position

`uc` is now credible as a multi-toolchain native builder for the measured Cairo `2.14` repo set. The strongest current story is:

- all five measured real repos stayed on native with no fallback
- Cairo `2.14` support is productized via a checked-in helper build flow
- native support probing and build failures now emit stable machine-readable diagnostics
- benchmark output now surfaces repo-level instability instead of hiding it behind medians

This is not yet a "faster everywhere" launch.

## What We Can Claim

Based on the same-window lane `current-contracts-native-productized-20260424T213026Z`:

- host: Apple M3 Pro, 18 GiB RAM, macOS 26.4.1
- binaries: `target/release/uc` plus `UC_NATIVE_TOOLCHAIN_2_14_BIN=.uc/toolchain-helpers/uc-cairo214-helper/bin/uc`
- scenario: local offline Scarb manifests, `--daemon-mode off`, strict native benchmark mode, 5 cold runs, 5 warm-noop runs, 2.2s warm-settle
- dataset: `erc8004`, `agent-account`, `token-factory`, `braavos`, and `monero` real repo manifests listed in the benchmark JSON

- support matrix: 5 `native_supported`, 0 `fallback_used`, 0 `native_unsupported`, 0 `build_failed`
- `erc8004`: cold `2.277x`, warm-noop `6.058x`
- `agent-account`: cold `2.603x`, warm-noop `4.158x`
- `braavos`: cold `1.383x`, warm-noop `38.086x`
- `monero`: cold `1.777x`, warm-noop `175.247x`

Operational claims we can also make now:

- helper lanes can be built locally with `./scripts/build_native_toolchain_helper.sh --lane 2.14`
- `./scripts/doctor.sh --uc-bin <path> --manifest-path <Scarb.toml>` detects missing helper lanes before a build
- `uc support native --format json` and `uc build --report-path ...` provide stable diagnostic codes and remediation fields for agents and CI

## What We Should Not Claim

- do not claim uniform wins on every repo and every lane
- do not claim the cold path is solved
- do not claim there is a proven new compiler optimization in this PR

The same-window sweep shows one clear holdout:

- `token-factory`: cold `1.067x`, warm-noop `0.481x`

It also shows instability warnings on multiple lanes, including `token-factory` warm-noop and `erc8004` cold. That is why the benchmark harness now emits explicit stability warnings and support-matrix counts.

## Performance Conclusion

The profiling evidence still points to the same bottleneck:

- cold helper-lane builds remain dominated by `native_frontend_compile`
- no cache/setup-only patch produced a clean material improvement worth merging
- the explored Rayon thread-cap direction was rejected because it regressed real runs

The next performance work should target frontend compile hotspots, starting with the weakest supported repo in the latest sweep.

## Launch Sequence

1. Launch helper-backed Cairo `2.14` support and agent-grade diagnostics as the concrete product improvement.
2. Present the support matrix and benchmark table together, including the instability caveat.
3. Hold broader "faster than Scarb" messaging until the weakest repo lane is improved or explicitly carved out.

## Required Follow-Up

1. Profile `token-factory` warm-noop and cold helper-lane behavior directly.
2. Keep per-repo stability warnings in every public benchmark table.
3. Extend helper lanes beyond Cairo `2.14` only after the next supported-version demand is concrete.

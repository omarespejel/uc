# Helper Daemon Routing Experiment 2026-04-29

## Goal

Allow external-helper native lanes, especially Cairo `2.14`, to use daemon mode safely without
talking to the wrong daemon binary or silently downgrading to Scarb.

## Change

Two changes were required:

1. External helper subprocesses now get a helper-scoped `UC_DAEMON_SOCKET_PATH` derived from the
   canonical helper binary path.
2. Daemon native requests that resolve to an external helper are allowed only when the daemon
   process is the matching helper binary itself. Non-matching binaries still reject the request.

This keeps the original safety property against cross-binary daemon reuse while unlocking the
valid self-hosted helper-daemon case.

## Validation

Targeted tests:

- `cargo test -p uc-cli execute_daemon_build_rejects_external_helper_native_request -- --nocapture`
- `cargo test -p uc-cli daemon_socket_path_for_external_helper_is_stable_and_helper_scoped -- --nocapture`
- `cargo test -p uc-cli build_uc_build_command_sets_helper_scoped_daemon_socket_override -- --nocapture`

Artifacts:

- lane `daemon-helper-warm-20260429`: `benchmarks/results/daemon-helper-warm-20260429.json`
- lane `daemon-helper-require-warm-20260429`: `benchmarks/results/daemon-helper-require-warm-20260429.json`
- lane `daemon-helper-glint-20260429`: `benchmarks/results/daemon-helper-glint-20260429.json`

Helper lane rebuilt from this branch before measurement:

- `<HOME>/.uc/toolchain-helpers/uc-cairo214-helper/bin/uc`

Run conditions:

- host class: local macOS Apple Silicon workstation (`Darwin arm64`, `Apple M3 Pro`)
- native mode: `UC_NATIVE_BUILD_MODE=require`
- toolchain lane: external helper Cairo `2.14`
- scenario: helper-backed native builds on fresh temp copies with second-run warm comparisons
- daemon comparison: `--daemon-mode off` vs `--daemon-mode require`

## Result

With the helper daemon explicitly started on its scoped socket, second-run warm builds stayed on
`uc_native_external_helper` with `daemon_used=true` and `cache_hit=true` for all measured cases.

Warm second-run results under lane `daemon-helper-require-warm-20260429` versus the `daemon-helper-warm-20260429`
source comparison:

- `monero_atomic_swap`: `33.117ms` off vs `32.122ms` require (`1.03x`)
- `braavos_account`: `43.997ms` off vs `42.904ms` require (`1.03x`)
- `glint_contracts`: `30.862ms` off vs `29.035ms` require (`1.06x`)
- `zcash_relay`: `27.732ms` off vs `41.289ms` require (`0.67x`)

## Conclusion

This is worth shipping as a correctness and daemon-availability fix for external-helper lanes.

It is **not** a clean performance PR by itself:

- heavier helper-backed contracts see small warm wins,
- tiny helper-backed workloads can still regress because the daemon request overhead is larger than
  the local cache-hit path.

## Recommendation

Merge the routing fix as a daemon correctness improvement, then do any follow-up performance work
behind a separate slice focused on reducing helper-daemon request overhead for very small warm
builds.

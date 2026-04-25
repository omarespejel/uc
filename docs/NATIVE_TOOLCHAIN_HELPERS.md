# Native Toolchain Helpers

`uc` selects a native Cairo lane before compile starts.

- The active binary provides the builtin lane for its baked-in `cairo-lang` version.
- Older lanes, such as Cairo `2.14`, are supplied via external helper binaries.
- Only lanes listed under `workspace.metadata.uc-native-toolchain-helpers` in `Cargo.toml` are productized for automatic helper building.

## Build The Cairo 2.14 Helper

```bash
./scripts/build_native_toolchain_helper.sh --lane 2.14
# Then run the exact `export UC_NATIVE_TOOLCHAIN_2_14_BIN=...` command printed by the script.
```

To validate the compatibility lane without producing a release binary:

```bash
./scripts/build_native_toolchain_helper.sh --lane 2.14 --check-only
```

The helper builder:
- stages an isolated copy of the current repo
- rewrites the workspace Cairo dependencies to exact `2.14.0`
- removes the local `third_party` Cairo patches that only apply to the main lane's cairo-lang version
- builds the current `uc` command surface with the lane-specific helper compatibility feature enabled
- runs targeted `uc-cli` regression tests for the helper-only compatibility paths

## Compatibility Guardrails

The helper rewriter is fail-closed: it rewrites only the current workspace dependency shape and exits if a required Cairo dependency line cannot be rewritten exactly once. A `[patch.crates-io]` section is optional; when present, the helper staging tree drops it because the main-lane `third_party` Cairo patches are not compatible with the helper lane.

The helper-lane compatibility shims are covered by targeted regressions for:

- unused-import diagnostics not becoming part of the helper session key
- removed tracked files invalidating cached native content
- native crate-cache restore preserving existing config fields
- file-keyed update behavior for removed untracked file slots

## Preflight A Real Manifest

```bash
./scripts/doctor.sh \
  --uc-bin /abs/path/to/uc \
  --manifest-path /abs/path/to/project/Scarb.toml
```

If a repo needs an external helper lane, doctor will report the missing or invalid `UC_NATIVE_TOOLCHAIN_<major>_<minor>_BIN` env var before a build starts.

If a repo asks for a lane that is not productized by this release, `uc support native --format json` emits `UCN1006` with `safe_automated_action=manual_legacy_adapter_required`. Agents must keep that repo in the support matrix as `native_unsupported` unless a reviewed compatible helper binary is supplied explicitly through the reported helper env var.

## Cairo 2.5 Boundary

Cairo `2.5` is not a productized helper-builder lane in this release. It is older than the Cairo `2.6` split that introduced `cairo-lang-starknet-classes`, and it also predates several native compile APIs used by the current helper shim. Supporting it requires a dedicated legacy compatibility adapter rather than a metadata-only helper lane.

Until that adapter exists, Cairo `2.5` workloads should be included in support matrices and classified as `native_unsupported`, not excluded from the benchmark/support story.

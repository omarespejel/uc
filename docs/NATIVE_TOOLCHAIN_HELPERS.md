# Native Toolchain Helpers

`uc` selects a native Cairo lane before compile starts.

- The active binary provides the builtin lane for its baked-in `cairo-lang` version.
- Older lanes, such as Cairo `2.14`, are supplied via external helper binaries.

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
- removes the local `third_party` Cairo patches that only apply to the main `2.16` lane
- builds the current `uc` command surface with the lane-specific helper compatibility feature enabled
- runs targeted `uc-cli` regression tests for the helper-only compatibility paths

## Preflight A Real Manifest

```bash
./scripts/doctor.sh \
  --uc-bin /abs/path/to/uc \
  --manifest-path /abs/path/to/project/Scarb.toml
```

If a repo needs an external helper lane, doctor will report the missing or invalid `UC_NATIVE_TOOLCHAIN_<major>_<minor>_BIN` env var before a build starts.

# Native Toolchain Helpers

`uc` selects a native Cairo lane before compile starts.

- The active binary provides the builtin lane for its baked-in `cairo-lang` version.
- Older supported lanes are supplied through external helper binaries.
- Productized helper lanes are listed in workspace metadata and mirrored in packaged diagnostics.

## Build The Cairo 2.14 Helper

```bash
./scripts/build_native_toolchain_helper.sh --lane 2.14
# Then run the exact export command printed by the script.
```

To validate the lane without producing a release binary:

```bash
./scripts/build_native_toolchain_helper.sh --lane 2.14 --check-only
```

The helper builder:

- stages an isolated copy of the current repo,
- rewrites Cairo dependencies to the lane version,
- removes main-lane patches that do not apply to the helper lane,
- applies reviewed lane-specific patch files from `toolchains/cairo-2.14/patches/*.patch`,
- builds the current `uc` command surface with helper compatibility enabled,
- runs targeted `uc-cli` regression tests for helper-only paths.

## Compatibility Guardrails

The helper rewriter is fail-closed. It rewrites only the current workspace dependency shape and exits if a required dependency line cannot be rewritten exactly once.

Lane-specific Cairo patches are applied only in the helper staging tree:

- lane metadata can set `patch-dir = "toolchains/cairo-2.14/patches"`,
- patch files must be named after the patched crate,
- patched sources live only under `.uc/helper-lane-patches/`,
- the staging manifest receives a fresh patch section pointing at those copies,
- the staging lockfile is refreshed before locked helper builds or tests.

## Cairo 2.14 Frontend Trace

The Cairo `2.14` helper lane includes opt-in trace points for size-estimation work:

```bash
UC_CAIRO214_SIZE_TRACE=/tmp/uc-cairo214-size-trace.tsv \
  UC_PHASE_TIMING=1 \
  UC_NATIVE_TOOLCHAIN_2_14_BIN=/abs/path/to/uc-cairo214-helper \
  uc build --engine uc --daemon-mode off --offline --manifest-path /abs/path/to/project.toml
```

When the variable is unset, the patched helper is silent. When set, the helper appends bounded TSV counter samples. Use this only for local diagnostics, not benchmark claims.

## Preflight A Manifest

```bash
./scripts/doctor.sh \
  --uc-bin /abs/path/to/uc \
  --manifest-path /abs/path/to/project.toml
```

If a project needs an external helper lane, doctor reports the missing or invalid `UC_NATIVE_TOOLCHAIN_<major>_<minor>_BIN` env var before build starts.

If a project asks for a lane that is not productized by this release, `uc support native --manifest-path <manifest> --format json` emits `UCN1006` with `safe_automated_action=manual_legacy_adapter_required`. Agents must keep that project in the support matrix as `native_unsupported` unless a reviewed compatible helper binary is supplied explicitly.

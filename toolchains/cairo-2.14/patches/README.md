# Cairo 2.14 Helper Lane Patches

This directory contains reviewed Cairo `2.14` helper-lane patches.

Patch files must be named after the patched crate, for example:

```text
cairo-lang-compiler.patch
cairo-lang-lowering.patch
cairo-lang-sierra-generator.patch
```

The helper builder applies each patch to an exact `cairo-lang` crate source copied from the local Cargo registry for the lane's locked version, then appends a staging-only `[patch.crates-io]` section pointing at the patched copy. Do not commit vendored crate sources here.

Current patches add opt-in frontend size-estimation trace counters for the Cairo `2.14` helper lane. They are silent unless `UC_CAIRO214_SIZE_TRACE=/abs/path/to/trace.tsv` is set while running that helper. Trace rows use cheap DB-discriminated Salsa ids by default; set `UC_CAIRO214_SIZE_TRACE_NAMES=1` only when per-row bounded function previews and stable FNV-1a grouping hashes are needed.

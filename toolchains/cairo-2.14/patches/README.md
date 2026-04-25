# Cairo 2.14 Helper Lane Patches

This directory is intentionally empty until a reviewed Cairo `2.14` helper-lane patch is needed.

Patch files must be named after the patched crate, for example:

```text
cairo-lang-compiler.patch
cairo-lang-lowering.patch
cairo-lang-sierra-generator.patch
```

The helper builder applies each patch to an exact `cairo-lang` crate source copied from the local Cargo registry for the lane's locked version, then appends a staging-only `[patch.crates-io]` section pointing at the patched copy. Do not commit vendored crate sources here.

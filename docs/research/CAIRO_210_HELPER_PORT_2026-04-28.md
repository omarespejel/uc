# Cairo 2.10 Helper Port Notes

## Purpose

This note records the first production-grade blockers encountered while trying
to turn the `argent_contracts` support gap into a real Cairo `2.10` native
helper lane.

This is not a launch claim and not a productization decision. It is a verified
boundary note so future work starts from evidence instead of guesswork.

## Corpus Context

The post-PR58 20-case diagnostic sweep produced this support matrix:

- `native_supported=12`
- `native_unsupported=6`
- `build_failed=2`
- `fallback_used=0` (exclusive classification)

The agent-grade diagnostics fix removed `UCO5001` from the two build-failed
fallback cases, so the highest-value remaining coverage gap in the current
corpus is Cairo `2.10.1` for `argent_contracts`.

## Finding 1: The helper builder cannot yet rewrite Cairo 2.10 salsa metadata

Scratch reproduction:

```bash
tmpdir=$(mktemp -d /tmp/uc-cairo210-scratch.XXXXXX)
rsync -a --exclude '.git' --exclude 'target' --exclude '.uc' --exclude 'benchmarks/results' ./ "$tmpdir/"
# Add lane metadata and feature alias in the scratch copy, then:
(cd "$tmpdir" && ./scripts/build_native_toolchain_helper.sh --lane 2.10 --check-only)
```

Observed failure:

```text
failed to rewrite salsa in .../Cargo.toml
```

Why this happens:

- the helper rewriter assumes the workspace dependency is shaped like
  `salsa = "..."`;
- Cairo `2.10.1` depends on `rust-analyzer-salsa 0.17.0-pre.6`;
- older-compatible helper lanes therefore need lane metadata that can rewrite
  the `salsa` dependency to an aliased Cargo package form.

Primary-source crate metadata inspected locally from crates.io:

- `cairo-lang-compiler 2.10.1` depends on
  `salsa = { package = "rust-analyzer-salsa", version = "0.17.0-pre.6" }`
- `cairo-lang-starknet 2.10.1` and
  `cairo-lang-starknet-classes 2.10.1` are present on crates.io, so this is
  not the same boundary as Cairo `2.4` or `2.5`

## Finding 2: Solver-generated lockfiles are not safe enough for this lane

Scratch reproduction after manually rewriting the workspace dependency shape:

```bash
tmpdir=$(mktemp -d /tmp/uc-cairo210-compile.XXXXXX)
rsync -a --exclude '.git' --exclude 'target' --exclude '.uc' --exclude 'benchmarks/results' ./ "$tmpdir/"
# Rewrite Cairo deps to =2.10.1, rewrite salsa to rust-analyzer-salsa, add helper-cairo-210 feature alias.
(cd "$tmpdir" && cargo check -p uc-cli --no-default-features --features native-compile,helper-cairo-210)
```

Observed failure:

```text
error[E0570]: "aapcs" is not a supported ABI for the current target
...
error: could not compile `size-of` (lib) due to 5 previous errors
```

What this means:

- a naive lane port that lets Cargo solve a fresh lockfile will pull transitive
  crates that are not acceptable on the current host/toolchain combination;
- the helper lane therefore needs a reviewed, lane-specific lockfile for the
  current `uc` workspace shape, not just upstream Cairo's lockfile and not an
  unconstrained local solve.

## Finding 3: Upstream Cairo's lockfile is useful input, but not drop-in

Primary source:

- `https://raw.githubusercontent.com/starkware-libs/cairo/v2.10.1/Cargo.lock`

Scratch probe:

```bash
curl -L https://raw.githubusercontent.com/starkware-libs/cairo/v2.10.1/Cargo.lock -o /tmp/Cargo.lock
# Place it into a scratch uc copy after rewriting deps, then:
cargo check -p uc-cli --locked --no-default-features --features native-compile,helper-cairo-210
```

Observed failure:

```text
error: the lock file ... needs to be updated but --locked was passed
```

Why:

- upstream Cairo's lockfile matches Cairo's workspace, not `uc`'s current
  helper-stage workspace;
- it is still a useful primary-source reference for expected dependency eras,
  but it is not a drop-in lane lockfile for `uc`.

## Current Recommendation

Do the Cairo `2.10` lane in this order:

1. Land helper-builder support for aliased `salsa` metadata.
2. Generate and review a Cairo `2.10` lane lockfile for the actual `uc`
   helper-stage workspace.
3. Only after the helper builder and lockfile are stable, attempt full helper
   compilation and `argent_contracts` validation.
4. Productize the lane only after `uc support native`, helper build, and the
   affected corpus cases all pass locally under the normal repo validation
   discipline.

## What Not To Do

- Do not mark Cairo `2.10` productized based only on metadata changes.
- Do not reuse the Cairo `2.14` lockfile as if it were valid for `2.10`.
- Do not claim `argent_contracts` native support until the helper lane builds
  and the corpus reclassifies it from `native_unsupported`.

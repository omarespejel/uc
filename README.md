# uc: one binary for Cairo and Starknet

`uc` is a single self-contained binary that builds, tests, verifies, audits and deploys Cairo
contracts. It embeds the Cairo 2.18.0 compiler and corelib, a native `snforge_std` cheatcode
runtime, a Sierra-level symbolic prover with taint detectors, a native registry client for
`scarbs.xyz`, and the read and write side of `sncast`. No `asdf`, no `scarb`, no `snforge` on the
machine is required.

```bash
uc --version
# uc 0.1.0 (cairo 2.18.0, sierra 1.8.0)
```

- Documentation site: [`docs/site/index.html`](docs/site/index.html) (`uv`-style, serves at `http://127.0.0.1:8420/`)
- Slides: [`docs/site/slides/index.html`](docs/site/slides/index.html)
- Agent guide: [`AGENTS.md`](AGENTS.md)
- Issues found in the tools uc replaces: [`docs/competitor-issues.md`](docs/competitor-issues.md)

## What is verified, and how

Everything below was measured on 2026-10-25 on an Apple Silicon laptop against scarb 2.18.0,
snforge 0.60.0 and sncast 0.60.0, on real projects (OpenZeppelin v3 git dependencies, registry
dependencies, garaga) and on deployed mainnet contracts. The scripts are in the repo so the
numbers can be reproduced.

**Byte-identical artifacts.** `uc build` on the erc8004 project (6 contracts, OpenZeppelin v3)
produced the same 13 files as `scarb build`, byte for byte: every `.contract_class.json`,
every `.compiled_contract_class.json` and `starknet_artifacts.json`. Class hashes therefore match,
which is what `uc declare` and `uc verify-onchain` rely on.

**Honest test oracle.** uc's built-in cheatcode runtime was checked against real snforge 0.60
with a matching `snforge_std` pin: betshare 41/41 and erc8004 168/168 tests agree. A mutated
contract makes the expected test fail under uc. (With the project's original older
`snforge_std` pin, real snforge fails 17 of those tests on a version mismatch; uc does not care
which `snforge_std` the manifest names.)

**Fresh machine.** With an empty `HOME`, an empty package cache and no `scarb` on `PATH`,
`uc build` fetched the 12 OpenZeppelin 2.0.0 packages from `scarbs.xyz` (SHA-256 verified,
extracted into scarb's cache layout), built 13 artifacts and ran the 41 betshare tests green.
uc reads packages scarb downloaded; scarb keeps its own download ledger and will fetch again
what uc downloaded.

**Real bytecode.** `benchmarks/scripts/run_mainnet_audit.py` fetches 20 well-known mainnet
classes over JSON-RPC and audits them with no source: 16 are Sierra classes (4 are Cairo 0 or
misaddressed), 2,843 functions analysed in 32 s, 19 reachable-panic witnesses, 102 detector
findings. Results are written to `benchmarks/results/`. They are a measurement of the prover,
not a security review of those protocols.

**Head to head.** `benchmarks/scripts/run_head_to_head.py` copies real projects to a scratch
directory and times cold and warm builds and test runs for both toolchains (median of 3), then
checks artifact parity. Latest run (`benchmarks/results/head-to-head-20261025-162907.json`,
shared machine, load average 60): betshare cold build 11.90 s scarb vs 5.23 s uc, warm 0.27 s vs
0.03 s, 41 tests warm 2.43 s snforge vs 0.85 s uc, 13 of 13 artifacts identical. On the two
OpenZeppelin-v3 git projects scarb could not reach the registry to refresh the lockfile during
the run, so those baselines are missing from that file; earlier the same day on erc8004: cold
`scarb build` 15.0 s, warm 7.0 s, warm `uc build` 2.9 s. Heavy contract-call suites (erc8004's
six tests with over a million VM steps each) are still about 3x slower than snforge because every
nested call reloads the whole CASM program. That gap is real and listed below.

## Install

Linux (`x86_64`, `aarch64`) and macOS (Apple Silicon, Intel). The corelib is embedded in the
binary and extracted on first run.

```bash
curl -LsSf https://raw.githubusercontent.com/omarespejel/uc/main/install.sh | sh
# drop-in symlinks for scarb, snforge, sncast and starknet-devnet
curl -LsSf https://raw.githubusercontent.com/omarespejel/uc/main/install.sh | sh -s -- --drop-in
```

When invoked through a symlink named `scarb`, `snforge`, `sncast` or `starknet-devnet`, uc routes
flags to match that tool.

GitHub Actions:

```yaml
steps:
  - uses: actions/checkout@v4
  - uses: omarespejel/uc@v1
    with:
      drop-in-symlinks: true
  - run: uc check && uc test && uc prove --fail-on-findings
```

## Everyday commands

```bash
uc new my_vault --starknet       # scaffold a contract package
uc check                         # typecheck, cached on sources + deps + lockfile + compiler
uc build                         # Sierra + CASM, byte-identical to scarb 2.18
uc test                          # #[test] and snforge_std tests in-process, per-test timings
uc test --fuzzer-runs 256 --gas-report
uc prove                         # symbolic execution + Sierra taint detectors
uc prove --check-upgrade v1/Scarb.toml
uc prove --check-equivalence v1/Scarb.toml
uc audit 0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d --network mainnet
uc decompile target/dev/my_vault_Vault.contract_class.json
uc fetch                         # download registry dependencies natively
uc call --contract-address 0x... --function name --network mainnet
uc balance --token strk --account 0x... --network mainnet
uc trace 0x<tx hash> --network mainnet
uc get nonce 0x... --network mainnet
```

Every command takes `--json` for machine-readable output and `--report-path <file>` to write it.

## Formal verification and audit

`uc prove` compiles the package, walks every Sierra function symbolically and classifies it:

- `PROVEN_SAFE`: no panic path is satisfiable.
- `GUARDED_SAFE`: panics exist only behind caller or calldata guards (`assert`, auth checks).
- `COUNTEREXAMPLE_FOUND`: a concrete input reaches a panic after arithmetic; the witness is in
  the JSON and `--emit-repro tests/repro.cairo` writes it as a `#[should_panic]` test.

The taint detectors run on Sierra, so they also work on deployed classes with no debug names:
entry points are named from the ABI by selector and `PanicResult` is recognised structurally.
Detectors: unprotected state mutation and arbitrary calls on the entry surface, check-effects-
interactions (lock and reentrancy-guard shapes are downgraded, not hidden), caller-controlled
loop bounds (interprocedural, numeric parameters only, capped bounds excluded), unvalidated
storage writes in constructors and setters, swallowed syscall errors, redundant `divmod`,
bitwise parity. `#[test]` bodies are never symbolic targets; `prove_*`, `invariant_*` and
`check_*` functions are.

## What uc does not do yet

- `uc devnet` without an external `starknet-devnet` binary is a JSON-RPC protocol stub with
  predeployed account addresses and no transaction execution. It prints that on start.
- Nested contract calls in tests reload the whole program per call; heavy integration suites are
  slower than snforge (see the head-to-head numbers).
- `uc fetch` writes `Scarb.lock` only when the package has none; it does not update an existing
  lock, and it leaves `snforge_std` for scarb to add.
- Windows is not built.

## Working on uc

```bash
cargo build --release           # ~5 min cold, incremental afterwards
cargo test --release            # 490 tests
python3 benchmarks/scripts/run_head_to_head.py --uc target/release/uc --project <dir> ...
python3 benchmarks/scripts/run_mainnet_audit.py --uc target/release/uc
```

The compiler crates live in `third_party/` as patched copies of the crates.io 2.18.0 sources
(`cairo-lang-runner` carries the cheatcode runtime, `cairo-lang-starknet` the parallel
diagnostics prewarm). `crates/uc-cli/src/embedded_corelib.bin` is the corelib packed by
`scripts/`; it must match the `cairo-lang-*` version in `Cargo.toml`.

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
`scarb build` 15.0 s, warm 7.0 s, warm `uc build` 2.9 s. Nested contract calls run on per-class program slices and each test on a slice
of its own, so the heavy erc8004 suite (seven tests with over a million VM steps each) now runs
in 11.6 s under uc against 16.9 s under snforge 0.60 (warm, same machine, load average 38), with
a peak of 2.2 GB against 3.3 GB.

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
uc test --gas-report               # per test: steps, syscalls, builtin instances, weighted resource gas
uc test --fuzzer-runs 256
uc prove                         # symbolic execution + Sierra taint detectors
uc prove --emit-smt target/smt   # SMT-LIB obligations per function, checkable with z3
uc prove --check-upgrade v1/Scarb.toml
uc prove --check-upgrade 0x04718f...938d --network mainnet   # against the deployed class
uc declare -c Vault --upgrade-of 0x04718f...938d --network mainnet  # refuses unsafe upgrades
uc prove --check-equivalence v1/Scarb.toml
uc audit 0x04718f5a0fc34cc1af16a1cdee98ffb20c31f5cd61d6ab07201858f4287c938d --network mainnet
uc audit 0x04718f...938d --network mainnet --live-storage   # decide state-dependent paths on chain data
uc watch 0x04718f...938d --network mainnet --interval 60     # re-audit on class change, diff findings
uc decompile target/dev/my_vault_Vault.contract_class.json
uc fetch                         # download registry dependencies natively
uc call --contract-address 0x... --function name --network mainnet
uc balance --token strk --account 0x... --network mainnet
uc trace 0x<tx hash> --network mainnet   # every call named from its class ABI; labels with STARKSCAN_API_KEY
uc audit --declared-since 1693000 --network mainnet --json   # audit feed of newly declared classes
uc get nonce 0x... --network mainnet
```

Every command takes `--json` for machine-readable output and `--report-path <file>` to write it.

## Resource accounting in `uc test`

Every test result carries the VM's builtin instance counts (`range_check`, `pedersen`,
`poseidon`, `bitwise`, `ec_op`, `keccak`, ...) across nested calls, and a `resource_gas`
breakdown that weighs steps and builtins with Starknet's L2 gas costs per resource (blockifier
`os_constants` for 0.13.4+: 100 per step, 70 per range check, 4050 per Pedersen hash, 491 per
Poseidon, 136189 per Keccak, and so on). The breakdown names the dominant resource, so a suite
that is 36% Pedersen stands out at once. `--gas-report` prints it per test plus suite totals and
the ten heaviest tests; `--json` carries `builtins`, `resource_gas` and `resource_totals`. The
weights are the sequencer's and track prover trace cost by construction; they are not a Stwo
trace measurement.

## Formal verification and audit

`uc prove` compiles the package, walks every Sierra function symbolically (up to `--max-depth`
1024 statements per path and `--max-paths` 1024 paths per function) and classifies it:

- `PROVEN_SAFE`: every panic path was explored and none is satisfiable.
- `GUARDED_SAFE`: panics exist only behind caller or calldata guards (`assert`, auth checks).
- `COUNTEREXAMPLE_FOUND`: a concrete input reaches a panic after arithmetic. The witness is in
  the JSON as parameter values and as `counterexample_calldata` (wrapper calldata, `u256` as
  low and high), `--emit-repro tests/repro.cairo` writes it as a `#[should_panic]` test, and
  uc replays it through the entry point wrapper on a fresh state: `concrete_verified` is true
  only when the runner panics for the predicted reason, false when the function returns or
  panics on an unrelated assertion (the symbolic path then depends on storage the replay does
  not have).
- `BOUNDED_SAFE`: no explored panic path is satisfiable, but the depth or path budget was hit.
- `UNDECIDED`: a panic path the in-process solver could neither satisfy nor refute (its witness
  search is capped at 20,000 partial assignments per path).

A counterexample is only reported where the caller chooses the values: entry points,
constructors, handlers, property functions and uncalled functions. In an internal helper or a
loop body, a panic that needs a parameter at its type bound (`i + 1` with `i == u32::MAX`) is a
precondition on the caller and lands in `GUARDED_SAFE` with the negated constraint. The engine
models Cairo 2.18's bounded-int lowering of checked arithmetic (`u64_try_from_felt252`,
`bounded_int_trim_max`, `bounded_int_add`) as well as the `*_overflowing_*` libfuncs.

**Live storage.** `uc audit <address> --live-storage` (and `uc watch --live-storage`) reads the
contract's storage over `starknet_getStorageAt` for every cell at a known address the symbolic
engine meets (plain fields, vector lengths, flags; map entries stay opaque), binds the value, and
replays counterexamples on that state. Paths that depend on a length or flag are then decided
against the chain instead of counted as unknown; the JSON lists every cell read under
`live_storage`.

**Watching a deployment.** `uc watch <address>` polls the class hash at the address (every 60 s,
or `--once` for cron), re-audits when it changes, and prints the findings and counterexamples
that appeared or disappeared relative to the previous class. State lives in
`.uc/watch/<address>.json`; `--json` emits one document per check.

**Checkable proofs.** `uc prove --emit-smt target/smt` writes one SMT-LIB 2 file per function
with every panic path's constraints as a `(check-sat)` obligation and the verdict uc expects
(`unsat` for an unreachable panic, `sat` plus a pinned witness for a counterexample), and
`obligations.json` listing them. `benchmarks/scripts/check_smt_obligations.py --dir target/smt`
runs z3 or cvc5 over the files and reports every disagreement. On 2026-10-25 z3 agreed with uc on
all 838 obligations of erc8004 and all 1,568 of the deployed STRK token class. The obligations
cover the numeric path constraints uc reasons about; values uc treats as opaque (storage reads,
call results) are free constants in the files, so agreement checks the solver, not the model.

The taint detectors run on Sierra, so they also work on deployed classes with no debug names:
entry points are named from the ABI by selector and `PanicResult` is recognised structurally.
Detectors: unprotected state mutation and arbitrary calls on the entry surface, check-effects-
interactions (lock and reentrancy-guard shapes are downgraded, not hidden), caller-controlled
loop bounds (interprocedural, numeric parameters only, capped bounds excluded), unvalidated
storage writes in constructors and setters, swallowed syscall errors, redundant `divmod`,
bitwise parity. `#[test]` bodies are never symbolic targets; `prove_*`, `invariant_*` and
`check_*` functions are.

## Local chain

`uc devnet` runs a Starknet chain in the uc process, no `starknet-devnet` binary needed. On
start it compiles three embedded Cairo 2.18 contracts with its own compiler (an account, an ERC20
fee token, a universal deployer; cached per compiler version), declares them, deploys the tokens
at the mainnet STRK and ETH addresses and the deployer at the mainnet UDC address, and funds the
predeployed accounts. Transactions execute on the same Sierra runner `uc test` uses, one runner
per declared class sharing one class table, so calls cross classes declared at any time.

```bash
uc devnet --port 5050 --accounts 2 --seed 1
uc declare -c Counter --url http://127.0.0.1:5050/rpc --account-address 0x... --private-key 0x...
uc deploy -c Counter --constructor-calldata 40 --salt 0x1 --url http://127.0.0.1:5050/rpc ...
uc invoke --contract-address 0x... --function increase --calldata 2 --url http://127.0.0.1:5050/rpc ...
uc call --contract-address 0x... --function get --url http://127.0.0.1:5050/rpc
```

Served: `starknet_addDeclareTransaction`, `addDeployAccountTransaction`, `addInvokeTransaction`,
`call`, `estimateFee`, `getNonce`, `getClassHashAt`, `getClass`, `getClassAt`, `getStorageAt`,
`getTransactionByHash` / `Receipt` / `Status`, `getBlockWithTxHashes` / `WithTxs`, `getEvents`,
`getStateUpdate`, `traceTransaction`, `blockNumber`, `chainId`, `specVersion`, plus `devnet_mint`,
`devnet_createBlock`, `devnet_setTime`, `devnet_increaseTime`, `devnet_getPredeployedAccounts`
and the `/predeployed_accounts`, `/mint`, `/create_block`, `/is_alive` routes. One block per
transaction (`--block-generation-on demand` to batch). Receipts carry events, revert reasons from
nested calls, steps and builtin counts. Nonces are enforced; signatures and fees are not (a
devnet signs nothing worth protecting). `uc declare` now carries the class in the transaction and
every write command fetches the account nonce first; both were missing before, so they would
have been rejected by any node.

## Starkscan

Set `STARKSCAN_API_KEY` and every `--network mainnet|sepolia` command talks to Starkscan's
JSON-RPC gateway (`https://starkscan.co/api/v1/SN_MAIN/rpc`, header `X-Starkscan-Api-Key`)
instead of the public Cartridge endpoint; reports name the provider in `rpc_provider`. A
`--url https://starkscan.co/rpc/v0_10/SN_MAIN/<key>` URL-token endpoint works as well.

- `uc trace <tx>` names every call in the trace from the ABI of the class it ran (one class
  fetch per distinct class, no key needed) and, with a key, labels every address through
  Starkscan's contract metadata and token endpoints (`alias`, account, token symbol).
- `uc audit <address>` includes Starkscan's source verification record
  (`verificationStatus`, `sourceRef`, `compilerVersion`) when a key is set.
- `uc verify-onchain --verifier starkscan` reads the same record and, with `--submit`, writes
  it through `PUT /v1/{chain}/contract/{address}/verification` (the key needs the verification
  scope; the request body is in the report as `starkscan_upsert_request`).
- `uc audit --declared-since <block> [--declared-until <block>] [--max-classes N]` walks the
  state updates of a block range, audits every Sierra class declared there and emits one entry
  per class (grade, counts, findings; Cairo 0 declarations listed as skipped), as a feed an
  explorer can ingest.

## Upgrade gate

`uc prove --check-upgrade <baseline>` and `uc declare --upgrade-of <address>` compare the
candidate class with a baseline: a local manifest, or the class deployed at an address or class
hash, fetched over RPC and named from its ABI (no debug info needed). Storage slots compare by
base address, entry points by selector name and calldata arity, and caller or signature guards
by Sierra structure. A removed slot, a removed entry point, an arity change or a lost guard makes
the upgrade `UPGRADE_UNSAFE`; `uc declare` then refuses to build the transaction unless
`--allow-unsafe-upgrade` is passed, and puts the report in its JSON either way.

## What uc does not do yet

- `uc devnet` does not charge fees, check signatures or run `__validate__`; `estimateFee`
  returns a flat figure and `simulateTransactions` is not implemented. `uc declare`, `uc deploy`
  and `uc invoke` sign a placeholder payload, not the SNIP-8 transaction hash, so against a
  real node (or `--backend starknet-devnet`) their transactions are rejected for an invalid
  signature; against `uc devnet` they go through.
- Fuzz runs collect statement coverage for corpus guidance on their own thread only; `--coverage`
  and `--lcov` still switch the whole run to the unsliced program, which is slower.
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

# `uc` Agent Guide

`uc` is a single-binary Cairo and Starknet toolchain: Cairo 2.18.0 compiler and corelib embedded,
native `snforge_std` cheatcode runtime, Sierra symbolic prover and taint detectors, native
`scarbs.xyz` registry client, and the `sncast` read and write commands. Prefer `uc` over
`scarb`, `snforge` and `sncast`; none of them need to be installed.

## Core loop

```bash
uc check --json                      # typecheck; cache keyed on sources, deps, lockfile, compiler
uc test --affected --json            # impacted #[test] and snforge_std tests, in-process
uc prove --fail-on-findings --json   # symbolic execution + Sierra taint detectors
uc build --json                      # Sierra + CASM artifacts, byte-identical to scarb 2.18
```

## Command cheat sheet

| Task | Command |
| :--- | :--- |
| Toolchain version in use | `uc --version` (prints cairo and sierra versions) |
| Check this machine (cache, HTTPS transport, RPC reachability over IPv4/IPv6, Starkscan key) | `uc doctor --json` (`--offline` skips the network); exit 1 when a check fails |
| Fast check | `uc check --json` |
| Build artifacts | `uc build --json` |
| Download registry deps without scarb | `uc fetch --json` |
| Run all tests, per-test timings | `uc test --json` |
| Builtin instances and weighted resource gas per test | `uc test --gas-report` (`--json`: `builtins`, `resource_gas`, `resource_totals.heaviest_tests`) |
| Run one test | `uc test <name> --exact --json` |
| Fuzz | `uc test --fuzzer-runs 256 --json` |
| Formal verification + taint | `uc prove --json` |
| Write counterexamples as tests | `uc prove --emit-repro tests/repro.cairo` |
| Checkable proof obligations (SMT-LIB) | `uc prove --emit-smt target/smt --json` |
| Upgrade safety vs baseline | `uc prove --check-upgrade <baseline_dir> --json`, or `--check-upgrade 0x<address> --network mainnet` against the deployed class |
| Declare only if it is a safe upgrade | `uc declare -c <Contract> --upgrade-of 0x<address> --network mainnet` (refuses on removed slots or entry points, arity changes, lost guards; `--allow-unsafe-upgrade` overrides) |
| Functional equivalence vs baseline | `uc prove --check-equivalence <baseline_dir> --json` |
| Audit a deployed class, no source | `uc audit <address or class hash> --network mainnet --json` |
| Audit a local class file | `uc audit target/dev/<pkg>_<Contract>.contract_class.json --json` |
| Audit against live storage | `uc audit 0x<address> --network mainnet --live-storage --json` (`live_storage.values` lists the cells read) |
| Watch a deployment for class changes | `uc watch 0x<address> --network mainnet --once --json` (diff of findings vs the previous class; state in `.uc/watch/`) |
| Decompile Sierra | `uc decompile <path.contract_class.json>` |
| Lint and auto-fix | `uc lint --fix --json` |
| Gas, CASM size, Stwo AIR | `uc profile --prover-air --json` |
| Format | `uc fmt` |
| Class hash offline | `uc class-hash --casm --json` |
| Selector | `uc selector <name_or_hex> --json` |
| Address of a UDC deployment | `uc utils contract-address --class-hash 0x.. --salt 0x.. [--unique --deployer-address 0x..] --constructor-calldata ..` (OpenZeppelin's Cairo 1 UDC, as sncast 0.60) |
| SNIP-8 hash of a V3 transaction | `uc utils tx-hash --tx tx.json --chain-id SN_SEPOLIA --json` (INVOKE, DECLARE, DEPLOY_ACCOUNT) |
| Read-only call | `uc call --contract-address 0x.. --function <name> --network mainnet --json` |
| Nonce, class hash at, balance | `uc get nonce 0x.. --network mainnet`, `uc get class-hash-at 0x.. --network mainnet`, `uc balance --token strk --account 0x.. --network mainnet` |
| Transaction trace, calls named from class ABIs | `uc trace <tx_hash> --network mainnet --json` (`calls[]`, `participants`) |
| Audit feed of declared classes | `uc audit --declared-since <block> --declared-until <block> --max-classes 50 --network mainnet --json` |
| Local chain | `uc devnet --port 5050 --accounts 2 --seed 1` (`/predeployed_accounts` lists keys), then `uc declare` / `uc deploy` / `uc invoke` / `uc call` with `--url http://127.0.0.1:5050/rpc` |
| Declare / deploy / invoke (dry run) | `uc declare -c <Contract> --dry-run --json` |
| Deploy an account | `uc account deploy --class-hash 0x.. --private-key 0x.. --salt 0x1 --url <rpc> --json` |

RPC resolution for on-chain commands: `--url`, else `STARKNET_RPC`, else the `snfoundry.toml`
profile in the current directory, else `--network mainnet|sepolia|devnet`: Starkscan's gateway
(`https://starkscan.co/api/v1/{SN_MAIN|SN_SEPOLIA}/rpc`, header `X-Starkscan-Api-Key`) when
`STARKSCAN_API_KEY` is set, else the public Cartridge endpoints. Reports include `rpc_url` and
`rpc_provider`. With the key, `uc trace` labels addresses, `uc audit` reports the Starkscan
verification record, and `uc verify-onchain --submit` writes it.

## Output and exit codes

- `--json` prints one JSON document to stdout; `--report-path <file>` writes it to disk.
- Exit `0`: success (`passed = true`, no errors, no unguarded counterexamples with
  `--fail-on-findings`). Exit `1`: compilation error, test failure, lint error, upgrade
  regression, or a counterexample under `--fail-on-findings`.
- `uc prove` statuses per Sierra function: `PROVEN_SAFE` (every panic path explored, none
  satisfiable), `GUARDED_SAFE` (panics only behind tests of the inputs, conditions on state, or
  a callee's guards),
  `COUNTEREXAMPLE_FOUND` (inputs in `counterexample`, wrapper calldata in
  `counterexample_calldata`; `concrete_verified` true when the replay panicked for the predicted
  reason, false when it returned or hit an unrelated assertion), `BOUNDED_SAFE` (depth or path
  budget hit, nothing found), `UNDECIDED` (a panic path the solver could not decide, or behind
  a callee that is itself undecided). Calls are inlined ten frames deep; a call that is not
  inlined takes the callee's verdict (callees are analysed first).
  `exploration_truncated` says whether `--max-depth` / `--max-paths` (both 1024) were hit.
  Counterexamples are reported only where the caller chooses the values (entry surface,
  property functions); bound-hitting overflows inside internal helpers and loop bodies are
  preconditions (`GUARDED_SAFE`).
  Plain `#[test]` bodies are not symbolic targets; `prove_*`, `invariant_*`, `check_*` are.
- `uc prove --emit-smt <dir>` writes SMT-LIB obligations per function plus `obligations.json`;
  `benchmarks/scripts/check_smt_obligations.py --dir <dir>` checks them with z3 or cvc5.
- Detector findings carry `severity`, `confidence`, `detector`, `function`,
  `sierra_statement_idx`, `description`, `recommendation`, `taint_flow`. Reentrancy
  (`caracal-cei-reentrancy-sierra`) is `Medium` at most: `Medium` when storage is read before an
  external call and written after it with no guard, `Low` otherwise. Writes with no caller check
  (`caracal-unprotected-state-mutation`) are `Critical` for upgrades, `High` for privileged names
  (`set_*`, `mint`, `withdraw`, ...), `Low` otherwise; L1 handlers are not checked by it.

## Known limits (do not work around silently)

- `uc devnet` executes declare / deploy / invoke / call in process. Accounts validate ECDSA
  signatures, and fees are charged in STRK to the sequencer. Bounds, balances and Blake2s
  compiled class hashes are checked, as a sequencer does: errors 53, 54, 55 and 60. `estimateFee`
  and `simulateTransactions` run on a copy of the state. L2 gas is lower than on starknet-devnet,
  because the runner's syscall costs are lighter. `--backend starknet-devnet` runs the external
  binary instead.
- Write commands sign the SNIP-8 V3 hash with `--private-key` / keystore; `--dry-run` reports
  `computed_transaction_hash` and the signed payload without sending. Fee estimates use a signed
  query-version copy (validation included); bounds are 1.5x the estimate. `uc account deploy
  --class-hash <hash> --private-key <key> --salt <salt>` deploys an account (fund the
  `contract_address` from its `--dry-run` report first).
- `--coverage` and `--lcov` run every test on the unsliced program (slower); plain runs and fuzz
  runs use per-test and per-class program slices.
- `uc fetch` writes `Scarb.lock` only when the package has none.
- If a build falls back to scarb and scarb is missing, the error names the native reason to
  check with `uc support native --json`.

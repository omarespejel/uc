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
| Fast check | `uc check --json` |
| Build artifacts | `uc build --json` |
| Download registry deps without scarb | `uc fetch --json` |
| Run all tests, per-test timings | `uc test --json` |
| Builtin instances and weighted resource gas per test | `uc test --gas-report` (`--json`: `builtins`, `resource_gas`, `resource_totals.heaviest_tests`) |
| Run one test | `uc test <name> --exact --json` |
| Fuzz | `uc test --fuzzer-runs 256 --json` |
| Formal verification + taint | `uc prove --json` |
| Write counterexamples as tests | `uc prove --emit-repro tests/repro.cairo` |
| Upgrade safety vs baseline | `uc prove --check-upgrade <baseline_dir> --json` |
| Functional equivalence vs baseline | `uc prove --check-equivalence <baseline_dir> --json` |
| Audit a deployed class, no source | `uc audit <address or class hash> --network mainnet --json` |
| Audit a local class file | `uc audit target/dev/<pkg>_<Contract>.contract_class.json --json` |
| Decompile Sierra | `uc decompile <path.contract_class.json>` |
| Lint and auto-fix | `uc lint --fix --json` |
| Gas, CASM size, Stwo AIR | `uc profile --prover-air --json` |
| Format | `uc fmt` |
| Class hash offline | `uc class-hash --casm --json` |
| Selector | `uc selector <name_or_hex> --json` |
| Read-only call | `uc call --contract-address 0x.. --function <name> --network mainnet --json` |
| Nonce, class hash at, balance | `uc get nonce 0x.. --network mainnet`, `uc get class-hash-at 0x.. --network mainnet`, `uc balance --token strk --account 0x.. --network mainnet` |
| Transaction trace | `uc trace <tx_hash> --network mainnet --json` |
| Declare / deploy / invoke (dry run) | `uc declare -c <Contract> --dry-run --json` |

RPC resolution for on-chain commands: `--url`, else `STARKNET_RPC`, else the `snfoundry.toml`
profile in the current directory, else `--network mainnet|sepolia|devnet` (public Cartridge
endpoints). Every report includes the `rpc_url` used.

## Output and exit codes

- `--json` prints one JSON document to stdout; `--report-path <file>` writes it to disk.
- Exit `0`: success (`passed = true`, no errors, no unguarded counterexamples with
  `--fail-on-findings`). Exit `1`: compilation error, test failure, lint error, upgrade
  regression, or a counterexample under `--fail-on-findings`.
- `uc prove` statuses per Sierra function: `PROVEN_SAFE` (no satisfiable panic path),
  `GUARDED_SAFE` (panics only behind caller or calldata guards), `COUNTEREXAMPLE_FOUND` (concrete
  inputs in `counterexample`; `concrete_verified` is set when it was replayed in the runner).
  Plain `#[test]` bodies are not symbolic targets; `prove_*`, `invariant_*`, `check_*` are.
- Detector findings carry `severity`, `confidence`, `detector`, `function`,
  `sierra_statement_idx`, `description`, `recommendation`, `taint_flow`. Guarded
  check-effects-interactions shapes (a write before the external call) are reported at `Low`.

## Known limits (do not work around silently)

- `uc devnet` without an external `starknet-devnet` on `PATH` is a protocol stub: JSON-RPC shape
  only, no transaction execution. It says so on start.
- `--coverage` and `--lcov` run every test on the unsliced program (slower); plain runs and fuzz
  runs use per-test and per-class program slices.
- `uc fetch` writes `Scarb.lock` only when the package has none.
- If a build falls back to scarb and scarb is missing, the error names the native reason to
  check with `uc support native --json`.

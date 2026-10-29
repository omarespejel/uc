# Working on Projects

`uc` manages the complete lifecycle of Cairo packages and Starknet smart contracts using standard `Scarb.toml`, `Scarb.lock`, and `snfoundry.toml` files.

> [!TIP]
> **Self-Contained on Linux & macOS**: `uc` embeds the Cairo standard library (`corelib`) directly inside its binary (`crates/uc-cli/src/embedded_corelib.bin`). Copying the `uc` binary to any fresh Linux (`x86_64` / `aarch64`) or macOS (`arm64` / `x86_64`) machine works immediately with zero external files.

---

## Project Structure

```text
my_vault/
├── Scarb.toml          # Package metadata, edition, dependencies, and target configuration
├── Scarb.lock          # Deterministic dependency lockfile
├── snfoundry.toml      # Optional [sncast.<profile>] RPC URLs, accounts, and keystores
├── src/
│   └── lib.cairo       # Root Cairo module and #[starknet::contract] definitions
└── tests/
    └── test_vault.cairo # Integration tests using snforge_std cheatcodes
```

---

## Creating & Managing Packages

| Command | Description |
| :--- | :--- |
| `uc new <path>` | Create a new Cairo package or Starknet contract project |
| `uc init` | Initialize a project in the current directory |
| `uc add <dep>` / `uc rm <dep>` | Add or remove dependencies in `Scarb.toml` |
| `uc tree` | Render the resolved dependency graph |
| `uc update` / `uc list <pkg>` | Update `Scarb.lock` or list available versions from the registry |
| `uc package` / `uc publish` | Pack `.tar.zst` archive or publish to the Scarb registry |

---

## Compiling, Testing & Formal Verification

1. **`uc check`**: syntax, macro expansion, semantic and lowering diagnostics. A few hundred ms cold on a small package; a cache hit (keyed on sources, dependencies, lockfile and compiler version) answers in about a millisecond.
2. **`uc build`**: emits `.contract_class.json`, `.compiled_contract_class.json` and `starknet_artifacts.json` into `target/dev/`, byte-identical to scarb 2.18 for the same sources. Registry dependencies are downloaded natively from `scarbs.xyz` (checksum verified) when missing; `uc fetch` does only that and writes `Scarb.lock` if the package has none.
3. **`uc test`**: unit tests, `snforge_std` cheatcodes (`declare`, `deploy`, `precalculate_address`, `start_cheat_caller_address`, `spy_events`, `start_mock_call`, `replace_bytecode`), property fuzzing (`-r/--fuzzer-runs`) and live RPC state forking (`--fork-url`), with per-test timings.
4. **`uc prove`** and **`uc audit`**: symbolic interval execution, Sierra taint detectors, stateful invariant campaigns, upgrade safety (`--check-upgrade`) and equivalence proofs (`--check-equivalence`). `uc audit <address>` runs the same analysis on a deployed class with no source.
5. **`uc devnet`**: launches an installed `starknet-devnet` with the `0xSpaceShard/starknet-devnet-rs` flags. Without one it serves a JSON-RPC protocol stub (predeployed account addresses, `/mint`, `/create_block`) with no transaction execution, and says so on start.

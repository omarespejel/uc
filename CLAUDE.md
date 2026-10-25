# `uc` Guide for Claude & AI Agents

See [`AGENTS.md`](AGENTS.md) for the complete agent command table and JSON output contracts.

## Key Rules

1. **Use `uc` for everything**: `uc` natively implements `scarb` (`build`, `check`, `fmt`, `doc`, `execute`, `add`, `remove`, `tree`, `package`, `publish`), `snforge` (`test`, `optimize-inlining`, `clean-cache`, `check-requirements`), `sncast` (`call`, `declare`, `declare-from`, `deploy`, `invoke`, `multicall`, `balance`, `get`, `account`, `tx-status`, `verify-onchain`, `utils`), `cairo-language-server` (`lsp`), and `caracal` (`prove`, `decompile`).
2. **Always pass `--json`** when parsing command outputs programmatically.
3. **Use `uc check` before `uc build`** during iterative editing for sub-50ms semantic feedback.
4. **Use `uc prove`** on any Starknet contract modification to catch unprotected storage writes, taint flows, arithmetic overflows, stateful invariant violations, and upgrade storage collisions.

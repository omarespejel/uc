# `scarb_smoke` Fixture Scope

This fixture intentionally combines multiple Starknet patterns so UC cache/fingerprint logic is exercised on realistic Cairo shapes while keeping benchmark runtime small.

Included patterns:
- Multiple `#[starknet::contract]` modules (`token`, `registry`, `permissioned_vault`)
- Cross-contract dispatcher call (`token -> registry`)
- Interface-heavy ABI surfaces (`IToken`, `IRegistry`, `IPermissionedVault`)
- Nested storage keys and tuple maps for deterministic artifact stress

Security/trust notes for fixture-only code:
- `token::sync_permission_seed` calls an external registry contract; a reentrancy guard is enabled in the fixture.
- The fixture models benchmark behavior, not production contract hardening. Production contracts should still implement complete threat-model-specific controls.

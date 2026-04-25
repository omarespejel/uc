# Agent Diagnostics Contract

This document is the stable contract for machine-readable `uc` diagnostics. Human terminal output may change; agent-facing JSON must remain boring, versioned, and parseable.

## Contract

Every `NativeDiagnostic` emitted by `uc support native --format json`, `uc build --json`, or a build report must include:

- `schema_version`: integer schema version. Current version: `1`.
- `code`: stable diagnostic code such as `UCN1004`.
- `category`: stable machine category.
- `severity`: `info`, `warn`, or `error`.
- `title`: short human label.
- `docs_url`: absolute GitHub remediation URL with a stable diagnostic-code anchor.
- `what_happened`: concrete failure statement.
- `why`: root cause or best-known causal explanation.
- `how_to_fix`: ordered remediation text for humans and agents.
- `next_commands`: commands an agent can run next without parsing prose.
- `safe_automated_action`: symbolic action policy for autonomous agents.
- `retryable`: whether retrying the same operation can plausibly succeed after remediation.
- `fallback_used`: whether `uc` downgraded from native to Scarb.
- `toolchain_expected`: expected Cairo/toolchain lane when relevant.
- `toolchain_found`: found compiler/helper/path when relevant.

## Codes

### UCN0001

Native compile support is unavailable because the binary was built without the native feature.

- Category: `feature_unavailable`
- Safe action: `use_native_enabled_binary`
- Agent behavior: switch to a native-enabled `uc` binary before retrying native support checks.

### UCN1000

Legacy edition toolchain could not be resolved.

- Category: `toolchain_resolution`
- Safe action: `manual_manifest_or_lockfile_fix_required`
- Agent behavior: run `scarb metadata` and inspect `Scarb.lock`; do not edit manifests without explicit source-edit permission.

### UCN1001

Native toolchain mismatch.

- Category: `toolchain_mismatch`
- Safe action: `select_matching_toolchain_lane`
- Agent behavior: select or build a helper lane matching the manifest/lockfile major.minor Cairo version.

### UCN1002

Unsupported manifest Cairo-version constraint.

- Category: `manifest_version`
- Safe action: `manual_manifest_or_lockfile_fix_required`
- Agent behavior: report the unsupported range and request/require an exact lane source. Do not rewrite dependency ranges automatically.

### UCN1003

Unparseable native compiler version.

- Category: `compiler_version`
- Safe action: `select_matching_toolchain_lane`
- Agent behavior: inspect `uc support native --format json` and `scarb --version`; use a released helper lane if the active binary reports a development or unparseable version.

### UCN1004

Required native toolchain helper is missing.

- Category: `toolchain_lane_unavailable`
- Safe action: `build_helper_lane`
- Agent behavior: run the productized helper builder for the requested major.minor lane, export the printed `UC_NATIVE_TOOLCHAIN_<major>_<minor>_BIN`, and rerun support probing.

### UCN1005

Configured native toolchain helper is invalid.

- Category: `toolchain_lane_unavailable`
- Safe action: `rebuild_helper_lane`
- Agent behavior: rebuild the helper or point the environment variable at an executable helper binary.

### UCN1006

Native toolchain helper lane is not productized.

- Category: `toolchain_lane_unsupported`
- Safe action: `manual_legacy_adapter_required`
- Agent behavior: do not run the helper builder for this lane. Keep the workload in the support matrix as `native_unsupported` unless a reviewed compatible helper binary is explicitly supplied or a dedicated compatibility adapter lands.

### UCN2001

Native preflight downgraded to Scarb.

- Category: `native_fallback_preflight_ineligible`
- Safe action: `inspect_native_support_then_retry`
- Agent behavior: run `uc support native --format json`, fix the support issue first, then rerun with `UC_NATIVE_DISALLOW_SCARB_FALLBACK=1` only when native is required.

### UCN2002

Native local build downgraded to Scarb.

- Category: `native_fallback_local_native_error`
- Safe action: `inspect_native_support_then_retry`
- Agent behavior: keep the fallback result, inspect native support and build report diagnostics, then retry native with fallback disallowed only after fixing the native failure.

### UCN2003

Daemon backend downgraded to Scarb.

- Category: `native_fallback_daemon_backend_downgrade`
- Safe action: `inspect_native_support_then_retry`
- Agent behavior: inspect daemon fallback hints and support JSON before rerunning the daemon path.

## Agent Policy

Agents may perform only safe actions by default:

- `build_helper_lane`
- `rebuild_helper_lane`
- `select_matching_toolchain_lane`
- `inspect_native_support_then_retry`
- `use_native_enabled_binary`

Agents must not edit Cairo source, dependency ranges, lockfiles, release metadata, or legacy toolchain adapter code unless the user or calling tool explicitly grants source-edit permission. Diagnostics with `safe_automated_action=manual_legacy_adapter_required` are stop-and-report states, not autonomous fix states.

## Compatibility Notes

Schema version `1` is intentionally explicit rather than sparse:

- `diagnostics` serializes as an array, including `[]` when no diagnostics were emitted.
- `toolchain_found` serializes as `null` when no matching helper/compiler was found.
- `docs_url` is an absolute GitHub URL so agents can dereference it without knowing the repository checkout path.

Consumers should gate on `schema_version` and stable `code`/`category` values instead of relying on omitted optional fields.

## Sources

- AGENTS.md describes repository instructions as a predictable agent context surface: <https://agents.md/>
- MCP tool results support structured content and output schemas: <https://modelcontextprotocol.io/specification/2025-11-25/schema>
- LSP standardizes diagnostics and code actions across tools: <https://microsoft.github.io/language-server-protocol/>
- SARIF 2.1.0 is the standard interchange format for static-analysis results: <https://docs.oasis-open.org/sarif/sarif/v2.1.0/os/sarif-v2.1.0-os.html>
- GitHub code scanning consumes SARIF and requires stable categories for multiple uploads: <https://docs.github.com/en/code-security/how-tos/scan-code-for-vulnerabilities/integrate-with-existing-tools/uploading-a-sarif-file-to-github>

# Agent Diagnostics

Diagnostics are part of the agent contract. They must be stable enough for automation and explicit enough for remediation.

Every diagnostic should include:

- `schema_version`
- `code`
- `category`
- `severity`
- `what_happened`
- `why`
- `how_to_fix`
- `next_commands`
- `safe_automated_action`
- `retryable`
- `fallback_used`
- expected/found state when available
- artifact/log/replay paths when available

## Routing Codes

| Code | Meaning | Agent Behavior |
| --- | --- | --- |
| `UCN1001` | Native support is available. | Continue to planning or build. |
| `UCN1006` | Required helper lane is not productized. | Mark as `native_unsupported` unless a reviewed helper binary is provided. |
| `UCN1100` | Manifest path resolution failed. | Fix the manifest path and retry. |
| `UCN1200` | Manifest could not be read or parsed. | Fix file access or syntax before running build. |
| `UCN2001` | Native preflight selected compatibility fallback. | Report fallback explicitly; do not count as native success. |
| `UCN2002` | Native build downgraded to compatibility fallback. | Preserve fallback state and inspect native diagnostics. |

## Decision Status

Agent-facing support reports use:

- `native_supported`
- `native_unsupported`
- `fallback_likely`
- `build_blocked`

Low-level probe status may still use:

- `supported`
- `unsupported`
- `unavailable`

Agents should prefer `decision_status` when choosing the next action.

## Fallback Contract

Fallback is compatibility behavior.

Required behavior:

- set `fallback_used=true` when fallback happened,
- include why native did not continue,
- include whether fallback succeeded or failed,
- never count fallback as native support,
- keep replay/log paths when available.

## Retryability

`retryable=true` means the same command can plausibly succeed after the reported remediation. It does not mean the agent should blindly retry without changing inputs.

Examples:

- missing manifest path: retryable after path correction,
- invalid manifest syntax: retryable after file edit,
- missing reviewed helper lane: not retryable until a helper is supplied,
- compiler error in project code: retryable after source correction.

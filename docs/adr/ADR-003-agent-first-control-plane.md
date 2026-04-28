# ADR-003: Agent-First Control Plane

## Status

Accepted

## Decision

`uc` is designed first as an agent-facing Cairo project control plane, with compiler/build and package-management subsystems behind that control plane.

## Rationale

- Agents are the primary users of `uc`, not humans typing interactive commands.
- Agent workflows require deterministic preflight, machine-readable planning, explicit fallback state, and replayable failure data.
- Modern package managers and toolchains have converged on lockfile-first planning, explicit fetch phases, shared stores/caches, integrated runtime/toolchain acquisition, and strict offline/frozen modes.
- Treating `uc` as only a build wrapper around Scarb would leave the package, toolchain, and policy control planes outside of `uc`, which is the wrong ownership boundary.

## Consequences

- Stable JSON/schema design becomes core product work.
- First-party project model, resolver/fetch, and toolchain surfaces move earlier in the roadmap.
- Human-readable terminal UX becomes secondary to machine-readable correctness.
- Silent fallback, silent network access, and silent lockfile mutation are considered product defects.

# Launch Prep 2026-04-24

## Current Position

`uc` is now credible as a multi-toolchain native builder for the measured Cairo `2.14` repo set. The strongest current story is:

- all five measured real repos stayed on native with no fallback
- Cairo `2.14` support is productized via a checked-in helper build flow
- native support probing and build failures now emit stable machine-readable diagnostics
- agent-facing diagnostics now include schema version, docs URL, next commands, and safe automated action fields
- benchmark output now surfaces repo-level instability instead of hiding it behind medians

This is not yet a "faster everywhere" launch.

## What We Can Claim In This PR

- helper lanes can be built locally with `./scripts/build_native_toolchain_helper.sh --lane 2.14`
- `./scripts/doctor.sh --uc-bin <path> --manifest-path <Scarb.toml>` detects missing helper lanes before a build
- `uc support native --format json` and `uc build --report-path ...` provide stable diagnostic codes and remediation fields for agents and CI
- `docs/agent/AGENT_DIAGNOSTICS.md` and `docs/agent/schemas/*.schema.json` define the current agent JSON contract
- benchmark output includes support-matrix counts and stability warnings for noisy lanes

## What We Should Not Claim

- do not quote support-matrix counts or per-repo speedup ratios from this document unless the matching benchmark artifact directory is committed or published in a durable location
- do not claim uniform wins on every repo and every lane
- do not claim the cold path is solved
- do not claim there is a proven new compiler optimization in this PR

The local same-window sweep used during development produced useful directional data, but its artifact directory is not versioned with this PR. Publish those metrics only from a committed benchmark artifact or an external immutable run record that includes host, binary, lane, dataset, flags, and sample counts.

## Performance Conclusion

The profiling evidence still points to the same bottleneck:

- cold helper-lane builds remain dominated by `native_frontend_compile`
- no cache/setup-only patch produced a clean material improvement worth merging
- the explored Rayon thread-cap direction was rejected because it regressed real runs

The next performance work should target frontend compile hotspots, starting with the weakest supported repo in the latest sweep.

## Agent-First Launch Minimum

The agent-first launch minimum is tracked in `docs/AGENT_FIRST_LAUNCH_MINIMUM_2026-04-24.md`.
This PR covers the native helper lane, support-matrix reporting, and stable diagnostic contract pieces.
It does not yet cover the future flight recorder, MCP server, SARIF export, or deployed-contract corpus claim.

## Launch Sequence

1. Launch helper-backed Cairo `2.14` support and agent-grade diagnostics as the concrete product improvement.
2. Present the support matrix and benchmark table together, including the instability caveat.
3. Hold broader "faster than Scarb" messaging until the weakest repo lane is improved or explicitly carved out.

## Required Follow-Up

1. Profile `token-factory` warm-noop and cold helper-lane behavior directly.
2. Keep per-repo stability warnings in every public benchmark table.
3. Extend helper lanes beyond Cairo `2.14` only after the next supported-version demand is concrete.

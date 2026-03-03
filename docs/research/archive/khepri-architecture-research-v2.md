# Khepri v2: Architecture Redline + Product Proof Plan

**Date:** 2026-03-03  
**Status:** Revised for technical design review  
**Supersedes:** `khepri-architecture-research.md`

---

## 1. Executive Decision

Khepri is still a strong direction, but the first implementation must be narrower:

- No process-local IDs in RPC (no `CrateId` over the wire).
- No "single global DB" assumption.
- No assumption that Salsa DB can be persisted/restored as-is.
- No LSP/STWO work in MVP.

The first product milestone is to prove that daemonized compile service improves warm build latency **without degrading correctness beyond the defined MVP tolerance**.

---

## 2. Redline Summary From v1

## 2.1 Keep
- Persistent process strategy.
- Shared compiler-state objective.
- Fallback-to-Scarb safety model.
- Phased rollout with CI verification.

## 2.2 Correct

1. RPC contract:
- **v1 problem:** API centered on `Vec<CrateId>`.
- **v2 correction:** API uses stable semantic inputs:
  - workspace root / manifest path
  - package selection
  - profile/features/cfg
  - command kind (`build/check/lint/test/execute/prove`)
  - changed file payloads (optional)

2. Mutation model:
- **v1 problem:** "file overrides are the only mutation needed."
- **v2 correction:** cache/session key must include:
  - project config (crate roots and units)
  - cfg set
  - compiler optimization flags
  - macro/analyzer/inline plugin overrides
  - target kind and target params
  - virtual wrapper-lib injection inputs

3. Session model:
- **v1 problem:** single global `RwLock<RootDatabase>`.
- **v2 correction:** one session DB per normalized key:
  - `(workspace_root, compiler_rev, profile, features, cfg_hash, plugin_hash, target_family)`

4. Parallelism baseline:
- **v1 problem:** implied existing per-unit parallel compilation.
- **v2 correction:** current unit processing is effectively sequential at unit level; this is opportunity, not existing behavior.

5. Scope estimate:
- **v1 problem:** "minimal ~200 line Scarb change."
- **v2 correction:** expect multi-surface integration:
  - compile path
  - lint DB path
  - diagnostics mapping
  - proc-macro/plugin lifecycle
  - artifact registration

6. Persistence expectation:
- **v1 problem:** assumed DB snapshot persisted to disk.
- **v2 correction:** persistence in MVP is artifact-level + session metadata only; in-memory DB is rebuildable.

---

## 3. Product Risk Framing (TPM View)

Top risks, ordered:

1. **Value risk:** daemon does not materially reduce p95 warm rebuild latency.
2. **Correctness risk:** output artifacts/diagnostics diverge from Scarb baseline.
3. **Adoption risk:** integration friction with existing Scarb workflows/plugins is too high.

The first proof must directly burn down risk #1 and #2 together.

---

## 4. The First Thing We Must Prove

## 4.1 Primary hypothesis (H1)

For repeated local development builds in the same workspace, a sessionized Khepri compile service can reduce **p95 warm rebuild latency by >=40%** vs Scarb baseline, while preserving output correctness.

## 4.2 Why this first

- It validates the core business value proposition quickly.
- It avoids early over-investment in LSP/STWO/remote cache.
- It gives a hard go/no-go before broader platform work.

## 4.3 Experiment design (MVP proof)

### Scope
- Commands: `build` only.
- Workspace count: 2 representative projects:
  - `stwo-cairo/stwo_cairo_verifier`
  - one medium Scarb workspace from examples/external set.
- No remote cache.
- No LSP integration.
- No STWO proving cache.

### Test cases
1. Warm no-op rebuild.
2. Single-file edit in one crate.
3. Single-file edit affecting plugin path.
4. Profile switch (`dev` -> custom profile).

### Metrics
- `p50/p95` wall time per case.
- CPU time and max RSS.
- cache/session hit ratio.
- artifact parity:
  - Sierra JSON hash
  - CASM hash
  - diagnostics parity (count + severity + stable location)

### Success criteria
- Performance:
  - `p95` warm rebuild improvement >= 40% on both repos.
- Correctness:
  - zero artifact hash mismatches in test matrix.
  - diagnostics parity >= 99.5% (exact match goal; 99.5% is the temporary MVP tolerance for formatting-only diagnostic drift).
- Reliability:
  - daemon crash fallback to Scarb works 100% in matrix.

### Go/No-Go
- **Go:** all success criteria pass.
- **No-Go:** if performance gain < 25% or correctness mismatches persist after one stabilization cycle.

---

## 5. MVP Architecture (Proof-Oriented)

## 5.1 Components
- `khepri-daemon`
  - session manager
  - compile service
  - local CAS for artifacts
- `khepri-client`
  - called from Scarb integration point
  - fallback wrapper
- `khepri-compat-bridge` in Scarb
  - controlled by opt-in env var/config flag

## 5.2 RPC contract (stable)

```protobuf
message CompileRequest {
  string workspace_root = 1;
  string manifest_path = 2;
  string profile = 3;
  repeated string packages = 4;
  repeated string features = 5;
  bool no_default_features = 6;
  map<string, string> env = 7;
  repeated FilePatch changed_files = 8;
}

message CompileResponse {
  repeated Artifact artifacts = 1;
  repeated Diagnostic diagnostics = 2;
  SessionStats stats = 3;
}
```

No compiler-internal IDs cross process boundaries.

## 5.3 Session key

```text
session_key = hash(
  workspace_root,
  compiler_version,
  profile,
  feature_set,
  cfg_set,
  plugin_suite_signature,
  target_kind_family
)
```

## 5.4 Correctness guardrail

Dual-run mode in CI:
- Run Scarb and Khepri for same request.
- Compare hashes and diagnostics.
- Fail fast on mismatch.

---

## 6. Updated Roadmap

## Phase A (2-3 weeks): Proof Harness
- Reproducible benchmark harness (cold/warm/edit).
- Artifact and diagnostics comparator.
- CI job producing trend reports.

## Phase B (4-6 weeks): Compile Daemon MVP
- Sessionized in-memory DB lifecycle.
- Stable compile RPC contract.
- Scarb fallback bridge.
- Dual-run verification mode.

## Phase C (after proof passes): Expand scope
- `check` and `lint`.
- Remote cache.
- Optional LSP proxy.
- Optional STWO cache pipeline.

---

## 7. Integration Guidance for Scarb

- Treat Khepri as acceleration backend, not platform.
- Keep current CLI and manifest semantics unchanged.
- Gate activation behind explicit opt-in:
  - `[tool.khepri].enabled = true` or `SCARB_KHEPRI=1`.
- Preserve hard fallback:
  - if daemon unavailable/error => current Scarb path.

---

## 8. Open Design Questions (v2)

1. Should plugin compilation stay in Scarb with binary handles passed to daemon, or move fully into daemon session lifecycle?
2. Should sessions be per-workspace only, or per-workspace+profile/features by default?
3. What diagnostics equivalence level is acceptable for rollout: strict byte-equal vs semantic-equal?
4. Should dual-run parity checks be mandatory in CI until beta exit?

---

## 9. Immediate Next Actions

1. Freeze baseline benchmark matrix and publish first report.
2. Implement compile RPC with stable semantic input contract.
3. Add dual-run comparator against Scarb artifacts.
4. Run go/no-go gate on H1 before adding LSP/STWO scope.

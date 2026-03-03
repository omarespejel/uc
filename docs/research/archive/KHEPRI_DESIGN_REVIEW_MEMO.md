# Khepri Build Acceleration: Design Review Memo

> Historical archive note: this memo is retained for context and has been superseded by the `uc` planning docs in `docs/`.

**Date:** 2026-03-03  
**Owner:** Compiler Acceleration Initiative  
**Decision Type:** Architecture + Delivery Scope

## Problem

Current Cairo build workflows pay repeated warm-path latency because compiler state is not reused across invocations. This creates:
- slower local edit-build loops;
- duplicated work across commands (`build`/`check`/`lint`);
- higher CI time for repeated rebuild scenarios.

Local measurements show meaningful warm-path opportunity in representative repos:
- large gap between cold and warm command runtimes for `test`;
- non-trivial warm latency for `lint`;
- metadata/dependency path overhead in online mode versus cached/offline.

## Decision

Build a **sessionized compile daemon** as an acceleration backend for Scarb, while preserving Scarb CLI/manifests/workspace semantics.

Scope for initial delivery:
- optimize `build` only;
- keep hard fallback to current Scarb path;
- prove value with strict latency and correctness gates before expanding scope.

What this is not:
- not a Scarb rewrite;
- not immediate LSP unification;
- not immediate STWO trace/proof caching.

## Alternatives Considered

1. **Rewrite/modernize Scarb fully**
- Pros: maximum architectural freedom.
- Cons: high parity risk, long timeline, ecosystem migration burden.
- Decision: rejected for initial delivery.

2. **Incremental improvements inside Scarb only**
- Pros: lower integration friction.
- Cons: does not guarantee persistent state reuse model quickly.
- Decision: partial strategy only, not primary.

3. **Daemon backend with Scarb compatibility (selected)**
- Pros: fast value path, bounded risk, no workflow breakage, measurable.
- Cons: RPC/session complexity, dual-path maintenance during rollout.
- Decision: selected.

## Metrics (Success Criteria)

Primary hypothesis:
- `p95` warm rebuild latency improves by **>= 40%** versus Scarb baseline on target repos.

Correctness criteria:
- zero Sierra/CASM hash mismatches in test matrix;
- diagnostics parity target: exact match, minimum acceptable >= 99.5%.

Reliability criteria:
- fallback success rate: 100% in validation matrix;
- no blocker regressions in CI compatibility lane.

Evaluation matrix:
- warm no-op rebuild;
- single-file edit rebuild;
- profile/feature variant rebuild;
- daemon unavailable/crash fallback behavior.

## Risks and Mitigations

1. **Correctness drift**
- Mitigation: dual-run CI comparator (Scarb vs daemon) with artifact+diagnostic diff gating.

2. **Performance gains below threshold**
- Mitigation: early hard Go/No-Go gate after MVP benchmark cycle.

3. **Integration complexity underestimated**
- Mitigation: narrow MVP (`build` only), phased expansion.

4. **Plugin/proc-macro edge cases**
- Mitigation: unsupported-path fallback to current Scarb compile path.

5. **State/session mismatch bugs**
- Mitigation: strict session keying by workspace + compiler version + profile/features/cfg/plugin signature + target family.

## Approval Checklist

- [ ] Approve build-only MVP scope.
- [ ] Approve hard fallback requirement as non-negotiable.
- [ ] Approve dual-run CI parity gate before wider rollout.
- [ ] Approve Go/No-Go threshold: `p95` warm improvement >= 40%, correctness criteria met.
- [ ] Approve deferring LSP/STWO/remote-cache scope until MVP gate passes.

## Implementation Milestones

1. Baseline harness + comparator (perf + parity).
2. Compile daemon MVP + Scarb bridge + fallback.
3. Gate review with benchmark and parity evidence.
4. Stabilization and opt-in CI rollout.

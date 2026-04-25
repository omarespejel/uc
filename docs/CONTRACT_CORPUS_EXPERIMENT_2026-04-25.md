# Contract Corpus Experiment Loop

This is the launch-readiness loop for expanding from bounded proof points to a 20+ contract corpus while improving developer experience and native performance.

## Goal

Build evidence that is useful for agents and defensible for launch:

- newest Cairo versions for speed evidence,
- older Cairo lanes for coverage evidence,
- explicit support matrix for every contract,
- opportunity log for every support gap, fallback, unstable lane, diagnostic gap, and phase hotspot,
- no public speed claim unless the benchmark artifact and claim guard support it.

## Ten-Step Loop

1. Select at least 20 real contracts or contract packages, biased toward the newest Cairo versions currently used by active Starknet projects.
2. Keep a reviewed source inventory as the durable input; do not hand-author generated corpus artifacts.
3. Prefer deployed-contract rows when making deployed-contract claims; use declared-class/sample rows only for support evidence.
4. Run source-index and corpus generation from the reviewed inventory.
5. Run corpus evidence with pinned host settings, explicit `uc` binary, explicit sample counts, and same-window Scarb/uc comparisons.
6. Generate the corpus opportunity summary:

   ```bash
   ./benchmarks/scripts/summarize_corpus_opportunities.py \
     --benchmark-json /abs/path/to/deployed-contract-corpus-bench.json \
     --out-json /abs/path/to/corpus-opportunities.json \
     --out-md /abs/path/to/corpus-opportunities.md
   ```

7. Fix blockers before chasing speed: native unsupported, fallback used, build failed, and failed benchmark lanes come first.
8. Only profile speed on native-supported cases. Prioritize `native_frontend_compile_ms`, semantic/diagnostic hot paths, and unstable benchmark lanes before adding more cache glue.
9. Re-run the affected cases in the same host window after each material change. Do not compare old-window numbers against new-window numbers for launch copy.
10. Promote results into launch evidence only when claim guards are true, no relevant lanes are unstable, diagnostics are structured enough for agent remediation, and the PR review loop is quiet.

## What To Maximize

The corpus should maximize current developer pain, not just easy wins:

- support across real Cairo versions, including older helper lanes when active projects require them,
- newest Cairo speed evidence for launch relevance,
- structured diagnostics that agents can parse without prose inference,
- replayable failure paths for every unsupported or failed case,
- phase telemetry that names the next engineering target.

## Opportunity Codes

The summary script emits stable `UCO*` codes:

| Code | Meaning | Default action |
|---|---|---|
| `UCO1001` | Native support gap | Add/fix toolchain lane before benchmarking. |
| `UCO1002` | Fallback path used | Treat as unsupported for launch speed claims; fix the native failure class. |
| `UCO1003` | Auto-build classification failed | Capture replay bundle and add a regression fixture. |
| `UCO2001` | Benchmark lane failed | Inspect the failed lane log before quoting the case. |
| `UCO2002` | Benchmark lane unstable | Rerun same-window after reducing noise; do not use for headline speed. |
| `UCO3001` | Native frontend compile dominates | Profile semantic/native frontend work for this case. |
| `UCO3002` | CASM generation is material | Inspect CASM generation before unrelated optimizations. |
| `UCO3003` | Artifact write is material | Inspect artifact output and filesystem overhead. |
| `UCO3004` | Fingerprinting is material | Inspect project scan/fingerprint costs. |
| `UCO3005` | UC cold build slower than Scarb | Profile before shipping any perf claim. |
| `UCO3006` | UC cold speedup is weak | Use as an optimization target, not launch copy. |
| `UCO3007` | Native session prepare is material | Inspect helper/session setup overhead. |
| `UCO4001` | Launch evidence candidate | Keep only if benchmark claim guards also pass. |
| `UCO4002` | Strong warm no-op speedup | Quote only with sample, lane, host, and stability caveats. |
| `UCO5001` | Diagnostic is not agent-grade | Extend missing or generic diagnostic detail before automated remediation. |

## Launch Boundary

The 20-contract corpus is an experiment harness first. It becomes launch evidence only after support and claim guards are true for the exact artifact being quoted.

## 2026-04-26 Local 20-Case Sweep

This sweep is diagnostic evidence, not launch copy. It used `--runs 1`,
`--cold-runs 1`, and no pinned-host strict stability window. The purpose was to
expand coverage, surface blockers, and produce an agent-readable backlog.

Artifacts:

- Evidence root: `<abs/path>/uc-20-contract-experiment-20260425`
- Cases file: `<evidence-root>/cases.tsv`
- Benchmark JSON: `<evidence-root>/results/real-repo-bench-20260426-011206.json`
- Benchmark Markdown: `<evidence-root>/results/real-repo-bench-20260426-011206.md`
- Opportunity JSON: `<evidence-root>/corpus-opportunities.json`
- Opportunity Markdown: `<evidence-root>/corpus-opportunities.md`
- Cairo 2.14 helper: `${UC_NATIVE_TOOLCHAIN_2_14_BIN}`, produced by
  `./scripts/build_native_toolchain_helper.sh --lane 2.14`

Support matrix:

| Classification | Count |
|---|---:|
| `native_supported` | 12 |
| `native_unsupported` | 6 |
| `fallback_used` | 0 |
| `build_failed` | 2 |

Benchmark status:

| Status | Count |
|---|---:|
| `ok` | 12 |
| `skipped` | 8 |

Opportunity counts after applying the generic-diagnostic quality rule:

| Code | Count | Meaning |
|---|---:|---|
| `UCO1001` | 6 | Native support gap. |
| `UCO1003` | 2 | Auto-build classification failed. |
| `UCO3001` | 12 | Native frontend compile dominates. |
| `UCO3006` | 2 | Cold speedup is weak. |
| `UCO4001` | 12 | Bounded launch-evidence candidate after stricter validation. |
| `UCO4002` | 2 | Strong warm no-op speedup. |
| `UCO5001` | 4 | Diagnostic is not agent-grade. |

The two `build_failed` rows were `accounts_workshop` and
`starknetpy_contracts`. Both selected the Cairo 2.14 external helper and failed
inside a cached `cairo-contracts` dependency with a Cairo diagnostic shaped like
`error[E0002]: Method span could not be called on type core::array::Span::<core::felt252>`.
The generated build reports carried structured `UCN2002` diagnostics, but the
`what_happened` and `why` fields were only `Compilation failed.`. That is too
generic for an agent to remediate, so the opportunity summarizer now treats
generic diagnostic text as `UCO5001` even when all required fields are present.

Low-sample same-window observed ratios for native-supported rows:

- Benchmark lane: `benchmarks/scripts/run_real_repo_benchmarks.sh` real-repo
  lane, comparing `scarb build` against `uc build --engine uc --daemon-mode off
  --offline`.
- Stages: `build.cold` observed value and `build.warm_noop` observed value
  from a single-sample diagnostic run.
- Sample settings: `--runs 1`, `--cold-runs 1`,
  `--warm-settle-seconds 0`, `--timeout-secs 180`.
- Host condition: local ad hoc macOS developer workstation run; no pinned CPU,
  no strict host-noise preflight, and no recorded hardware claim metadata.
- Claim-guard status: no deployed-contract `claim_guard` was produced for this
  real-repo diagnostic sweep. The ratios below are backlog triage signals only,
  not launch speed claims.

| Tag | Cold observed ratio (single-sample) | Warm no-op observed ratio (single-sample) | Launch-use status |
|---|---:|---:|---|
| `braavos_account` | 1.196x | 34.334x | Diagnostic only; cold speedup weak. |
| `monero_atomic_swap` | 1.211x | 135.222x | Diagnostic only; cold speedup weak. |
| `agentic_session` | 1.821x | 4.819x | Needs strict rerun. |
| `agentic_agent` | 2.089x | 4.530x | Needs strict rerun. |
| `agentic_erc8004` | 1.789x | 8.791x | Needs strict rerun. |
| `agentic_huginn` | 1.562x | 2.102x | Needs strict rerun. |
| `book_ownable` | 1.290x | 0.544x | Warm regression target. |
| `book_vote_contracts` | 1.693x | 6.109x | Needs strict rerun. |
| `book_ownable_components` | 1.918x | 6.284x | Needs strict rerun. |
| `token_factory` | 1.287x | 0.977x | Warm parity/regression target. |
| `zcash_relay` | 1.747x | 1.058x | Needs strict rerun. |
| `glint_contracts` | 2.132x | 3.123x | Needs strict rerun. |

Next blockers from this sweep:

1. Add or select helper lanes for the remaining unsupported Cairo versions
   before counting those rows as solved.
2. Turn generic native compile failures into agent-grade diagnostics with the
   original Cairo error code, source span, expected/found toolchain, retryability,
   fallback state, and replay/log path.
3. Rerun only native-supported cases under strict same-window sample settings
   before using any speed ratio externally.
4. Profile `native_frontend_compile_ms` on supported repos first; this was the
   dominant hotspot across all 12 supported rows.

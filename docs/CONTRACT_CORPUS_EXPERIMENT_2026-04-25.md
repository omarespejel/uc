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
| `UCO5001` | Diagnostic is not agent-grade | Extend diagnostic fields before automated remediation. |

## Launch Boundary

The 20-contract corpus is an experiment harness first. It becomes launch evidence only after support and claim guards are true for the exact artifact being quoted.

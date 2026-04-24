# uc Native Debug

Use this skill when native compile is slow, stalls, or diverges from Scarb.

## Debug Order
1. Run with `--engine uc --daemon-mode off --offline` first.
2. Enable phase timings: `UC_PHASE_TIMING=1`.
3. Enable live native progress logs: `UC_NATIVE_PROGRESS=1`.
4. Add heartbeats for long frontend work: `UC_NATIVE_PROGRESS_HEARTBEAT_SECS=5`.
5. If contract compilation is the suspect, force one-contract batches: `UC_NATIVE_PROGRESS_COMPILE_BATCH_SIZE=1`.
6. Enable debug logs: `RUST_LOG=uc=debug`.
7. Reproduce on a single manifest before widening to benchmark harnesses.
8. If the issue is in native compile session state, inspect `crates/uc-cli/src/main.rs` and `crates/uc-cli/src/fingerprint.rs`.
9. If the issue is in contract compilation or semantics, isolate a specific manifest and contract set before changing cache logic.

## Do Not
- Claim a speedup from a noisy host.
- Change benchmark thresholds to hide regressions.
- Relax invalidation logic just to preserve a cache hit.

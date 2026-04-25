# Agent-First Compiler Direction

`uc` should be designed for agents first and humans second. The practical meaning is simple: every important compiler state must be structured, replayable, policy-aware, and safe to automate.

## Why This Is Different

A human-first compiler can print a good paragraph and assume a developer will infer the next step. An agent-first compiler cannot rely on inference. It must expose the state that lets an agent decide whether to fix, retry, fall back, benchmark, or stop.

The launch wedge is not just speed. The launch wedge is:

> A Cairo compiler agents can operate reliably: structured diagnostics, native multi-toolchain support, reproducible failure bundles, and benchmark claims anyone can verify.

The longer-term product boundary is wider than `scarb build` acceleration. `uc` should become the agent-first project tool that owns manifest import, lockfile interpretation, metadata, resolver/source-cache behavior, build/test/check/lint/fmt commands, diagnostics, and failure replay. Scarb remains a compatibility bridge until parity gates pass.

## Product Principles

1. Do not make agents parse prose.
2. Every failure gets a stable code.
3. Every diagnostic says what happened, why, how to fix it, whether fallback happened, and what command to run next.
4. Every fallback is visible in JSON and benchmark reports.
5. Every benchmark claim must carry lane, manifest source, host, sample count, and support classification.
6. Safe automated actions are explicit and reversible by default.
7. Source edits require explicit permission.
8. Repo policy comes from checked-in files like `AGENTS.md`, `.codex/START_HERE.md`, and `docs/agent/*`.
9. Scarb compatibility is a migration path, not the final control plane.

## Launch-Minimum Agent Surfaces

Already in this PR or required before launch:

- `uc support native --format json` emits stable support reports.
- `uc build --json` and `--report-path` carry build diagnostics.
- `uc build --record-failure <path>` writes a redacted, replay-safe failure bundle on build errors.
- `uc replay <bundle>` reads that bundle and is dry-run by default.
- `uc agent eval --manifest-path <Scarb.toml>` returns a decision agents can act on before compiling.
- `uc agent safe-action <action>` exposes dry-run-first remediation commands.
- `uc mcp serve` emits the read-only command/resource catalog for MCP adapters.
- Native support reports include toolchain selection metadata.
- Diagnostics include schema version, docs URL, next commands, safe automated action, retryability, fallback status, expected toolchain, and found toolchain.
- Real-repo benchmarks include native support classification and fallback status.
- `scripts/doctor.sh --uc-bin <path> --manifest-path <Scarb.toml>` probes support before measurement.

### Manual / Debug Surfaces

- Agents should prefer `uc agent safe-action build-helper-lane --lane 2.14` for older-Cairo helper setup.
- `./scripts/build_native_toolchain_helper.sh --lane 2.14` remains available for manual debugging when the agent command surface is not enough.

## Remaining PRs

1. `agent-flight-recorder-hardening`
   - Extend the failure bundle with timing spans and source graph hashes.
   - Add bundle schema validation in the replay path.

2. `uc-mcp-stdio`
   - Wrap the read-only catalog in a real stdio MCP JSON-RPC server.
   - Keep mutable actions out of MCP until permission gates are explicit.

3. `agent-eval-fixtures`
   - Check in fixture manifests for missing helper lanes, unsupported manifests, fallback activation, stale cache, and benchmark unsupported cases.
   - Add real `monero` and `braavos` fixture runners, not just required-fixture markers.

4. `sarif-and-lsp`
   - Map diagnostics to SARIF for GitHub/code-scanning ingestion.
   - Reuse the same stable codes for LSP diagnostics and code actions.

5. `safe-source-edits`
   - Keep source edits behind an explicit `--allow-source-edits` gate.
   - Require failure-bundle replay evidence before allowing source-modifying actions.

6. `uc-project-inspect`
   - Add read-only Scarb manifest/lockfile import as `uc project inspect --manifest-path <Scarb.toml> --format json`.
   - Emit a stable project-model schema for agents before replacing metadata/resolver behavior.

7. `metadata-from-project-model`
   - Move `uc metadata` onto the `uc` project model behind an explicit gate.
   - Keep Scarb as comparator until metadata parity passes.

## MCP Shape

Read-only MCP tools should eventually expose:

- `uc.doctor`
- `uc.support_native`
- `uc.explain_diagnostic`
- `uc.select_toolchain`
- `uc.benchmark_report`
- `uc.profile_native_frontend`

MCP resources should expose:

- `uc://diagnostics/catalog`
- `uc://support/native-matrix/latest`
- `uc://benchmarks/latest`
- `uc://toolchains/native`
- `uc://repo/policy`

## Human Workflow

Humans can keep using terminal text:

```sh
uc support native --manifest-path Scarb.toml
uc build --engine uc --daemon-mode off
```

When debugging or sharing evidence, humans should switch to JSON:

```sh
uc support native --manifest-path Scarb.toml --format json | jq
uc build --engine uc --daemon-mode off --json | jq
```

## Agent Workflow

Agents should start with support probing, not build-and-guess:

```sh
uc support native --manifest-path Scarb.toml --format json
uc agent eval --manifest-path Scarb.toml
./scripts/doctor.sh --uc-bin ./target/release/uc --manifest-path /abs/path/to/Scarb.toml
```

After the project-inspect surface exists, agents should run it before build or metadata work:

```sh
uc project inspect --manifest-path Scarb.toml --format json
```

If the diagnostic says `safe_automated_action=build_helper_lane`, the agent may run:

```sh
uc agent safe-action build-helper-lane --lane 2.14
uc agent safe-action build-helper-lane --lane 2.14 --execute
```

Then it should export the printed helper env var and rerun support probing before benchmarking.

When a build fails, agents should preserve a replayable artifact:

```sh
failure_bundle="$(mktemp -t uc-failure.XXXXXX.json)"
uc build --engine uc --daemon-mode off --manifest-path Scarb.toml --record-failure "$failure_bundle"
uc replay "$failure_bundle"
```

Use `uc replay "$failure_bundle" --execute` only when the agent is allowed to rerun the recorded build command. Replay reports still emit structured JSON if the command cannot spawn or exceeds capture limits.

## Sources

- AGENTS.md: <https://agents.md/>
- MCP schema and structured tool results: <https://modelcontextprotocol.io/specification/2025-11-25/schema>
- Language Server Protocol overview: <https://microsoft.github.io/language-server-protocol/>
- SARIF 2.1.0 specification: <https://docs.oasis-open.org/sarif/sarif/v2.1.0/os/sarif-v2.1.0-os.html>
- GitHub SARIF upload behavior: <https://docs.github.com/en/code-security/how-tos/scan-code-for-vulnerabilities/integrate-with-existing-tools/uploading-a-sarif-file-to-github>

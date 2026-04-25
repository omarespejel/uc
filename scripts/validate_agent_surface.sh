#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

required_files=(
  AGENTS.md
  .codex/START_HERE.md
  docs/agent/README.md
  docs/agent/PR_BOT_POLICY.md
  docs/agent/REPO_MAP.md
  docs/agent/AGENT_FIRST_COMPILER.md
  docs/agent/AGENT_DIAGNOSTICS.md
  docs/agent/AGENT_QUICKSTART.md
  docs/agent/HUMAN_QUICKSTART.md
  docs/AGENT_FIRST_LAUNCH_MINIMUM_2026-04-24.md
  docs/SCARB_SUNSET_STRATEGY.md
  docs/agent/schemas/native-diagnostic.schema.json
  docs/agent/schemas/native-support-report.schema.json
  docs/agent/schemas/build-report.schema.json
  docs/agent/schemas/agent-eval-report.schema.json
  docs/agent/schemas/failure-bundle.schema.json
  docs/agent/schemas/replay-report.schema.json
  docs/agent/schemas/mcp-catalog.schema.json
  docs/agent/schemas/safe-action-report.schema.json
  docs/agent/schemas/benchmark-report.schema.json
  .coderabbit.yaml
  .pr_agent.toml
  best_practices.md
  pr_compliance_checklist.yaml
  scripts/install_git_hooks.sh
  scripts/local_ci_gate.sh
  scripts/build_native_toolchain_helper.sh
  scripts/tests/local_ci_gate_test.sh
  scripts/tests/doctor_test.sh
  scripts/tests/build_native_toolchain_helper_test.sh
  .githooks/pre-push
  .claude/skills/uc-pr-review-loop/SKILL.md
  .claude/skills/uc-native-debug/SKILL.md
  .claude/skills/uc-benchmark-gate/SKILL.md
)

for path in "${required_files[@]}"; do
  [[ -f "$path" ]] || { echo "missing required file: $path" >&2; exit 1; }
done

if command -v mktemp >/dev/null 2>&1; then
  tmp_map="$(mktemp)"
else
  echo "mktemp not found; please install coreutils or provide mktemp in PATH" >&2
  exit 1
fi
trap 'rm -f "$tmp_map"' EXIT
./scripts/refresh_repo_map.sh "$tmp_map" >/dev/null
if ! diff -u "$tmp_map" docs/agent/REPO_MAP.md; then
  echo "repo map is stale; run make agent-map" >&2
  exit 1
fi
trap - EXIT
rm -f "$tmp_map"

echo "agent surface validated"

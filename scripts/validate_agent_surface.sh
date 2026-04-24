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
  .coderabbit.yaml
  .pr_agent.toml
  best_practices.md
  pr_compliance_checklist.yaml
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
  tmp_map="$(/usr/bin/mktemp)"
fi
./scripts/refresh_repo_map.sh "$tmp_map" >/dev/null
if ! diff -u "$tmp_map" docs/agent/REPO_MAP.md; then
  echo "repo map is stale; run make agent-map" >&2
  rm -f "$tmp_map"
  exit 1
fi
rm -f "$tmp_map"

echo "agent surface validated"

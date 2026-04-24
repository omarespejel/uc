#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

failures=0

check_required() {
  local cmd="$1"
  if command -v "$cmd" >/dev/null 2>&1; then
    printf '[ok] %s -> %s\n' "$cmd" "$(command -v "$cmd")"
  else
    printf '[missing] %s\n' "$cmd" >&2
    failures=$((failures + 1))
  fi
}

check_optional() {
  local cmd="$1"
  if command -v "$cmd" >/dev/null 2>&1; then
    printf '[ok] %s -> %s\n' "$cmd" "$(command -v "$cmd")"
  else
    printf '[warn] optional command missing: %s\n' "$cmd"
  fi
}

printf 'uc doctor\n'
printf 'repo: %s\n' "$ROOT"

for cmd in git cargo rustc rg jq scarb; do
  check_required "$cmd"
done
check_optional gh

for path in AGENTS.md .codex/START_HERE.md .coderabbit.yaml .pr_agent.toml best_practices.md pr_compliance_checklist.yaml docs/agent/PR_BOT_POLICY.md docs/agent/REPO_MAP.md scripts/install_git_hooks.sh scripts/local_ci_gate.sh scripts/tests/local_ci_gate_test.sh .githooks/pre-push; do
  if [[ -f "$path" ]]; then
    printf '[ok] file %s\n' "$path"
  else
    printf '[missing] file %s\n' "$path" >&2
    failures=$((failures + 1))
  fi
done

hooks_path="$(git config --get core.hooksPath || true)"
if [[ "$hooks_path" == ".githooks" || "$hooks_path" == "$ROOT/.githooks" ]]; then
  printf '[ok] git core.hooksPath=%s\n' "$hooks_path"
else
  printf '[missing] git core.hooksPath is not set to .githooks (run: make install-hooks)\n' >&2
  failures=$((failures + 1))
fi

if [[ -x .githooks/pre-push ]]; then
  printf '[ok] executable hook .githooks/pre-push\n'
else
  printf '[missing] executable hook .githooks/pre-push\n' >&2
  failures=$((failures + 1))
fi

if command -v cargo >/dev/null 2>&1; then
  printf 'cargo: %s\n' "$(cargo --version)"
fi
if command -v rustc >/dev/null 2>&1; then
  printf 'rustc: %s\n' "$(rustc --version)"
fi
if command -v scarb >/dev/null 2>&1; then
  printf 'scarb: %s\n' "$(scarb --version | head -n 1)"
fi
if [[ -n "${UC_NATIVE_CORELIB_SRC:-}" ]]; then
  if [[ -d "${UC_NATIVE_CORELIB_SRC}" ]]; then
    printf '[ok] UC_NATIVE_CORELIB_SRC=%s\n' "$UC_NATIVE_CORELIB_SRC"
  else
    printf '[missing] UC_NATIVE_CORELIB_SRC path does not exist: %s\n' "$UC_NATIVE_CORELIB_SRC" >&2
    failures=$((failures + 1))
  fi
else
  printf '[info] UC_NATIVE_CORELIB_SRC not set; runtime auto-discovery will be used if supported\n'
fi

if (( failures > 0 )); then
  printf 'doctor failed: %d issue(s)\n' "$failures" >&2
  exit 1
fi

printf 'doctor passed\n'

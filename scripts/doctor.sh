#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

failures=0
UC_BIN="${UC_DOCTOR_UC_BIN:-$ROOT/target/release/uc}"
declare -a MANIFEST_PATHS=()

usage() {
  cat <<'USAGE'
Usage:
  doctor.sh [--uc-bin /abs/path/to/uc] [--manifest-path /abs/path/to/Scarb.toml ...]
USAGE
}

require_option_value() {
  local flag="$1"
  local value="${2-}"
  if [[ -z "$value" || "$value" == -* ]]; then
    echo "Missing value for $flag" >&2
    usage >&2
    exit 2
  fi
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --uc-bin)
      require_option_value "$1" "${2-}"
      UC_BIN="$2"
      shift 2
      ;;
    --manifest-path)
      require_option_value "$1" "${2-}"
      MANIFEST_PATHS+=("$2")
      shift 2
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

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

check_helper_env_vars() {
  local found=0
  while IFS='=' read -r name value; do
    [[ "$name" =~ ^UC_NATIVE_TOOLCHAIN_[0-9]+_[0-9]+_BIN$ ]] || continue
    found=1
    if [[ -x "$value" && -f "$value" ]]; then
      printf '[ok] %s=%s\n' "$name" "$value"
    else
      printf '[missing] %s points to a non-executable helper: %s\n' "$name" "$value" >&2
      failures=$((failures + 1))
    fi
  done < <(env | sort)

  if [[ "$found" -eq 0 ]]; then
    printf '[info] no UC_NATIVE_TOOLCHAIN_<major>_<minor>_BIN helper lanes configured\n'
    printf '[info] build one with ./scripts/build_native_toolchain_helper.sh --lane 2.14 when older Cairo repos need it\n'
  fi
}

probe_manifest_native_support() {
  local manifest_path="$1"
  local report_json
  if ! command -v jq >/dev/null 2>&1; then
    printf '[missing] jq is required for manifest probe: %s\n' "$manifest_path" >&2
    failures=$((failures + 1))
    return
  fi
  if [[ ! -x "$UC_BIN" ]]; then
    printf '[missing] uc binary is missing or not executable for manifest probe: %s\n' "$UC_BIN" >&2
    failures=$((failures + 1))
    return
  fi
  if [[ ! -f "$manifest_path" ]]; then
    printf '[missing] manifest path does not exist: %s\n' "$manifest_path" >&2
    failures=$((failures + 1))
    return
  fi
  if ! report_json="$("$UC_BIN" support native --manifest-path "$manifest_path" --format json)"; then
    printf '[missing] native support probe failed for %s via %s\n' "$manifest_path" "$UC_BIN" >&2
    failures=$((failures + 1))
    return
  fi

  local supported issue_code reason
  supported="$(jq -r '.supported' <<<"$report_json")"
  issue_code="$(jq -r '.diagnostics[0].code // .issue_kind // "none"' <<<"$report_json")"
  reason="$(jq -r '.reason // "no reason provided"' <<<"$report_json")"
  if [[ "$supported" == "true" ]]; then
    printf '[ok] native support %s\n' "$manifest_path"
    return
  fi

  local issue_kind
  issue_kind="$(jq -r '.issue_kind // "unknown"' <<<"$report_json")"
  case "$issue_kind" in
    missing_toolchain_helper|invalid_toolchain_helper)
      printf '[missing] native support %s -> %s (%s)\n' "$manifest_path" "$issue_code" "$reason" >&2
      failures=$((failures + 1))
      ;;
    *)
      printf '[warn] native support %s -> %s (%s)\n' "$manifest_path" "$issue_code" "$reason"
      ;;
  esac
}

printf 'uc doctor\n'
printf 'repo: %s\n' "$ROOT"

for cmd in git cargo rustc rg jq scarb python3; do
  check_required "$cmd"
done
check_optional gh

for path in AGENTS.md .codex/START_HERE.md .coderabbit.yaml .pr_agent.toml best_practices.md pr_compliance_checklist.yaml docs/agent/PR_BOT_POLICY.md docs/agent/REPO_MAP.md docs/NATIVE_TOOLCHAIN_HELPERS.md scripts/install_git_hooks.sh scripts/local_ci_gate.sh scripts/build_native_toolchain_helper.sh scripts/tests/local_ci_gate_test.sh scripts/tests/doctor_test.sh scripts/tests/build_native_toolchain_helper_test.sh .githooks/pre-push; do
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
if command -v python3 >/dev/null 2>&1; then
  if python3 - <<'PY' >/dev/null 2>&1
import sys, tomllib
if sys.version_info < (3, 11):
    raise SystemExit(1)
PY
  then
    printf '[ok] python3 tomllib support\n'
  else
    printf '[missing] python3 >= 3.11 with tomllib is required for native helper builds\n' >&2
    failures=$((failures + 1))
  fi
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

check_helper_env_vars

for manifest_path in "${MANIFEST_PATHS[@]}"; do
  probe_manifest_native_support "$manifest_path"
done

if (( failures > 0 )); then
  printf 'doctor failed: %d issue(s)\n' "$failures" >&2
  exit 1
fi

printf 'doctor passed\n'

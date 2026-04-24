#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
ROOT_DIR="$(git -C "$SCRIPT_DIR/../.." rev-parse --show-toplevel 2>/dev/null || (cd "$SCRIPT_DIR/../.." && pwd -P))"
LIB_PATH="$SCRIPT_DIR/lib/native_ci_gate.sh"

# shellcheck source=/dev/null
source "$LIB_PATH"

UC_BIN="${UC_BIN:-$ROOT_DIR/target/release/uc}"
RESULTS_DIR="$ROOT_DIR/benchmarks/results"
declare -a STRICT_CASE_MANIFESTS=()
declare -a STRICT_CASE_TAGS=()
declare -a BACKEND_CASE_MANIFESTS=()
declare -a BACKEND_CASE_TAGS=()
declare -a BACKEND_CASE_ALLOWED=()
declare -A PREFETCHED_MANIFESTS=()

usage() {
  cat <<'USAGE'
Usage:
  run_native_real_repo_smoke.sh [--uc-bin /abs/path/to/uc] [--results-dir /abs/path]
    [--strict-case <manifest-path> <tag> ...]
    [--backend-case <manifest-path> <tag> <allowed-backends-csv> ...]
USAGE
}

require_option_value() {
  local flag="$1"
  local value="${2-}"
  if [[ -z "$value" || "$value" == --* ]]; then
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
    --results-dir)
      require_option_value "$1" "${2-}"
      RESULTS_DIR="$2"
      shift 2
      ;;
    --strict-case)
      if [[ $# -lt 3 ]]; then
        usage >&2
        exit 2
      fi
      STRICT_CASE_MANIFESTS+=("$2")
      STRICT_CASE_TAGS+=("$3")
      shift 3
      ;;
    --backend-case)
      if [[ $# -lt 4 ]]; then
        usage >&2
        exit 2
      fi
      BACKEND_CASE_MANIFESTS+=("$2")
      BACKEND_CASE_TAGS+=("$3")
      BACKEND_CASE_ALLOWED+=("$4")
      shift 4
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

if [[ "${#STRICT_CASE_MANIFESTS[@]}" -eq 0 && "${#BACKEND_CASE_MANIFESTS[@]}" -eq 0 ]]; then
  echo "run_native_real_repo_smoke.sh requires at least one --strict-case or --backend-case" >&2
  usage >&2
  exit 2
fi

if [[ ! -x "$UC_BIN" ]]; then
  echo "UC binary is missing or not executable: $UC_BIN" >&2
  exit 1
fi

mkdir -p "$RESULTS_DIR"

prefetch_manifest_dependencies() {
  local manifest_path="$1"
  if [[ -n "${PREFETCHED_MANIFESTS[$manifest_path]:-}" ]]; then
    return
  fi
  if ! command -v scarb >/dev/null 2>&1; then
    echo "scarb is required to prefetch dependencies for offline real-repo smoke runs" >&2
    exit 1
  fi
  scarb fetch --manifest-path "$manifest_path" >/dev/null
  PREFETCHED_MANIFESTS["$manifest_path"]=1
}

run_case() {
  local manifest_path="$1"
  local tag="$2"
  local allowed_backends="$3"
  local report_path="$RESULTS_DIR/native-real-${tag}.json"
  local log_path="$RESULTS_DIR/native-real-${tag}.log"

  prefetch_manifest_dependencies "$manifest_path"
  "$UC_BIN" build \
    --engine uc \
    --daemon-mode off \
    --offline \
    --manifest-path "$manifest_path" \
    --report-path "$report_path" \
    >"$log_path" 2>&1

  uc_native_ci_verify_report "$report_path" "$tag" "$allowed_backends"
}

for idx in "${!STRICT_CASE_MANIFESTS[@]}"; do
  run_case "${STRICT_CASE_MANIFESTS[$idx]}" "${STRICT_CASE_TAGS[$idx]}" "uc-native"
done

for idx in "${!BACKEND_CASE_MANIFESTS[@]}"; do
  run_case \
    "${BACKEND_CASE_MANIFESTS[$idx]}" \
    "${BACKEND_CASE_TAGS[$idx]}" \
    "${BACKEND_CASE_ALLOWED[$idx]}"
done

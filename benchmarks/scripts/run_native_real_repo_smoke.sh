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

usage() {
  cat <<'USAGE'
Usage:
  run_native_real_repo_smoke.sh [--uc-bin /abs/path/to/uc] [--results-dir /abs/path]
    [--strict-case <manifest-path> <tag> ...]
    [--backend-case <manifest-path> <tag> <allowed-backends-csv> ...]
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --uc-bin)
      UC_BIN="$2"
      shift 2
      ;;
    --results-dir)
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

if [[ ! -x "$UC_BIN" ]]; then
  echo "UC binary is missing or not executable: $UC_BIN" >&2
  exit 1
fi

mkdir -p "$RESULTS_DIR"

run_case() {
  local manifest_path="$1"
  local tag="$2"
  local allowed_backends="$3"
  local report_path="$RESULTS_DIR/native-real-${tag}.json"
  local log_path="$RESULTS_DIR/native-real-${tag}.log"

  "$UC_BIN" build \
    --engine uc \
    --daemon-mode off \
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

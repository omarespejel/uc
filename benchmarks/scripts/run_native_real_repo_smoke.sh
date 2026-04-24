#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
ROOT_DIR="$(git -C "$SCRIPT_DIR/../.." rev-parse --show-toplevel 2>/dev/null || (cd "$SCRIPT_DIR/../.." && pwd -P))"
LIB_PATH="$SCRIPT_DIR/lib/native_ci_gate.sh"

# shellcheck source=/dev/null
source "$LIB_PATH"

UC_BIN="${UC_BIN:-$ROOT_DIR/target/release/uc}"
RESULTS_DIR="$ROOT_DIR/benchmarks/results"
CASE_TIMEOUT_SECS="${UC_NATIVE_REAL_REPO_TIMEOUT_SECS:-0}"
declare -a STRICT_CASE_MANIFESTS=()
declare -a STRICT_CASE_TAGS=()
declare -a BACKEND_CASE_MANIFESTS=()
declare -a BACKEND_CASE_TAGS=()
declare -a BACKEND_CASE_ALLOWED=()
declare -A PREFETCHED_MANIFESTS=()
declare -A SEEN_TAGS=()

usage() {
  cat <<'USAGE'
Usage:
  run_native_real_repo_smoke.sh [--uc-bin /abs/path/to/uc] [--results-dir /abs/path]
    [--timeout-secs <seconds>]
    [--strict-case <manifest-path> <tag> ...]
    [--backend-case <manifest-path> <tag> <allowed-backends-csv> ...]
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

validate_timeout_secs() {
  local value="$1"
  if [[ ! "$value" =~ ^[0-9]+$ ]]; then
    echo "Invalid timeout seconds: $value" >&2
    usage >&2
    exit 2
  fi
}

validate_case_tag() {
  local tag="$1"
  if [[ ! "$tag" =~ ^[A-Za-z0-9._-]+$ ]]; then
    echo "Invalid case tag: $tag" >&2
    usage >&2
    exit 2
  fi
  if [[ -n "${SEEN_TAGS[$tag]:-}" ]]; then
    echo "Duplicate case tag: $tag" >&2
    usage >&2
    exit 2
  fi
  SEEN_TAGS["$tag"]=1
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
    --timeout-secs)
      require_option_value "$1" "${2-}"
      validate_timeout_secs "$2"
      CASE_TIMEOUT_SECS="$2"
      shift 2
      ;;
    --strict-case)
      if [[ $# -lt 3 ]]; then
        usage >&2
        exit 2
      fi
      require_option_value "--strict-case manifest-path" "${2-}"
      require_option_value "--strict-case tag" "${3-}"
      validate_case_tag "$3"
      STRICT_CASE_MANIFESTS+=("$2")
      STRICT_CASE_TAGS+=("$3")
      shift 3
      ;;
    --backend-case)
      if [[ $# -lt 4 ]]; then
        usage >&2
        exit 2
      fi
      require_option_value "--backend-case manifest-path" "${2-}"
      require_option_value "--backend-case tag" "${3-}"
      require_option_value "--backend-case allowed-backends-csv" "${4-}"
      validate_case_tag "$3"
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
  local manifest_dir
  if [[ -n "${PREFETCHED_MANIFESTS[$manifest_path]:-}" ]]; then
    return
  fi
  if ! command -v scarb >/dev/null 2>&1; then
    echo "scarb is required to prefetch dependencies for offline real-repo smoke runs" >&2
    exit 1
  fi
  manifest_dir="$(cd "$(dirname "$manifest_path")" && pwd -P)"
  (
    cd "$manifest_dir"
    if ! scarb fetch >/dev/null; then
      echo "scarb fetch failed for manifest_path=$manifest_path manifest_dir=$manifest_dir" >&2
      exit 1
    fi
  )
  PREFETCHED_MANIFESTS["$manifest_path"]=1
}

run_uc_build_case() {
  local log_path="$1"
  local tag="$2"
  shift 2
  local uc_exit=0

  set +e
  if [[ "$CASE_TIMEOUT_SECS" -gt 0 ]]; then
    python3 - "$CASE_TIMEOUT_SECS" "$log_path" "$@" <<'PY'
import subprocess
import sys
from pathlib import Path

timeout_secs = int(sys.argv[1])
log_path = Path(sys.argv[2])
command = sys.argv[3:]
log_path.parent.mkdir(parents=True, exist_ok=True)
with log_path.open("wb") as log_file:
    try:
        completed = subprocess.run(
            command,
            stdout=log_file,
            stderr=subprocess.STDOUT,
            timeout=timeout_secs,
            check=False,
        )
    except subprocess.TimeoutExpired:
        log_file.write(
            f"\nuc build timed out after {timeout_secs}s: {' '.join(command)}\n".encode(
                "utf-8"
            )
        )
        raise SystemExit(124)
raise SystemExit(completed.returncode)
PY
    uc_exit=$?
  else
    "$@" >"$log_path" 2>&1
    uc_exit=$?
  fi
  set -e

  if [[ $uc_exit -ne 0 ]]; then
    echo "uc build failed for '$tag' with exit code $uc_exit (see $log_path)" >&2
    return "$uc_exit"
  fi
}

run_case() {
  local manifest_path="$1"
  local tag="$2"
  local allowed_backends="$3"
  local report_path="$RESULTS_DIR/native-real-${tag}.json"
  local log_path="$RESULTS_DIR/native-real-${tag}.log"

  prefetch_manifest_dependencies "$manifest_path"
  run_uc_build_case \
    "$log_path" \
    "$tag" \
    "$UC_BIN" build \
    --engine uc \
    --daemon-mode off \
    --offline \
    --manifest-path "$manifest_path" \
    --report-path "$report_path"

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

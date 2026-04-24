#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
ROOT_DIR="$(git -C "$SCRIPT_DIR/../.." rev-parse --show-toplevel 2>/dev/null || (cd "$SCRIPT_DIR/../.." && pwd -P))"
LIB_PATH="$SCRIPT_DIR/lib/native_ci_gate.sh"

# shellcheck source=/dev/null
source "$LIB_PATH"

UC_BIN="${UC_BIN:-$ROOT_DIR/target/release/uc}"
RESULTS_DIR="$ROOT_DIR/benchmarks/results"
NO_SCARB_PATH=""
CASE_COUNT=0
declare -a CASE_MANIFESTS=()
declare -a CASE_TAGS=()
declare -a CASE_SKIP_UNSUPPORTED=()

usage() {
  cat <<'USAGE'
Usage:
  run_native_only_gate.sh [--uc-bin /abs/path/to/uc] [--results-dir /abs/path]
    --case <manifest-path> <tag> <allow-skip-unsupported-0-or-1>
    [--case <manifest-path> <tag> <allow-skip-unsupported-0-or-1> ...]
USAGE
}

cleanup() {
  if [[ -n "$NO_SCARB_PATH" && -d "$NO_SCARB_PATH" ]]; then
    rm -rf "$NO_SCARB_PATH"
  fi
}
trap cleanup EXIT

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
    --case)
      if [[ $# -lt 4 ]]; then
        usage >&2
        exit 2
      fi
      CASE_MANIFESTS+=("$2")
      CASE_TAGS+=("$3")
      CASE_SKIP_UNSUPPORTED+=("$4")
      CASE_COUNT=$((CASE_COUNT + 1))
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

if [[ "$CASE_COUNT" -eq 0 ]]; then
  echo "run_native_only_gate.sh requires at least one --case" >&2
  usage >&2
  exit 2
fi

if [[ ! -x "$UC_BIN" ]]; then
  echo "UC binary is missing or not executable: $UC_BIN" >&2
  exit 1
fi

mkdir -p "$RESULTS_DIR"
NO_SCARB_PATH="$(mktemp -d)"
uc_native_ci_install_no_scarb_stub "$NO_SCARB_PATH/scarb"

for idx in "${!CASE_MANIFESTS[@]}"; do
  manifest_path="${CASE_MANIFESTS[$idx]}"
  tag="${CASE_TAGS[$idx]}"
  allow_skip_unsupported="${CASE_SKIP_UNSUPPORTED[$idx]}"
  report_path="$RESULTS_DIR/native-only-${tag}.json"
  log_path="$RESULTS_DIR/native-only-${tag}.log"

  set +e
  PATH="$NO_SCARB_PATH:$PATH" \
    "$UC_BIN" build \
      --engine uc \
      --daemon-mode off \
      --offline \
      --manifest-path "$manifest_path" \
      --report-path "$report_path" \
      >"$log_path" 2>&1
  exit_code=$?
  set -e

  if [[ "$exit_code" -ne 0 ]]; then
    if [[ "$allow_skip_unsupported" == "1" ]] && uc_native_ci_log_indicates_unsupported "$log_path"; then
      echo "native-only gate: skipping unsupported fixture '$tag'"
      rm -f "$report_path"
      continue
    fi
    echo "native-only gate failed: native require build failed for '$tag'" >&2
    cat "$log_path" >&2
    exit 1
  fi

  uc_native_ci_verify_report "$report_path" "$tag" "uc-native"
done

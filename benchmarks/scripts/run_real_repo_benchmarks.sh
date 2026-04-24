#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
ROOT_DIR="$(git -C "$SCRIPT_DIR/../.." rev-parse --show-toplevel 2>/dev/null || (cd "$SCRIPT_DIR/../.." && pwd -P))"
UC_BIN="${UC_BIN:-$ROOT_DIR/target/release/uc}"
RESULTS_DIR="$ROOT_DIR/benchmarks/results"
RUNS="${RUNS:-5}"
COLD_RUNS="${COLD_RUNS:-5}"
CASE_TIMEOUT_SECS="${UC_REAL_REPO_BENCH_TIMEOUT_SECS:-0}"
WARM_SETTLE_SECONDS="${WARM_SETTLE_SECONDS:-2.2}"
STAMP="$(date +%Y%m%d-%H%M%S)"
TMP_DIR="$(mktemp -d)"
declare -a CASE_MANIFESTS=()
declare -a CASE_TAGS=()
declare -A PREFETCHED_MANIFESTS=()
declare -A SEEN_TAGS=()

cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

usage() {
  cat <<'USAGE'
Usage:
  run_real_repo_benchmarks.sh [--uc-bin /abs/path/to/uc] [--results-dir /abs/path]
    [--runs <n>] [--cold-runs <n>] [--timeout-secs <seconds>]
    [--warm-settle-seconds <seconds>]
    --case <manifest-path> <tag> [--case <manifest-path> <tag> ...]
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

validate_positive_int() {
  local flag="$1"
  local value="$2"
  if [[ ! "$value" =~ ^[0-9]+$ || "$value" -le 0 ]]; then
    echo "$flag must be a positive integer, got: $value" >&2
    exit 2
  fi
}

validate_timeout_secs() {
  local value="$1"
  if [[ ! "$value" =~ ^[0-9]+$ ]]; then
    echo "Invalid timeout seconds: $value" >&2
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
    --runs)
      require_option_value "$1" "${2-}"
      validate_positive_int "$1" "$2"
      RUNS="$2"
      shift 2
      ;;
    --cold-runs)
      require_option_value "$1" "${2-}"
      validate_positive_int "$1" "$2"
      COLD_RUNS="$2"
      shift 2
      ;;
    --timeout-secs)
      require_option_value "$1" "${2-}"
      validate_timeout_secs "$2"
      CASE_TIMEOUT_SECS="$2"
      shift 2
      ;;
    --warm-settle-seconds)
      require_option_value "$1" "${2-}"
      WARM_SETTLE_SECONDS="$2"
      shift 2
      ;;
    --case)
      if [[ $# -lt 3 ]]; then
        usage >&2
        exit 2
      fi
      require_option_value "--case manifest-path" "${2-}"
      require_option_value "--case tag" "${3-}"
      validate_case_tag "$3"
      CASE_MANIFESTS+=("$2")
      CASE_TAGS+=("$3")
      shift 3
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

if [[ "${#CASE_MANIFESTS[@]}" -eq 0 ]]; then
  echo "run_real_repo_benchmarks.sh requires at least one --case" >&2
  usage >&2
  exit 2
fi

if [[ ! -x "$UC_BIN" ]]; then
  echo "UC binary is missing or not executable: $UC_BIN" >&2
  exit 1
fi

if ! command -v scarb >/dev/null 2>&1; then
  echo "scarb is required for real repo benchmarks" >&2
  exit 1
fi
if ! command -v jq >/dev/null 2>&1; then
  echo "jq is required for real repo benchmarks" >&2
  exit 1
fi
if ! command -v python3 >/dev/null 2>&1; then
  echo "python3 is required for real repo benchmarks" >&2
  exit 1
fi

mkdir -p "$RESULTS_DIR"

measure_command_ms() {
  local cwd="$1"
  local log_path="$2"
  shift 2
  python3 - "$cwd" "$log_path" "$CASE_TIMEOUT_SECS" "$@" <<'PY'
import subprocess
import sys
import time
from pathlib import Path

cwd = Path(sys.argv[1])
log_path = Path(sys.argv[2])
timeout_secs = int(sys.argv[3])
command = sys.argv[4:]
start_ns = time.monotonic_ns()
log_path.parent.mkdir(parents=True, exist_ok=True)
with log_path.open("wb") as log_file:
    try:
        completed = subprocess.run(
            command,
            cwd=str(cwd),
            stdout=log_file,
            stderr=subprocess.STDOUT,
            timeout=timeout_secs if timeout_secs > 0 else None,
            check=False,
        )
    except subprocess.TimeoutExpired:
        log_file.write(
            f"\ncommand timed out after {timeout_secs}s: {' '.join(command)}\n".encode("utf-8")
        )
        raise SystemExit(124)
elapsed_ms = (time.monotonic_ns() - start_ns) / 1_000_000
if completed.returncode != 0:
    raise SystemExit(completed.returncode)
print(f"{elapsed_ms:.3f}")
PY
}

stats_json_from_samples() {
  local samples_file="$1"
  jq -s '
    sort as $s
    | def quantile($p):
        if length == 0 then null
        else
          (length - 1) as $n
          | ($n * $p) as $i
          | ($i | floor) as $lo
          | ($i | ceil) as $hi
          | if $lo == $hi then .[$lo]
            else (.[$lo] + ((.[$hi] - .[$lo]) * ($i - $lo)))
            end
        end;
    {
      min_ms: ($s | min),
      max_ms: ($s | max),
      mean_ms: (add / length),
      p50_ms: ($s | quantile(0.50)),
      p95_ms: ($s | quantile(0.95))
    }
  ' "$samples_file"
}

prefetch_manifest_dependencies() {
  local manifest_path="$1"
  local manifest_dir
  if [[ -n "${PREFETCHED_MANIFESTS[$manifest_path]:-}" ]]; then
    return
  fi
  manifest_dir="$(cd "$(dirname "$manifest_path")" && pwd -P)"
  (
    cd "$manifest_dir"
    scarb fetch >/dev/null
  )
  PREFETCHED_MANIFESTS["$manifest_path"]=1
}

probe_native_support() {
  local manifest_path="$1"
  "$UC_BIN" support native --manifest-path "$manifest_path" --format json
}

reset_workload_outputs() {
  local cwd="$1"
  rm -rf "$cwd/target" "$cwd/.uc" "$cwd/.scarb"
}

replace_manifest_arg() {
  local manifest_path="$1"
  shift
  local -a command=("$@")
  local manifest_index=$(( ${#command[@]} - 1 ))
  command[$manifest_index]="$manifest_path"
  printf '%s\0' "${command[@]}"
}

run_cold_stats() {
  local source_dir="$1"
  local tag="$2"
  local tool="$3"
  local sample_count="$4"
  shift 4
  local -a command_template=("$@")
  local baseline_dir="$TMP_DIR/${tag}-${tool}-cold-baseline"
  local run_root="$TMP_DIR/${tag}-${tool}-cold-runs"
  local samples_file="$TMP_DIR/${tag}-${tool}-build.cold.samples"
  : > "$samples_file"
  rm -rf "$baseline_dir" "$run_root"
  mkdir -p "$baseline_dir" "$run_root"
  cp -PR "$source_dir/." "$baseline_dir"
  reset_workload_outputs "$baseline_dir"

  for idx in $(seq 1 "$sample_count"); do
    local run_dir="$run_root/run-$idx"
    local log_path="$RESULTS_DIR/real-repo-${tag}-${tool}-build.cold-run-${idx}.log"
    rm -rf "$run_dir"
    mkdir -p "$run_dir"
    cp -PR "$baseline_dir/." "$run_dir"
    reset_workload_outputs "$run_dir"
    local -a command=()
    while IFS= read -r -d '' item; do
      command+=("$item")
    done < <(replace_manifest_arg "$run_dir/Scarb.toml" "${command_template[@]}")
    measure_command_ms "$run_dir" "$log_path" "${command[@]}" >> "$samples_file"
  done
  stats_json_from_samples "$samples_file"
}

run_warm_noop_stats() {
  local source_dir="$1"
  local tag="$2"
  local tool="$3"
  local sample_count="$4"
  shift 4
  local -a command_template=("$@")
  local warm_dir="$TMP_DIR/${tag}-${tool}-warm"
  local samples_file="$TMP_DIR/${tag}-${tool}-build.warm_noop.samples"
  local -a command=()
  : > "$samples_file"
  rm -rf "$warm_dir"
  mkdir -p "$warm_dir"
  cp -PR "$source_dir/." "$warm_dir"
  reset_workload_outputs "$warm_dir"
  while IFS= read -r -d '' item; do
    command+=("$item")
  done < <(replace_manifest_arg "$warm_dir/Scarb.toml" "${command_template[@]}")

  measure_command_ms "$warm_dir" "$RESULTS_DIR/real-repo-${tag}-${tool}-warm-prime.log" "${command[@]}" >/dev/null
  sleep "$WARM_SETTLE_SECONDS"
  for idx in $(seq 1 "$sample_count"); do
    local log_path="$RESULTS_DIR/real-repo-${tag}-${tool}-build.warm_noop-run-${idx}.log"
    measure_command_ms "$warm_dir" "$log_path" "${command[@]}" >> "$samples_file"
  done
  stats_json_from_samples "$samples_file"
}

benchmark_supported_case() {
  local manifest_path="$1"
  local tag="$2"
  local support_json="$3"
  local source_dir
  source_dir="$(cd "$(dirname "$manifest_path")" && pwd -P)"
  prefetch_manifest_dependencies "$manifest_path"

  local -a scarb_cmd=(scarb --manifest-path "$manifest_path" --offline build)
  local -a uc_cmd=("$UC_BIN" build --engine uc --daemon-mode off --offline --manifest-path "$manifest_path")

  local scarb_cold_json
  local scarb_warm_json
  local uc_cold_json
  local uc_warm_json
  scarb_cold_json="$(run_cold_stats "$source_dir" "$tag" "scarb" "$COLD_RUNS" "${scarb_cmd[@]}")"
  scarb_warm_json="$(run_warm_noop_stats "$source_dir" "$tag" "scarb" "$RUNS" "${scarb_cmd[@]}")"
  UC_NATIVE_DISALLOW_SCARB_FALLBACK=1 uc_cold_json="$(run_cold_stats "$source_dir" "$tag" "uc" "$COLD_RUNS" "${uc_cmd[@]}")"
  UC_NATIVE_DISALLOW_SCARB_FALLBACK=1 uc_warm_json="$(run_warm_noop_stats "$source_dir" "$tag" "uc" "$RUNS" "${uc_cmd[@]}")"

  jq -n \
    --arg tag "$tag" \
    --arg manifest_path "$manifest_path" \
    --argjson native_support "$support_json" \
    --argjson scarb_cold "$scarb_cold_json" \
    --argjson scarb_warm "$scarb_warm_json" \
    --argjson uc_cold "$uc_cold_json" \
    --argjson uc_warm "$uc_warm_json" \
    '{
      tag: $tag,
      manifest_path: $manifest_path,
      native_support: $native_support,
      benchmarks: {
        scarb: {
          build: {
            cold: $scarb_cold,
            warm_noop: $scarb_warm
          }
        },
        uc: {
          build: {
            cold: $uc_cold,
            warm_noop: $uc_warm
          }
        }
      }
    }'
}

record_ineligible_case() {
  local manifest_path="$1"
  local tag="$2"
  local support_json="$3"
  jq -n \
    --arg tag "$tag" \
    --arg manifest_path "$manifest_path" \
    --argjson native_support "$support_json" \
    '{
      tag: $tag,
      manifest_path: $manifest_path,
      native_support: $native_support,
      benchmarks: null
    }'
}

for idx in "${!CASE_MANIFESTS[@]}"; do
  manifest_path="$(cd "$(dirname "${CASE_MANIFESTS[$idx]}")" && pwd -P)/$(basename "${CASE_MANIFESTS[$idx]}")"
  tag="${CASE_TAGS[$idx]}"
  support_json="$(probe_native_support "$manifest_path")"
  if [[ "$(jq -r '.supported' <<<"$support_json")" == "true" ]]; then
    benchmark_supported_case "$manifest_path" "$tag" "$support_json" >> "$TMP_DIR/cases.ndjson"
  else
    record_ineligible_case "$manifest_path" "$tag" "$support_json" >> "$TMP_DIR/cases.ndjson"
  fi
done

OUT_JSON="$RESULTS_DIR/real-repo-bench-$STAMP.json"
OUT_MD="$RESULTS_DIR/real-repo-bench-$STAMP.md"
GENERATED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

jq -s \
  --arg generated_at "$GENERATED_AT" \
  --arg uc_bin "$UC_BIN" \
  --arg warm_settle_seconds "$WARM_SETTLE_SECONDS" \
  --argjson runs "$RUNS" \
  --argjson cold_runs "$COLD_RUNS" \
  '{
    generated_at: $generated_at,
    uc_bin: $uc_bin,
    runs: $runs,
    cold_runs: $cold_runs,
    warm_settle_seconds: ($warm_settle_seconds | tonumber),
    cases: .
  }' "$TMP_DIR/cases.ndjson" > "$OUT_JSON"

{
  echo "# Real Repo Benchmark ($STAMP)"
  echo
  echo "- Generated at: $GENERATED_AT"
  echo "- UC binary: $UC_BIN"
  echo "- Warm runs: $RUNS"
  echo "- Cold runs: $COLD_RUNS"
  echo "- Warm settle seconds: $WARM_SETTLE_SECONDS"
  echo
  echo "## Native-Eligible Cases"
  echo "| Tag | Cold Scarb p95 (ms) | Cold UC p95 (ms) | Cold Speedup (x) | Warm Scarb p95 (ms) | Warm UC p95 (ms) | Warm Speedup (x) |"
  echo "|---|---:|---:|---:|---:|---:|---:|"
  jq -r '
    def r3: ((. * 1000 | round) / 1000);
    .cases[]
    | select(.native_support.supported == true)
    | . as $case
    | ($case.benchmarks.scarb.build.cold.p95_ms) as $scarb_cold
    | ($case.benchmarks.uc.build.cold.p95_ms) as $uc_cold
    | ($case.benchmarks.scarb.build.warm_noop.p95_ms) as $scarb_warm
    | ($case.benchmarks.uc.build.warm_noop.p95_ms) as $uc_warm
    | "| \($case.tag) | \($scarb_cold | r3) | \($uc_cold | r3) | \((if $uc_cold == 0 then null else $scarb_cold / $uc_cold end) | r3) | \($scarb_warm | r3) | \($uc_warm | r3) | \((if $uc_warm == 0 then null else $scarb_warm / $uc_warm end) | r3) |"
  ' "$OUT_JSON"
  echo
  echo "## Native-Ineligible Cases"
  echo "| Tag | Package Cairo Version | Reason |"
  echo "|---|---|---|"
  jq -r '
    .cases[]
    | select(.native_support.supported != true)
    | "| \(.tag) | \(.native_support.package_cairo_version // "<none>") | \(.native_support.reason // "unknown reason") |"
  ' "$OUT_JSON"
} > "$OUT_MD"

echo "Benchmark JSON: $OUT_JSON"
echo "Benchmark Markdown: $OUT_MD"

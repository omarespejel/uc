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

validate_non_negative_number() {
  local flag="$1"
  local value="$2"
  if [[ ! "$value" =~ ^([0-9]+([.][0-9]+)?|[.][0-9]+)$ ]]; then
    echo "$flag must be a non-negative number, got: $value" >&2
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

validate_positive_int "RUNS" "$RUNS"
validate_positive_int "COLD_RUNS" "$COLD_RUNS"
validate_non_negative_number "WARM_SETTLE_SECONDS" "$WARM_SETTLE_SECONDS"

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
  if [[ ! -s "$samples_file" ]]; then
    echo "stats_json_from_samples: no samples recorded in $samples_file" >&2
    return 1
  fi
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

measurement_ok_json() {
  local stage="$1"
  local sample_count="$2"
  local stats_json="$3"
  jq -n \
    --arg stage "$stage" \
    --argjson sample_count "$sample_count" \
    --argjson stats "$stats_json" \
    '{
      status: "ok",
      stage: $stage,
      sample_count: $sample_count,
      stats: $stats
    }'
}

measurement_failure_json() {
  local stage="$1"
  local exit_code="$2"
  local log_path="$3"
  jq -n \
    --arg stage "$stage" \
    --argjson exit_code "$exit_code" \
    --arg log_path "$log_path" \
    '{
      status: "failed",
      stage: $stage,
      exit_code: $exit_code,
      log_path: $log_path
    }'
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

classify_support_matrix_case() {
  local manifest_path="$1"
  local tag="$2"
  local support_json="$3"
  if [[ "$(jq -r '.supported' <<<"$support_json")" != "true" ]]; then
    jq -n \
      --argjson native_support "$support_json" \
      '{
        classification: "native_unsupported",
        compile_backend: null,
        fallback_used: false,
        exit_code: 0,
        elapsed_ms: null,
        log_path: null,
        report_path: null,
        build_report: null,
        reason: ($native_support.reason // "native support probe reported unsupported")
      }'
    return 0
  fi

  local source_dir
  source_dir="$(cd "$(dirname "$manifest_path")" && pwd -P)"
  local classify_dir="$TMP_DIR/${tag}-uc-auto-classify"
  local log_path="$RESULTS_DIR/real-repo-${tag}-uc-auto-build.log"
  local report_path="$RESULTS_DIR/real-repo-${tag}-uc-auto-build-report.json"
  rm -rf "$classify_dir"
  mkdir -p "$classify_dir"
  cp -PR "$source_dir/." "$classify_dir"
  reset_workload_outputs "$classify_dir"
  prefetch_manifest_dependencies "$manifest_path"

  local -a uc_auto_cmd=(env "UC_NATIVE_BUILD=auto")
  if [[ -n "${UC_NATIVE_CORELIB_SRC:-}" ]]; then
    uc_auto_cmd+=("UC_NATIVE_CORELIB_SRC=$UC_NATIVE_CORELIB_SRC")
  fi
  uc_auto_cmd+=(
    "$UC_BIN"
    build
    --engine
    uc
    --daemon-mode
    off
    --offline
    --report-path
    "$report_path"
    --manifest-path
    "$classify_dir/Scarb.toml"
  )

  local elapsed_ms=""
  local exit_code=0
  elapsed_ms="$(measure_command_ms "$classify_dir" "$log_path" "${uc_auto_cmd[@]}")" || exit_code=$?
  local build_report_json="null"
  if [[ -f "$report_path" ]]; then
    build_report_json="$(cat "$report_path")"
  fi
  rm -rf "$classify_dir"

  jq -n \
    --argjson native_support "$support_json" \
    --argjson build_report "$build_report_json" \
    --argjson exit_code "$exit_code" \
    --arg elapsed_ms "$elapsed_ms" \
    --arg log_path "$log_path" \
    --arg report_path "$report_path" \
    '
      def fallback_used:
        (($build_report.diagnostics // []) | any(.fallback_used == true));
      def compile_backend:
        ($build_report.compile_backend // null);
      def classification:
        if $native_support.supported != true then
          "native_unsupported"
        elif $exit_code != 0 then
          "build_failed"
        elif compile_backend == "uc_native" or compile_backend == "uc_native_external_helper" then
          "native_supported"
        elif compile_backend == "scarb_fallback" or compile_backend == "uc_scarb" or fallback_used then
          "fallback_used"
        else
          "build_failed"
        end;
      {
        classification: classification,
        compile_backend: compile_backend,
        fallback_used: fallback_used,
        exit_code: $exit_code,
        elapsed_ms: (if $elapsed_ms == "" then null else ($elapsed_ms | tonumber) end),
        log_path: $log_path,
        report_path: (if $build_report == null then null else $report_path end),
        build_report: $build_report,
        reason: (
          if classification == "native_unsupported" then
            ($native_support.reason // "native support probe reported unsupported")
          elif classification == "fallback_used" then
            ((($build_report.diagnostics // []) | map(select(.fallback_used == true) | .why) | first)
              // "uc auto build fell back to the scarb backend")
          elif classification == "build_failed" then
            ((($build_report.diagnostics // []) | map(.why) | first)
              // "uc auto build failed before backend classification completed")
          else
            null
          end
        )
      }'
}

reset_workload_outputs() {
  local cwd="$1"
  rm -rf "$cwd/target" "$cwd/.uc" "$cwd/.scarb"
}

replace_manifest_arg() {
  local manifest_path="$1"
  shift
  local -a command=("$@")
  local replaced=0
  local idx=0
  while [[ "$idx" -lt "${#command[@]}" ]]; do
    if [[ "${command[$idx]}" == "--manifest-path" ]]; then
      local value_index=$(( idx + 1 ))
      if [[ "$value_index" -ge "${#command[@]}" ]]; then
        echo "replace_manifest_arg: --manifest-path is missing its value" >&2
        return 1
      fi
      command[$value_index]="$manifest_path"
      replaced=1
      break
    fi
    idx=$(( idx + 1 ))
  done
  if [[ "$replaced" -ne 1 ]]; then
    echo "replace_manifest_arg: command is missing --manifest-path: ${command[*]}" >&2
    return 1
  fi
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
  local stage="build.cold"
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
    local exit_code=0
    measure_command_ms "$run_dir" "$log_path" "${command[@]}" >> "$samples_file" || exit_code=$?
    if [[ "$exit_code" -ne 0 ]]; then
      rm -rf "$baseline_dir" "$run_root"
      measurement_failure_json "$stage" "$exit_code" "$log_path"
      return 0
    fi
  done
  local stats_json
  if ! stats_json="$(stats_json_from_samples "$samples_file")"; then
    rm -rf "$baseline_dir" "$run_root"
    measurement_failure_json "$stage" "-1" "$samples_file"
    return 0
  fi
  rm -rf "$baseline_dir" "$run_root"
  measurement_ok_json "$stage" "$sample_count" "$stats_json"
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
  local stage="build.warm_noop"
  local -a command=()
  : > "$samples_file"
  rm -rf "$warm_dir"
  mkdir -p "$warm_dir"
  cp -PR "$source_dir/." "$warm_dir"
  reset_workload_outputs "$warm_dir"
  while IFS= read -r -d '' item; do
    command+=("$item")
  done < <(replace_manifest_arg "$warm_dir/Scarb.toml" "${command_template[@]}")

  local warm_prime_log="$RESULTS_DIR/real-repo-${tag}-${tool}-warm-prime.log"
  local exit_code=0
  measure_command_ms "$warm_dir" "$warm_prime_log" "${command[@]}" >/dev/null || exit_code=$?
  if [[ "$exit_code" -ne 0 ]]; then
    rm -rf "$warm_dir"
    measurement_failure_json "$stage.prime" "$exit_code" "$warm_prime_log"
    return 0
  fi
  sleep "$WARM_SETTLE_SECONDS"
  for idx in $(seq 1 "$sample_count"); do
    local log_path="$RESULTS_DIR/real-repo-${tag}-${tool}-build.warm_noop-run-${idx}.log"
    local exit_code=0
    measure_command_ms "$warm_dir" "$log_path" "${command[@]}" >> "$samples_file" || exit_code=$?
    if [[ "$exit_code" -ne 0 ]]; then
      rm -rf "$warm_dir"
      measurement_failure_json "$stage" "$exit_code" "$log_path"
      return 0
    fi
  done
  local stats_json
  if ! stats_json="$(stats_json_from_samples "$samples_file")"; then
    rm -rf "$warm_dir"
    measurement_failure_json "$stage" "-1" "$samples_file"
    return 0
  fi
  rm -rf "$warm_dir"
  measurement_ok_json "$stage" "$sample_count" "$stats_json"
}

benchmark_supported_case() {
  local manifest_path="$1"
  local tag="$2"
  local support_json="$3"
  local support_matrix_json="$4"
  local source_dir
  source_dir="$(cd "$(dirname "$manifest_path")" && pwd -P)"
  prefetch_manifest_dependencies "$manifest_path"

  local -a scarb_cmd=(scarb --manifest-path "$manifest_path" --offline build)
  local -a uc_cmd=(env "UC_NATIVE_DISALLOW_SCARB_FALLBACK=1")
  if [[ -n "${UC_NATIVE_CORELIB_SRC:-}" ]]; then
    uc_cmd+=("UC_NATIVE_CORELIB_SRC=$UC_NATIVE_CORELIB_SRC")
  fi
  uc_cmd+=("$UC_BIN" build --engine uc --daemon-mode off --offline --manifest-path "$manifest_path")

  local scarb_cold_json
  local scarb_warm_json
  local uc_cold_json
  local uc_warm_json
  scarb_cold_json="$(run_cold_stats "$source_dir" "$tag" "scarb" "$COLD_RUNS" "${scarb_cmd[@]}")"
  scarb_warm_json="$(run_warm_noop_stats "$source_dir" "$tag" "scarb" "$RUNS" "${scarb_cmd[@]}")"
  uc_cold_json="$(run_cold_stats "$source_dir" "$tag" "uc" "$COLD_RUNS" "${uc_cmd[@]}")"
  uc_warm_json="$(run_warm_noop_stats "$source_dir" "$tag" "uc" "$RUNS" "${uc_cmd[@]}")"

  jq -n \
    --arg tag "$tag" \
    --arg manifest_path "$manifest_path" \
    --argjson native_support "$support_json" \
    --argjson support_matrix "$support_matrix_json" \
    --argjson scarb_cold "$scarb_cold_json" \
    --argjson scarb_warm "$scarb_warm_json" \
    --argjson uc_cold "$uc_cold_json" \
    --argjson uc_warm "$uc_warm_json" \
    '{
      tag: $tag,
      manifest_path: $manifest_path,
      native_support: $native_support,
      support_matrix: $support_matrix,
      benchmark_status: (
        if [
          $scarb_cold.status,
          $scarb_warm.status,
          $uc_cold.status,
          $uc_warm.status
        ] | all(. == "ok") then "ok" else "failed" end
      ),
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

record_non_benchmarked_case() {
  local manifest_path="$1"
  local tag="$2"
  local support_json="$3"
  local support_matrix_json="$4"
  jq -n \
    --arg tag "$tag" \
    --arg manifest_path "$manifest_path" \
    --argjson native_support "$support_json" \
    --argjson support_matrix "$support_matrix_json" \
    '{
      tag: $tag,
      manifest_path: $manifest_path,
      native_support: $native_support,
      support_matrix: $support_matrix,
      benchmark_status: "skipped",
      benchmarks: null
    }'
}

for idx in "${!CASE_MANIFESTS[@]}"; do
  manifest_path="$(cd "$(dirname "${CASE_MANIFESTS[$idx]}")" && pwd -P)/$(basename "${CASE_MANIFESTS[$idx]}")"
  tag="${CASE_TAGS[$idx]}"
  support_json="$(probe_native_support "$manifest_path")"
  support_matrix_json="$(classify_support_matrix_case "$manifest_path" "$tag" "$support_json")"
  if [[ "$(jq -r '.classification' <<<"$support_matrix_json")" == "native_supported" ]]; then
    benchmark_supported_case "$manifest_path" "$tag" "$support_json" "$support_matrix_json" >> "$TMP_DIR/cases.ndjson"
  else
    record_non_benchmarked_case "$manifest_path" "$tag" "$support_json" "$support_matrix_json" >> "$TMP_DIR/cases.ndjson"
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
  echo "## Support Matrix"
  echo "| Tag | Classification | Compile Backend | Requested | Found | Fallback Used | Reason |"
  echo "|---|---|---|---|---|---|---|"
  jq -r '
    .cases[]
    | .support_matrix as $matrix
    | .native_support as $support
    | ($matrix.build_report.native_toolchain // $support.toolchain // null) as $toolchain
    | "| \(.tag) | \($matrix.classification) | \($matrix.compile_backend // "<none>") | \($toolchain.requested_version // $toolchain.requested_major_minor // $support.package_cairo_version // "<none>") | \($toolchain.compiler_version // "<none>") | \($matrix.fallback_used) | \($matrix.reason // "<none>") |"
  ' "$OUT_JSON"
  echo
  echo "## Native-Supported Benchmark Cases"
  echo "| Tag | Cold Scarb p95 (ms) | Cold UC p95 (ms) | Cold Speedup (x) | Warm Scarb p95 (ms) | Warm UC p95 (ms) | Warm Speedup (x) |"
  echo "|---|---:|---:|---:|---:|---:|---:|"
  jq -r '
    def r3: ((. * 1000 | round) / 1000);
    .cases[]
    | select(.support_matrix.classification == "native_supported" and .benchmark_status == "ok")
    | . as $case
    | ($case.benchmarks.scarb.build.cold.stats.p95_ms) as $scarb_cold
    | ($case.benchmarks.uc.build.cold.stats.p95_ms) as $uc_cold
    | ($case.benchmarks.scarb.build.warm_noop.stats.p95_ms) as $scarb_warm
    | ($case.benchmarks.uc.build.warm_noop.stats.p95_ms) as $uc_warm
    | "| \($case.tag) | \($scarb_cold | r3) | \($uc_cold | r3) | \((if $uc_cold == 0 then null else $scarb_cold / $uc_cold end) | r3) | \($scarb_warm | r3) | \($uc_warm | r3) | \((if $uc_warm == 0 then null else $scarb_warm / $uc_warm end) | r3) |"
  ' "$OUT_JSON"
  echo
  echo "## Native-Supported Benchmark Cases With Build Failures"
  echo "| Tag | Tool | Stage | Exit Code | Log Path |"
  echo "|---|---|---|---:|---|"
  jq -r '
    .cases[]
    | select(.support_matrix.classification == "native_supported" and .benchmark_status != "ok")
    | . as $case
    | [
        {tool: "scarb", lane: $case.benchmarks.scarb.build.cold},
        {tool: "scarb", lane: $case.benchmarks.scarb.build.warm_noop},
        {tool: "uc", lane: $case.benchmarks.uc.build.cold},
        {tool: "uc", lane: $case.benchmarks.uc.build.warm_noop}
      ][]
    | select(.lane.status != "ok")
    | "| \($case.tag) | \(.tool) | \(.lane.stage) | \(.lane.exit_code) | \(.lane.log_path) |"
  ' "$OUT_JSON"
  echo
  echo "## Fallback-Used Cases"
  echo "| Tag | Compile Backend | Requested | Found | Reason |"
  echo "|---|---|---|---|---|"
  jq -r '
    .cases[]
    | select(.support_matrix.classification == "fallback_used")
    | .support_matrix as $matrix
    | (.support_matrix.build_report.native_toolchain // .native_support.toolchain // null) as $toolchain
    | "| \(.tag) | \($matrix.compile_backend // "<none>") | \($toolchain.requested_version // $toolchain.requested_major_minor // .native_support.package_cairo_version // "<none>") | \($toolchain.compiler_version // "<none>") | \($matrix.reason // "unknown reason") |"
  ' "$OUT_JSON"
  echo
  echo "## Native-Unsupported Cases"
  echo "| Tag | Requested | Reason |"
  echo "|---|---|---|"
  jq -r '
    .cases[]
    | select(.support_matrix.classification == "native_unsupported")
    | "| \(.tag) | \(.native_support.package_cairo_version // .native_support.toolchain.requested_version // .native_support.toolchain.requested_major_minor // "<none>") | \(.support_matrix.reason // .native_support.reason // "unknown reason") |"
  ' "$OUT_JSON"
  echo
  echo "## Auto-Build Classification Failures"
  echo "| Tag | Exit Code | Log Path | Reason |"
  echo "|---|---:|---|---|"
  jq -r '
    .cases[]
    | select(.support_matrix.classification == "build_failed")
    | "| \(.tag) | \(.support_matrix.exit_code) | \(.support_matrix.log_path // "<none>") | \(.support_matrix.reason // "unknown reason") |"
  ' "$OUT_JSON"
} > "$OUT_MD"

echo "Benchmark JSON: $OUT_JSON"
echo "Benchmark Markdown: $OUT_MD"

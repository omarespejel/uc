#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
ROOT_DIR="$(git -C "$SCRIPT_DIR/../.." rev-parse --show-toplevel 2>/dev/null || (cd "$SCRIPT_DIR/../.." && pwd -P))"
REAL_REPO_BENCH_SCRIPT="${REAL_REPO_BENCH_SCRIPT:-$SCRIPT_DIR/run_real_repo_benchmarks.sh}"

UC_BIN="${UC_BIN:-$ROOT_DIR/target/release/uc}"
RESULTS_DIR="$ROOT_DIR/benchmarks/results"
RUNS="${RUNS:-12}"
COLD_RUNS="${COLD_RUNS:-12}"
WARM_SETTLE_SECONDS="${WARM_SETTLE_SECONDS:-2.2}"
SOURCE_BENCHMARK_JSON=""
STAMP="strict-supported-$(date +%Y%m%d-%H%M%S)"
TMP_DIR="$(mktemp -d)"

cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

usage() {
  cat <<'USAGE'
Usage:
  run_strict_supported_set_benchmarks.sh --benchmark-json /abs/path/to/real-repo-bench.json
    [--uc-bin /abs/path/to/uc] [--results-dir /abs/path]
    [--runs <n>] [--cold-runs <n>] [--warm-settle-seconds <seconds>]
    [--stamp <id>]

This wrapper extracts only the `native_supported` cases from a prior real-repo
benchmark artifact, reruns them in the same window through
run_real_repo_benchmarks.sh, and emits a strict supported-set artifact with
selection provenance and claim guards.
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

validate_stamp() {
  local value="$1"
  if [[ ! "$value" =~ ^[A-Za-z0-9._-]+$ ]]; then
    echo "Invalid stamp: $value (allowed: A-Z a-z 0-9 . _ -)" >&2
    exit 2
  fi
}

validate_benchmark_schema() {
  local label="$1"
  local path="$2"
  local require_summary="${3:-0}"
  if ! jq -e '.schema_version == 1 and (.cases | type == "array")' "$path" >/dev/null; then
    echo "Unsupported benchmark schema in $label (expected schema_version=1 with array cases): $path" >&2
    exit 1
  fi
  if [[ "$require_summary" == "1" ]] && ! jq -e '(.summary | type) == "object"' "$path" >/dev/null; then
    echo "Unsupported benchmark schema in $label (expected summary object for rerun artifact): $path" >&2
    exit 1
  fi
}

canonical_existing_file_path() {
  local label="$1"
  local path="$2"
  if [[ ! -f "$path" ]]; then
    echo "$label is missing or not a regular file: $path" >&2
    exit 1
  fi
  local dir base
  dir="$(cd "$(dirname "$path")" && pwd -P)"
  base="$(basename "$path")"
  if [[ "$dir" == "/" ]]; then
    printf '/%s\n' "$base"
  else
    printf '%s/%s\n' "${dir%/}" "$base"
  fi
}

canonical_dir_path() {
  local label="$1"
  local path="$2"
  if ! mkdir -p "$path"; then
    echo "Failed to create $label: $path" >&2
    exit 1
  fi
  (cd "$path" && pwd -P)
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --benchmark-json)
      require_option_value "$1" "${2-}"
      SOURCE_BENCHMARK_JSON="$2"
      shift 2
      ;;
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
    --warm-settle-seconds)
      require_option_value "$1" "${2-}"
      validate_non_negative_number "$1" "$2"
      WARM_SETTLE_SECONDS="$2"
      shift 2
      ;;
    --stamp)
      require_option_value "$1" "${2-}"
      STAMP="$2"
      validate_stamp "$STAMP"
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

if [[ -z "$SOURCE_BENCHMARK_JSON" ]]; then
  echo "--benchmark-json is required" >&2
  usage >&2
  exit 2
fi

SOURCE_BENCHMARK_JSON="$(canonical_existing_file_path "benchmark JSON" "$SOURCE_BENCHMARK_JSON")"
RESULTS_DIR="$(canonical_dir_path "results directory" "$RESULTS_DIR")"
validate_stamp "$STAMP"
validate_benchmark_schema "source artifact" "$SOURCE_BENCHMARK_JSON"

if [[ ! -x "$UC_BIN" ]]; then
  echo "UC binary is missing or not executable: $UC_BIN" >&2
  exit 1
fi

selected_cases_tsv="$RESULTS_DIR/strict-supported-set-$STAMP.tsv"
selected_cases_json="$TMP_DIR/native-supported-cases.json"
jq -r '
  .cases[]
  | select(.support_matrix.classification == "native_supported")
  | [.manifest_path, .tag]
  | @tsv
' "$SOURCE_BENCHMARK_JSON" > "$selected_cases_tsv"
{
  echo "["
  first=1
  while IFS=$'\t' read -r manifest_path tag; do
    [[ -z "$manifest_path" || -z "$tag" ]] && continue
    canonical_manifest_path="$(canonical_existing_file_path "selected manifest" "$manifest_path")"
    if [[ "$first" -eq 0 ]]; then
      echo ","
    fi
    jq -nc \
      --arg manifest_path "$canonical_manifest_path" \
      --arg tag "$tag" \
      '{manifest_path: $manifest_path, tag: $tag}'
    first=0
  done < "$selected_cases_tsv"
  echo "]"
} > "$selected_cases_json"

selected_case_count="$(wc -l < "$selected_cases_tsv" | tr -d '[:space:]')"
if [[ "$selected_case_count" == "0" ]]; then
  echo "No native_supported cases found in benchmark artifact: $SOURCE_BENCHMARK_JSON" >&2
  exit 1
fi

source_native_supported_count="$(jq '[
  .cases[]
  | select(.support_matrix.classification == "native_supported")
] | length' "$SOURCE_BENCHMARK_JSON")"
source_case_count="$(jq '[.cases[]] | length' "$SOURCE_BENCHMARK_JSON")"

run_log="$RESULTS_DIR/strict-supported-set-$STAMP.log"
"$REAL_REPO_BENCH_SCRIPT" \
  --uc-bin "$UC_BIN" \
  --results-dir "$RESULTS_DIR" \
  --runs "$RUNS" \
  --cold-runs "$COLD_RUNS" \
  --warm-settle-seconds "$WARM_SETTLE_SECONDS" \
  --stamp "$STAMP-rerun" \
  --cases-file "$selected_cases_tsv" | tee "$run_log"

rerun_json="$(sed -n 's/^Benchmark JSON: //p' "$run_log" | tail -n1)"
rerun_md="$(sed -n 's/^Benchmark Markdown: //p' "$run_log" | tail -n1)"
if [[ -z "$rerun_json" || -z "$rerun_md" ]]; then
  echo "Failed to capture rerun artifact paths from $run_log" >&2
  exit 1
fi
rerun_json="$(canonical_existing_file_path "rerun benchmark JSON" "$rerun_json")"
rerun_md="$(canonical_existing_file_path "rerun benchmark Markdown" "$rerun_md")"
validate_benchmark_schema "rerun artifact" "$rerun_json" 1

strict_json="$RESULTS_DIR/strict-supported-set-$STAMP.json"
strict_md="$RESULTS_DIR/strict-supported-set-$STAMP.md"
generated_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
replay_command="$(printf '%q ' "$0" --benchmark-json "$SOURCE_BENCHMARK_JSON" --uc-bin "$UC_BIN" --results-dir "$RESULTS_DIR" --runs "$RUNS" --cold-runs "$COLD_RUNS" --warm-settle-seconds "$WARM_SETTLE_SECONDS" --stamp "$STAMP" | sed 's/[[:space:]]*$//')"

jq -n \
  --slurpfile selected_cases "$selected_cases_json" \
  --slurpfile rerun "$rerun_json" \
  --argjson expected_runs "$RUNS" \
  --argjson expected_cold_runs "$COLD_RUNS" \
  --argjson expected_warm_settle_seconds "$WARM_SETTLE_SECONDS" \
  --arg generated_at "$generated_at" \
  --arg source_benchmark_json "$SOURCE_BENCHMARK_JSON" \
  --arg rerun_benchmark_json "$rerun_json" \
  --arg rerun_benchmark_markdown "$rerun_md" \
  --arg selection_cases_file "$selected_cases_tsv" \
  --arg replay_command "$replay_command" \
  --arg artifact_path "$strict_json" \
  --arg log_path "$run_log" \
  --argjson selected_case_count "$selected_case_count" \
  --argjson source_native_supported_count "$source_native_supported_count" \
  --argjson source_case_count "$source_case_count" \
  '
  def sorted_case_set:
    map({manifest_path, tag}) | sort_by(.manifest_path, .tag);
  def safe_claim($selected_count):
    (.runs == $expected_runs)
    and (.cold_runs == $expected_cold_runs)
    and (.warm_settle_seconds == $expected_warm_settle_seconds)
    and (.summary.support_matrix.native_supported == $selected_count)
    and (.summary.support_matrix.native_unsupported == 0)
    and (.summary.support_matrix.fallback_used == 0)
    and (.summary.support_matrix.build_failed == 0)
    and ((.summary.unstable_lane_count // 0) == 0)
    and ([.cases[].benchmark_status] | all(. == "ok"))
    and ((.cases | sorted_case_set) == ($selected_cases[0] | sorted_case_set));
  def guard_reason($selected_count):
    if $selected_count == 0 then
      "no native-supported cases were selected"
    elif .runs != $expected_runs then
      "runs changed in rerun"
    elif .cold_runs != $expected_cold_runs then
      "cold_runs changed in rerun"
    elif .warm_settle_seconds != $expected_warm_settle_seconds then
      "warm_settle_seconds changed in rerun"
    elif ((.cases | sorted_case_set) != ($selected_cases[0] | sorted_case_set)) then
      "rerun case set did not match the selected native_supported source cases"
    elif .summary.support_matrix.native_supported != $selected_count then
      "one or more selected cases no longer classified as native_supported in the rerun"
    elif .summary.support_matrix.native_unsupported != 0 then
      "one or more selected cases became native_unsupported in the rerun"
    elif .summary.support_matrix.fallback_used != 0 then
      "one or more selected cases used fallback in the rerun"
    elif .summary.support_matrix.build_failed != 0 then
      "one or more selected cases failed to build in the rerun"
    elif ((.summary.unstable_lane_count // 0) != 0) then
      "one or more rerun benchmark lanes were unstable"
    elif ([.cases[].benchmark_status] | any(. != "ok")) then
      "one or more rerun benchmark cases did not benchmark successfully"
    else
      "all selected cases remained native_supported and passed the strict same-window rerun"
    end;
  $rerun[0]
  | . + {
      generated_at: $generated_at,
      what_happened: "Reran the native-supported subset from a prior real-repo benchmark artifact under strict same-window settings.",
      why: "Launch-grade speed claims must come only from cases that are both native-supported and stable in the rerun artifact.",
      retryable: true,
      expected: {
        selected_classification: "native_supported",
        selected_case_count: $selected_case_count,
        runs: $expected_runs,
        cold_runs: $expected_cold_runs,
        warm_settle_seconds: $expected_warm_settle_seconds,
        unstable_lane_count: 0,
        benchmark_status: "ok"
      },
      found: {
        runs: .runs,
        cold_runs: .cold_runs,
        warm_settle_seconds: .warm_settle_seconds,
        support_matrix: .summary.support_matrix,
        unstable_lane_count: (.summary.unstable_lane_count // 0),
        benchmark_statuses: [.cases[].benchmark_status]
      },
      fallback_used: ((.summary.support_matrix.fallback_used // 0) != 0),
      replay_command: $replay_command,
      artifact_path: $artifact_path,
      log_path: $log_path,
      selection: {
        source_benchmark_json: $source_benchmark_json,
        source_case_count: $source_case_count,
        source_native_supported_count: $source_native_supported_count,
        selected_case_count: $selected_case_count,
        selected_cases_file: $selection_cases_file,
        rerun_benchmark_json: $rerun_benchmark_json,
        rerun_benchmark_markdown: $rerun_benchmark_markdown,
        selected_cases: $selected_cases[0],
        selected_tags: ($selected_cases[0] | map(.tag))
      },
      claim_guard: (
        . as $report
        | ($report | safe_claim($selected_case_count)) as $safe
        | {
            safe_to_say_native_supported_speed_claim: $safe,
            reason: ($report | guard_reason($selected_case_count)),
            native_supported_speed_claim_text: (
              if $safe then
                "Every case in this strict same-window native-supported rerun stayed native-supported and benchmarked successfully."
              else
                null
              end
            )
          }
      )
    }
  ' > "$strict_json"

jq -r '
  [
    "# Strict Supported-Set Benchmark",
    "",
    "- Generated at: \(.generated_at)",
    "- Source benchmark JSON: \(.selection.source_benchmark_json)",
    "- Source case count: \(.selection.source_case_count)",
    "- Source native-supported count: \(.selection.source_native_supported_count)",
    "- Selected case count: \(.selection.selected_case_count)",
    "- Rerun benchmark JSON: \(.selection.rerun_benchmark_json)",
    "- Rerun benchmark Markdown: \(.selection.rerun_benchmark_markdown)",
    "- Claim safe: \(.claim_guard.safe_to_say_native_supported_speed_claim)",
    "- Claim reason: \(.claim_guard.reason)",
    (if .claim_guard.native_supported_speed_claim_text then "- Claim text: \(.claim_guard.native_supported_speed_claim_text)" else "- Claim text: <not safe for this artifact>" end),
    "",
    "## Selected Tags",
    (.selection.selected_tags[] | "- " + .),
    "",
    "## Support Matrix",
    "- native_supported: \(.summary.support_matrix.native_supported)",
    "- native_unsupported: \(.summary.support_matrix.native_unsupported)",
    "- fallback_used: \(.summary.support_matrix.fallback_used)",
    "- build_failed: \(.summary.support_matrix.build_failed)",
    "- unstable_lane_count: \(.summary.unstable_lane_count // 0)"
  ] | join("\n")
' "$strict_json" > "$strict_md"

echo "Strict benchmark JSON: $strict_json"
echo "Strict benchmark Markdown: $strict_md"

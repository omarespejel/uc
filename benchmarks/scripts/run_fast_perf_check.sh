#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd)"
OUT_DIR="$ROOT_DIR/benchmarks/results"
MATRIX="${MATRIX:-smoke}"
RUNS="${RUNS:-4}"
COLD_RUNS="${COLD_RUNS:-4}"
BUILD_OFFLINE="${BUILD_OFFLINE:-1}"
UC_DAEMON_MODE="${UC_DAEMON_MODE:-require}"
CPU_SET="${CPU_SET:-}"
NICE_LEVEL="${NICE_LEVEL:-0}"
STRICT_PINNING="${STRICT_PINNING:-0}"
HOST_PREFLIGHT_MODE="${HOST_PREFLIGHT_MODE:-warn}"
ALLOW_NOISY_HOST="${ALLOW_NOISY_HOST:-0}"
WORKSPACE_ROOT="${WORKSPACE_ROOT:-}"
STAMP="$(date +%Y%m%d-%H%M%S)"
TMP_DIR="$(mktemp -d)"

# Fast-lane defaults for quick local iteration; full stability gate remains authoritative.
MIN_WARM_NOOP_P95_DELTA_PERCENT="${MIN_WARM_NOOP_P95_DELTA_PERCENT:-10}"
MIN_WARM_EDIT_P95_DELTA_PERCENT="${MIN_WARM_EDIT_P95_DELTA_PERCENT:-0}"
MIN_WARM_EDIT_SEMANTIC_P95_DELTA_PERCENT="${MIN_WARM_EDIT_SEMANTIC_P95_DELTA_PERCENT:--10}"
MIN_COLD_P95_DELTA_PERCENT="${MIN_COLD_P95_DELTA_PERCENT:--80}"

cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

usage() {
  cat <<USAGE
Usage: $(basename "$0") [options]

Options:
  --matrix <research|smoke>      Matrix to run (default: smoke)
  --workspace-root <path>        Workspace root (required for research)
  --runs <n>                     Warm/offline iterations (default: 4)
  --cold-runs <n>                Cold iterations (default: 4)
  --build-online                 Measure build scenarios in online mode
  --uc-daemon-mode <off|require> UC daemon mode (default: require)
  --cpu-set <list>               Optional CPU affinity list
  --nice-level <n>               Optional nice level (default: 0)
  --strict-pinning               Require requested pinning to apply
  --host-preflight <mode>        Host preflight mode (off|warn|require, default: warn)
  --allow-noisy-host             Disable host preflight checks
  --help                         Show this help

Fast gate thresholds (env):
  MIN_WARM_NOOP_P95_DELTA_PERCENT       default: 10
  MIN_WARM_EDIT_P95_DELTA_PERCENT       default: 0
  MIN_WARM_EDIT_SEMANTIC_P95_DELTA_PERCENT default: -10
  MIN_COLD_P95_DELTA_PERCENT            default: -80
USAGE
}

require_option_value() {
  local flag="$1"
  local value="${2-}"
  if [[ -z "$value" || "$value" == --* ]]; then
    echo "Missing value for $flag" >&2
    usage
    exit 1
  fi
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --matrix)
      require_option_value "$1" "${2-}"
      MATRIX="$2"
      shift 2
      ;;
    --workspace-root)
      require_option_value "$1" "${2-}"
      WORKSPACE_ROOT="$2"
      shift 2
      ;;
    --runs)
      require_option_value "$1" "${2-}"
      RUNS="$2"
      shift 2
      ;;
    --cold-runs)
      require_option_value "$1" "${2-}"
      COLD_RUNS="$2"
      shift 2
      ;;
    --build-online)
      BUILD_OFFLINE=0
      shift
      ;;
    --uc-daemon-mode)
      require_option_value "$1" "${2-}"
      UC_DAEMON_MODE="$2"
      shift 2
      ;;
    --cpu-set)
      require_option_value "$1" "${2-}"
      CPU_SET="$2"
      shift 2
      ;;
    --nice-level)
      require_option_value "$1" "${2-}"
      NICE_LEVEL="$2"
      shift 2
      ;;
    --strict-pinning)
      STRICT_PINNING=1
      shift
      ;;
    --host-preflight)
      require_option_value "$1" "${2-}"
      HOST_PREFLIGHT_MODE="$2"
      shift 2
      ;;
    --allow-noisy-host)
      ALLOW_NOISY_HOST=1
      shift
      ;;
    --help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage
      exit 1
      ;;
  esac
done

if [[ "$RUNS" =~ [^0-9] || "$RUNS" -le 0 ]]; then
  echo "--runs must be a positive integer, got: $RUNS" >&2
  exit 1
fi
if [[ "$COLD_RUNS" =~ [^0-9] || "$COLD_RUNS" -le 0 ]]; then
  echo "--cold-runs must be a positive integer, got: $COLD_RUNS" >&2
  exit 1
fi
if [[ "$BUILD_OFFLINE" != "0" && "$BUILD_OFFLINE" != "1" ]]; then
  echo "BUILD_OFFLINE must be 0 or 1, got: $BUILD_OFFLINE" >&2
  exit 1
fi
if [[ "$UC_DAEMON_MODE" != "off" && "$UC_DAEMON_MODE" != "require" ]]; then
  echo "--uc-daemon-mode must be one of: off, require (got: $UC_DAEMON_MODE)" >&2
  exit 1
fi
if [[ ! "$NICE_LEVEL" =~ ^-?[0-9]+$ ]]; then
  echo "--nice-level must be an integer, got: $NICE_LEVEL" >&2
  exit 1
fi
if [[ "$HOST_PREFLIGHT_MODE" != "off" && "$HOST_PREFLIGHT_MODE" != "warn" && "$HOST_PREFLIGHT_MODE" != "require" ]]; then
  echo "--host-preflight must be one of: off, warn, require (got: $HOST_PREFLIGHT_MODE)" >&2
  exit 1
fi
if [[ "$ALLOW_NOISY_HOST" != "0" && "$ALLOW_NOISY_HOST" != "1" ]]; then
  echo "ALLOW_NOISY_HOST must be 0 or 1, got: $ALLOW_NOISY_HOST" >&2
  exit 1
fi
if [[ "$MATRIX" == "research" && -z "$WORKSPACE_ROOT" ]]; then
  echo "--workspace-root is required for research matrix" >&2
  exit 1
fi

mkdir -p "$OUT_DIR"

run_tool_benchmark() {
  local tool="$1"
  local log_file="$TMP_DIR/${tool}.log"
  local -a cmd=(
    "$ROOT_DIR/benchmarks/scripts/run_local_benchmarks.sh"
    --matrix "$MATRIX"
    --tool "$tool"
    --runs "$RUNS"
    --cold-runs "$COLD_RUNS"
    --uc-daemon-mode "$UC_DAEMON_MODE"
    --host-preflight "$HOST_PREFLIGHT_MODE"
  )

  if [[ "$BUILD_OFFLINE" == "0" ]]; then
    cmd+=(--build-online)
  fi
  if [[ -n "$CPU_SET" ]]; then
    cmd+=(--cpu-set "$CPU_SET")
  fi
  if [[ "$NICE_LEVEL" != "0" ]]; then
    cmd+=(--nice-level "$NICE_LEVEL")
  fi
  if [[ "$STRICT_PINNING" == "1" ]]; then
    cmd+=(--strict-pinning)
  fi
  if [[ "$ALLOW_NOISY_HOST" == "1" ]]; then
    cmd+=(--allow-noisy-host)
  fi
  if [[ -n "$WORKSPACE_ROOT" ]]; then
    cmd+=(--workspace-root "$WORKSPACE_ROOT")
  fi

  echo "== Running $tool benchmark ($MATRIX, runs=$RUNS, cold-runs=$COLD_RUNS) ==" >&2
  "${cmd[@]}" | tee "$log_file" >&2
  local json_path
  json_path="$(grep -F "Benchmark JSON:" "$log_file" | tail -n 1 | sed -E 's/^Benchmark JSON:[[:space:]]*//')"
  if [[ -z "$json_path" || ! -f "$json_path" ]]; then
    echo "Failed to discover Benchmark JSON path for tool '$tool'" >&2
    exit 1
  fi
  printf "%s" "$json_path"
}

SCARB_JSON="$(run_tool_benchmark scarb)"
UC_JSON="$(run_tool_benchmark uc)"
DELTA_MD="$OUT_DIR/perf-fast-delta-$STAMP.md"

"$ROOT_DIR/benchmarks/scripts/compare_benchmark_results.sh" \
  --baseline "$SCARB_JSON" \
  --candidate "$UC_JSON" \
  --out "$DELTA_MD" >/dev/null

SUMMARY_JSON="$TMP_DIR/summary.json"
jq -nr \
  --slurpfile baseline "$SCARB_JSON" \
  --slurpfile candidate "$UC_JSON" '
    ($baseline[0].scenarios
      | map({key: (.scenario + "|" + .workload), value: .})
      | from_entries) as $base_map
    | ($candidate[0].scenarios
      | map({key: (.scenario + "|" + .workload), value: .})
      | from_entries) as $cand_map
    | ($base_map | keys_unsorted | sort) as $keys
    | if $keys != ($cand_map | keys_unsorted | sort) then
        error("baseline/candidate scenario keys differ")
      else
        [ $keys[] as $key
          | ($key | split("|")) as $parts
          | ($base_map[$key].stats.p95_ms) as $baseline_p95
          | ($cand_map[$key].stats.p95_ms) as $candidate_p95
          | {
              scenario: $parts[0],
              workload: $parts[1],
              baseline_p95_ms: $baseline_p95,
              candidate_p95_ms: $candidate_p95,
              p95_delta_percent: (
                if $baseline_p95 == 0 then
                  0
                else
                  (($baseline_p95 - $candidate_p95) / $baseline_p95 * 100)
                end
              )
            }
        ]
      end
  ' > "$SUMMARY_JSON"

echo
echo "Fast perf summary (p95 deltas; positive means UC faster):"
jq -r '
  .[]
  | "- \(.scenario) / \(.workload): baseline p95=\(.baseline_p95_ms|round)ms, candidate p95=\(.candidate_p95_ms|round)ms, delta=\(.p95_delta_percent|round)%"
' "$SUMMARY_JSON"

violations=0
while IFS=$'\t' read -r scenario workload delta; do
  threshold=""
  case "$scenario" in
    build.warm_noop)
      threshold="$MIN_WARM_NOOP_P95_DELTA_PERCENT"
      ;;
    build.warm_edit)
      threshold="$MIN_WARM_EDIT_P95_DELTA_PERCENT"
      ;;
    build.warm_edit_semantic)
      threshold="$MIN_WARM_EDIT_SEMANTIC_P95_DELTA_PERCENT"
      ;;
    build.cold)
      threshold="$MIN_COLD_P95_DELTA_PERCENT"
      ;;
  esac
  if [[ -z "$threshold" ]]; then
    continue
  fi
  if ! awk "BEGIN { exit !($delta >= $threshold) }"; then
    echo "Fast gate violation: $scenario / $workload delta ${delta}% < ${threshold}%"
    violations=$((violations + 1))
  fi
done < <(jq -r '.[] | [.scenario, .workload, .p95_delta_percent] | @tsv' "$SUMMARY_JSON")

echo
echo "Artifacts:"
echo "- Baseline JSON: $SCARB_JSON"
echo "- Candidate JSON: $UC_JSON"
echo "- Delta report: $DELTA_MD"

if [[ "$violations" -gt 0 ]]; then
  echo "Fast perf check failed with $violations gate violation(s)." >&2
  exit 1
fi

echo "Fast perf check passed."

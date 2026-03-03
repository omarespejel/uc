#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd -P)"
MATRIX="${MATRIX:-research}"
RUNS="${RUNS:-12}"
COLD_RUNS="${COLD_RUNS:-12}"
CYCLES="${CYCLES:-5}"
WORKSPACE_ROOT="${WORKSPACE_ROOT:-}"
GATE_CONFIG="${GATE_CONFIG:-}"
BUILD_OFFLINE="${BUILD_OFFLINE:-1}"
UC_DAEMON_MODE="${UC_DAEMON_MODE:-off}"
CPU_SET="${CPU_SET:-${UC_BENCH_CPU_SET:-}}"
NICE_LEVEL="${NICE_LEVEL:-${UC_BENCH_NICE_LEVEL:-0}}"
STRICT_PINNING="${STRICT_PINNING:-${UC_BENCH_STRICT_PINNING:-0}}"
WARM_SETTLE_SECONDS="${WARM_SETTLE_SECONDS:-2.2}"
LOCK_BASELINE="${LOCK_BASELINE:-0}"
ALLOW_UNPINNED="${ALLOW_UNPINNED:-0}"
STAMP="$(date +%Y%m%d-%H%M%S)"
OUT_DIR="$ROOT_DIR/benchmarks/results"
OUT_JSON="$OUT_DIR/stability-summary-$STAMP.json"
OUT_MD="$OUT_DIR/stability-summary-$STAMP.md"
TMP_DIR="$(mktemp -d)"

cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

usage() {
  cat <<USAGE
Usage: $(basename "$0") [options]

Options:
  --matrix <research|smoke>    Benchmark matrix (default: research)
  --workspace-root <path>      Path containing local cloned repos (required for research)
  --runs <n>                   Warm/offline iterations per run (default: 12)
  --cold-runs <n>              Cold iterations per run (default: 12)
  --build-online               Measure build scenarios in online mode (default: offline)
  --uc-daemon-mode <mode>      UC daemon mode for uc tool (off|require, default: off)
  --cycles <n>                 Number of paired scarb/uc cycles (default: 5)
  --cpu-set <list>             Optional CPU affinity list passed to local runner
  --nice-level <n>             Optional process nice level passed to local runner
  --warm-settle-seconds <n>    Wait after warm-up before warm-noop samples (default: 2.2)
  --strict-pinning             Fail if requested pinning cannot be applied
  --allow-unpinned             Allow running without CPU affinity pinning safeguards
  --lock-baseline              Copy passing stability summary into benchmarks/baselines
  --gate-config <path>         Optional gate rules JSON path
  --help                       Show this help
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
    --cycles)
      require_option_value "$1" "${2-}"
      CYCLES="$2"
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
    --warm-settle-seconds)
      require_option_value "$1" "${2-}"
      WARM_SETTLE_SECONDS="$2"
      shift 2
      ;;
    --strict-pinning)
      STRICT_PINNING=1
      shift
      ;;
    --allow-unpinned)
      ALLOW_UNPINNED=1
      shift
      ;;
    --lock-baseline)
      LOCK_BASELINE=1
      shift
      ;;
    --gate-config)
      require_option_value "$1" "${2-}"
      GATE_CONFIG="$2"
      shift 2
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
if [[ "$CYCLES" =~ [^0-9] || "$CYCLES" -le 0 ]]; then
  echo "--cycles must be a positive integer, got: $CYCLES" >&2
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
if ! [[ "$WARM_SETTLE_SECONDS" =~ ^[0-9]+([.][0-9]+)?$ ]]; then
  echo "--warm-settle-seconds must be a positive number, got: $WARM_SETTLE_SECONDS" >&2
  exit 1
fi
if [[ "$MATRIX" == "research" && -z "$WORKSPACE_ROOT" ]]; then
  echo "--workspace-root is required for research matrix" >&2
  exit 1
fi
if [[ "$RUNS" -ne 12 || "$COLD_RUNS" -ne 12 ]]; then
  echo "Stability lane requires --runs 12 and --cold-runs 12 (got runs=$RUNS cold-runs=$COLD_RUNS)." >&2
  exit 1
fi
if [[ -z "$CPU_SET" || "$STRICT_PINNING" != "1" ]]; then
  if [[ "$ALLOW_UNPINNED" == "1" ]]; then
    echo "Stability warning: running in unpinned mode (cpu-set='${CPU_SET:-<unset>}', strict-pinning=$STRICT_PINNING)." >&2
  else
    echo "Stability lane requires --cpu-set and --strict-pinning for reproducibility." >&2
    echo "Use --allow-unpinned only when affinity pinning is unavailable on this host." >&2
    exit 1
  fi
fi
if [[ -n "$GATE_CONFIG" && ! -f "$GATE_CONFIG" ]]; then
  echo "Gate config not found: $GATE_CONFIG" >&2
  exit 1
fi

mkdir -p "$OUT_DIR"
: > "$TMP_DIR/cycle-summaries.ndjson"

run_benchmark() {
  local tool="$1"
  local cycle="$2"
  local log="$TMP_DIR/${tool}-cycle-${cycle}.log"
  local -a cmd=(
    "$ROOT_DIR/benchmarks/scripts/run_local_benchmarks.sh"
    --matrix "$MATRIX"
    --tool "$tool"
    --runs "$RUNS"
    --cold-runs "$COLD_RUNS"
    --warm-settle-seconds "$WARM_SETTLE_SECONDS"
    --uc-daemon-mode "$UC_DAEMON_MODE"
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
  if [[ -n "$WORKSPACE_ROOT" ]]; then
    cmd+=(--workspace-root "$WORKSPACE_ROOT")
  fi

  if [[ -n "$WORKSPACE_ROOT" ]]; then
    WORKSPACE_ROOT="$WORKSPACE_ROOT" "${cmd[@]}" | tee "$log" >&2
  else
    "${cmd[@]}" | tee "$log" >&2
  fi

  awk '/^Benchmark JSON:/ {print $3}' "$log" | tail -n1
}

for cycle in $(seq 1 "$CYCLES"); do
  echo "== Paired cycle $cycle/$CYCLES ==" >&2
  scarb_json=""
  uc_json=""
  if (( cycle % 2 == 1 )); then
    order=("scarb" "uc")
  else
    order=("uc" "scarb")
  fi
  for tool in "${order[@]}"; do
    result_json="$(run_benchmark "$tool" "$cycle")"
    if [[ "$tool" == "scarb" ]]; then
      scarb_json="$result_json"
    else
      uc_json="$result_json"
    fi
  done

  if [[ -z "$scarb_json" || -z "$uc_json" ]]; then
    echo "Failed to discover benchmark JSON output for cycle $cycle" >&2
    exit 1
  fi

  jq -n \
    --argjson cycle "$cycle" \
    --arg baseline "$scarb_json" \
    --arg candidate "$uc_json" \
    --slurpfile b "$scarb_json" \
    --slurpfile c "$uc_json" '
      ($b[0]) as $bb
      | ($c[0]) as $cc
      | (reduce $bb.scenarios[] as $item ({}; .[$item.scenario + "|" + $item.workload] = $item)) as $bm
      | (reduce $cc.scenarios[] as $item ({}; .[$item.scenario + "|" + $item.workload] = $item)) as $cm
      | ($bm | keys_unsorted | sort) as $bk
      | ($cm | keys_unsorted | sort) as $ck
      | if $bk != $ck then
          error("baseline/candidate scenario keys differ")
        else
          {
            cycle: $cycle,
            run_order: (if ($cycle % 2) == 1 then "scarb-first" else "uc-first" end),
            baseline_json: $baseline,
            candidate_json: $candidate,
            scenarios: [
              $bk[]
              | . as $k
              | ($bm[$k]) as $base
              | ($cm[$k]) as $cand
              | ($base.stats.p50_ms) as $bp50
              | ($cand.stats.p50_ms) as $cp50
              | ($base.stats.p95_ms) as $bp95
              | ($cand.stats.p95_ms) as $cp95
              | {
                  scenario: $base.scenario,
                  workload: $base.workload,
                  baseline_p50_ms: $bp50,
                  candidate_p50_ms: $cp50,
                  p50_delta_percent: (if $bp50 == 0 then 0 else (($bp50 - $cp50) / $bp50 * 100) end),
                  baseline_p95_ms: $bp95,
                  candidate_p95_ms: $cp95,
                  p95_delta_percent: (if $bp95 == 0 then 0 else (($bp95 - $cp95) / $bp95 * 100) end)
                }
            ]
          }
        end
    ' >> "$TMP_DIR/cycle-summaries.ndjson"
done

jq -s \
  --arg generated_at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --arg matrix "$MATRIX" \
  --argjson runs "$RUNS" \
  --argjson cold_runs "$COLD_RUNS" \
  --argjson cycles "$CYCLES" \
  --arg workspace_root "${WORKSPACE_ROOT:-}" \
  --arg cpu_set "$CPU_SET" \
  --arg nice_level "$NICE_LEVEL" \
  --arg build_offline "$BUILD_OFFLINE" \
  --arg uc_daemon_mode "$UC_DAEMON_MODE" \
  --arg strict_pinning "$STRICT_PINNING" \
  --arg warm_settle_seconds "$WARM_SETTLE_SECONDS" '
    def median(a):
      (a | sort) as $s
      | ($s | length) as $n
      | if $n == 0 then null
        elif ($n % 2) == 1 then $s[($n / 2 | floor)]
        else (($s[($n / 2 | floor) - 1] + $s[($n / 2 | floor)]) / 2)
        end;
    def mean(a):
      if (a | length) == 0 then 0 else (a | add) / (a | length) end;
    def stdev(a):
      if (a | length) < 2 then 0
      else (mean(a) as $m | ([a[] | ((. - $m) * (. - $m))] | add / (a | length) | sqrt))
      end;
    {
      generated_at: $generated_at,
      matrix: $matrix,
      workspace_root: (if $workspace_root == "" then null else $workspace_root end),
      runs: $runs,
      cold_runs: $cold_runs,
      cycles: $cycles,
      pinned_conditions: {
        cpu_set: (if $cpu_set == "" then null else $cpu_set end),
        nice_level: ($nice_level | tonumber),
        build_offline: ($build_offline == "1"),
        uc_daemon_mode: $uc_daemon_mode,
        strict_pinning: ($strict_pinning == "1"),
        warm_settle_seconds: ($warm_settle_seconds | tonumber)
      },
      cycle_reports: map({
        cycle,
        run_order,
        baseline_json,
        candidate_json
      }),
      scenarios: (
        reduce .[] as $cycle ({}; reduce $cycle.scenarios[] as $item (.;
          ($item.scenario + "|" + $item.workload) as $k
          | .[$k].scenario = $item.scenario
          | .[$k].workload = $item.workload
          | .[$k].p50_deltas = ((.[$k].p50_deltas // []) + [$item.p50_delta_percent])
          | .[$k].p95_deltas = ((.[$k].p95_deltas // []) + [$item.p95_delta_percent])
          | .[$k].baseline_p95 = ((.[$k].baseline_p95 // []) + [$item.baseline_p95_ms])
          | .[$k].candidate_p95 = ((.[$k].candidate_p95 // []) + [$item.candidate_p95_ms])
        ))
        | to_entries
        | map(
            . as $entry
            | $entry.value + {
                median_p50_delta_percent: median($entry.value.p50_deltas),
                median_p95_delta_percent: median($entry.value.p95_deltas),
                mean_p95_delta_percent: mean($entry.value.p95_deltas),
                stdev_p95_delta_percent: stdev($entry.value.p95_deltas),
                min_p95_delta_percent: ($entry.value.p95_deltas | min),
                max_p95_delta_percent: ($entry.value.p95_deltas | max),
                median_baseline_p95_ms: median($entry.value.baseline_p95),
                median_candidate_p95_ms: median($entry.value.candidate_p95)
              }
          )
        | sort_by(.scenario, .workload)
      )
    }
  ' "$TMP_DIR/cycle-summaries.ndjson" > "$OUT_JSON"

{
  echo "# Stability Benchmark Summary ($STAMP)"
  echo
  echo "- Generated at: $(jq -r '.generated_at' "$OUT_JSON")"
  echo "- Matrix: $(jq -r '.matrix' "$OUT_JSON")"
  echo "- Cycles: $(jq -r '.cycles' "$OUT_JSON")"
  echo "- Runs: $(jq -r '.runs' "$OUT_JSON")"
  echo "- Cold runs: $(jq -r '.cold_runs' "$OUT_JSON")"
  echo "- CPU set: $(jq -r '.pinned_conditions.cpu_set // "<none>"' "$OUT_JSON")"
  echo "- Nice level: $(jq -r '.pinned_conditions.nice_level' "$OUT_JSON")"
  echo "- Build mode: $(jq -r 'if .pinned_conditions.build_offline then "offline" else "online" end' "$OUT_JSON")"
  echo "- UC daemon mode: $(jq -r '.pinned_conditions.uc_daemon_mode' "$OUT_JSON")"
  echo "- Run order: alternates per cycle (scarb-first, uc-first)"
  echo "- Strict pinning: $(jq -r '.pinned_conditions.strict_pinning' "$OUT_JSON")"
  echo "- Warm settle seconds: $(jq -r '.pinned_conditions.warm_settle_seconds' "$OUT_JSON")"
  if [[ -n "$WORKSPACE_ROOT" ]]; then
    echo "- Workspace root: $WORKSPACE_ROOT"
  fi
  echo
  echo "| Scenario | Workload | Median p95 delta % | Mean p95 delta % | p95 delta stdev | Min p95 delta % | Max p95 delta % |"
  echo "|---|---|---:|---:|---:|---:|---:|"
  jq -r '
    def r2: ((. * 100 | round) / 100);
    .scenarios[]
    | "| \(.scenario) | \(.workload) | \(.median_p95_delta_percent | r2) | \(.mean_p95_delta_percent | r2) | \(.stdev_p95_delta_percent | r2) | \(.min_p95_delta_percent | r2) | \(.max_p95_delta_percent | r2) |"
  ' "$OUT_JSON"
} > "$OUT_MD"

echo "Stability JSON: $OUT_JSON"
echo "Stability Markdown: $OUT_MD"

if [[ -n "$GATE_CONFIG" ]]; then
  "$ROOT_DIR/benchmarks/scripts/gate_benchmark_summary.sh" \
    --summary "$OUT_JSON" \
    --config "$GATE_CONFIG"
fi

if [[ "$LOCK_BASELINE" == "1" ]]; then
  BASELINE_DIR="$ROOT_DIR/benchmarks/baselines"
  mkdir -p "$BASELINE_DIR"
  LOCKED_JSON="$BASELINE_DIR/stability-${MATRIX}-latest.json"
  LOCKED_MD="$BASELINE_DIR/stability-${MATRIX}-latest.md"
  jq '
    def rel_result_path:
      if test("/benchmarks/results/")
      then sub("^.*/benchmarks/results/"; "benchmarks/results/")
      else .
      end;
    .cycle_reports |= map(
      .baseline_json |= rel_result_path
      | .candidate_json |= rel_result_path
    )
  ' "$OUT_JSON" > "$LOCKED_JSON"
  cp "$OUT_MD" "$LOCKED_MD"
  echo "Locked baseline JSON: $LOCKED_JSON"
  echo "Locked baseline Markdown: $LOCKED_MD"
fi

#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(git -C "$SCRIPT_DIR/../.." rev-parse --show-toplevel 2>/dev/null || (cd "$SCRIPT_DIR/../.." && pwd -P))"
MATRIX="${MATRIX:-research}"
RUNS="${RUNS:-10}"
COLD_RUNS="${COLD_RUNS:-5}"
CYCLES="${CYCLES:-5}"
WORKSPACE_ROOT="${WORKSPACE_ROOT:-}"
GATE_CONFIG="${GATE_CONFIG:-}"
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
  --runs <n>                   Warm/offline iterations per run (default: 10)
  --cold-runs <n>              Cold iterations per run (default: 5)
  --cycles <n>                 Number of paired scarb/uc cycles (default: 5)
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
    --cycles)
      require_option_value "$1" "${2-}"
      CYCLES="$2"
      shift 2
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
if [[ "$MATRIX" == "research" && -z "$WORKSPACE_ROOT" ]]; then
  echo "--workspace-root is required for research matrix" >&2
  exit 1
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
    zsh
    "$ROOT_DIR/benchmarks/scripts/run_local_benchmarks.sh"
    --matrix "$MATRIX"
    --tool "$tool"
    --runs "$RUNS"
    --cold-runs "$COLD_RUNS"
  )
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
  scarb_json="$(run_benchmark scarb "$cycle")"
  uc_json="$(run_benchmark uc "$cycle")"

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
  --arg workspace_root "${WORKSPACE_ROOT:-}" '
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
      cycle_reports: map({
        cycle,
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

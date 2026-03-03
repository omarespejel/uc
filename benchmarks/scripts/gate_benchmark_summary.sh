#!/usr/bin/env bash
set -euo pipefail

SUMMARY=""
CONFIG=""

usage() {
  cat <<USAGE
Usage: $(basename "$0") --summary <stability-summary.json> --config <gate-config.json>
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --summary)
      SUMMARY="$2"
      shift 2
      ;;
    --config)
      CONFIG="$2"
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

if [[ -z "$SUMMARY" || -z "$CONFIG" ]]; then
  usage
  exit 1
fi
if [[ ! -f "$SUMMARY" ]]; then
  echo "Summary JSON not found: $SUMMARY" >&2
  exit 1
fi
if [[ ! -f "$CONFIG" ]]; then
  echo "Config JSON not found: $CONFIG" >&2
  exit 1
fi

violations="$(jq -n --slurpfile s "$SUMMARY" --slurpfile c "$CONFIG" '
  ($s[0].scenarios // []) as $scenarios
  | ($c[0].rules // []) as $rules
  | [
      $rules[] as $rule
      | ([ $scenarios[] | select(.scenario == $rule.scenario and .workload == $rule.workload) ][0]) as $metric
      | if $metric == null then
          {
            scenario: $rule.scenario,
            workload: $rule.workload,
            kind: "missing",
            message: "scenario/workload missing from summary"
          }
        else
          (
            [
              if ($rule | has("min_median_p95_delta_percent")) and
                 ($metric.median_p95_delta_percent < $rule.min_median_p95_delta_percent)
              then {
                scenario: $rule.scenario,
                workload: $rule.workload,
                kind: "below_min",
                actual: $metric.median_p95_delta_percent,
                expected: $rule.min_median_p95_delta_percent,
                message: (
                  "median p95 delta "
                  + ((($metric.median_p95_delta_percent * 100 | round) / 100) | tostring)
                  + " < min "
                  + ((($rule.min_median_p95_delta_percent * 100 | round) / 100) | tostring)
                )
              } else empty end,
              if ($rule | has("max_median_p95_delta_percent")) and
                 ($metric.median_p95_delta_percent > $rule.max_median_p95_delta_percent)
              then {
                scenario: $rule.scenario,
                workload: $rule.workload,
                kind: "above_max",
                actual: $metric.median_p95_delta_percent,
                expected: $rule.max_median_p95_delta_percent,
                message: (
                  "median p95 delta "
                  + ((($metric.median_p95_delta_percent * 100 | round) / 100) | tostring)
                  + " > max "
                  + ((($rule.max_median_p95_delta_percent * 100 | round) / 100) | tostring)
                )
              } else empty end,
              if ($rule | has("min_single_cycle_p95_delta_percent")) and
                 ($metric.min_p95_delta_percent < $rule.min_single_cycle_p95_delta_percent)
              then {
                scenario: $rule.scenario,
                workload: $rule.workload,
                kind: "single_cycle_below_min",
                actual: $metric.min_p95_delta_percent,
                expected: $rule.min_single_cycle_p95_delta_percent,
                message: (
                  "worst single-cycle p95 delta "
                  + ((($metric.min_p95_delta_percent * 100 | round) / 100) | tostring)
                  + " < min "
                  + ((($rule.min_single_cycle_p95_delta_percent * 100 | round) / 100) | tostring)
                )
              } else empty end,
              if ($rule | has("max_single_cycle_p95_delta_percent")) and
                 ($metric.max_p95_delta_percent > $rule.max_single_cycle_p95_delta_percent)
              then {
                scenario: $rule.scenario,
                workload: $rule.workload,
                kind: "single_cycle_above_max",
                actual: $metric.max_p95_delta_percent,
                expected: $rule.max_single_cycle_p95_delta_percent,
                message: (
                  "best single-cycle p95 delta "
                  + ((($metric.max_p95_delta_percent * 100 | round) / 100) | tostring)
                  + " > max "
                  + ((($rule.max_single_cycle_p95_delta_percent * 100 | round) / 100) | tostring)
                )
              } else empty end
            ]
          )[]
        end
    ]
  ' )"

violation_count="$(jq 'length' <<<"$violations")"
echo "Benchmark gate violations: $violation_count"

if [[ "$violation_count" -gt 0 ]]; then
  jq -r '.[] | "- \(.scenario) / \(.workload): \(.message)"' <<<"$violations" >&2
  exit 1
fi

echo "Benchmark gate passed."

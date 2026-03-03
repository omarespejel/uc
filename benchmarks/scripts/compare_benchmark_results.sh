#!/usr/bin/env zsh
set -euo pipefail

BASELINE=""
CANDIDATE=""
OUT=""

usage() {
  cat <<USAGE
Usage: $(basename "$0") --baseline <baseline.json> --candidate <candidate.json> --out <report.md>
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --baseline)
      BASELINE="$2"
      shift 2
      ;;
    --candidate)
      CANDIDATE="$2"
      shift 2
      ;;
    --out)
      OUT="$2"
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

if [[ -z "$BASELINE" || -z "$CANDIDATE" || -z "$OUT" ]]; then
  usage
  exit 1
fi

if [[ ! -f "$BASELINE" || ! -f "$CANDIDATE" ]]; then
  echo "Baseline or candidate JSON file not found" >&2
  exit 1
fi

mkdir -p "$(dirname "$OUT")"

BASELINE_TOOL="$(jq -r '.tool_version' "$BASELINE")"
CANDIDATE_TOOL="$(jq -r '.tool_version' "$CANDIDATE")"
TIMESTAMP="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

{
  echo "# Benchmark Delta Report"
  echo
  echo "- Generated at: $TIMESTAMP"
  echo "- Baseline: $BASELINE_TOOL"
  echo "- Candidate: $CANDIDATE_TOOL"
  echo
  echo "| Scenario | Workload | Baseline p50 (ms) | Candidate p50 (ms) | p50 delta % | Baseline p95 (ms) | Candidate p95 (ms) | p95 delta % |"
  echo "|---|---|---:|---:|---:|---:|---:|---:|"

  jq -nr \
    --slurpfile b "$BASELINE" \
    --slurpfile c "$CANDIDATE" '
      ($b[0]) as $bb
      | ($c[0]) as $cc
      | (reduce $bb.scenarios[] as $item ({}; .[$item.scenario + "|" + $item.workload] = $item)) as $bm
      | (reduce $cc.scenarios[] as $item ({}; .[$item.scenario + "|" + $item.workload] = $item)) as $cm
      | ($bm + $cm | keys_unsorted | unique | sort)[] as $k
      | ($bm[$k]) as $base
      | ($cm[$k]) as $cand
      | select($base != null and $cand != null)
      | ($base.scenario) as $scenario
      | ($base.workload) as $workload
      | ($base.stats.p50_ms) as $bp50
      | ($cand.stats.p50_ms) as $cp50
      | ($base.stats.p95_ms) as $bp95
      | ($cand.stats.p95_ms) as $cp95
      | (if $bp50 == 0 then 0 else (($bp50 - $cp50) / $bp50 * 100) end) as $p50d
      | (if $bp95 == 0 then 0 else (($bp95 - $cp95) / $bp95 * 100) end) as $p95d
      | [
          $scenario,
          $workload,
          (($bp50 * 1000 | round) / 1000),
          (($cp50 * 1000 | round) / 1000),
          (($p50d * 100 | round) / 100),
          (($bp95 * 1000 | round) / 1000),
          (($cp95 * 1000 | round) / 1000),
          (($p95d * 100 | round) / 100)
        ]
      | @tsv
    ' | while IFS=$'\t' read -r scenario workload bp50 cp50 p50d bp95 cp95 p95d; do
      printf "| %s | %s | %s | %s | %s | %s | %s | %s |\n" \
        "$scenario" "$workload" "$bp50" "$cp50" "$p50d" "$bp95" "$cp95" "$p95d"
    done
} > "$OUT"

echo "Delta report: $OUT"

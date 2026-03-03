#!/usr/bin/env zsh
set -euo pipefail
zmodload zsh/datetime

ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
DEFAULT_WORKSPACE_ROOT="/Users/espejelomar/StarkNet/compiler-starknet"
WORKSPACE_ROOT="${WORKSPACE_ROOT:-$DEFAULT_WORKSPACE_ROOT}"
MATRIX="${MATRIX:-research}"
RUNS="${RUNS:-5}"
COLD_RUNS="${COLD_RUNS:-3}"
STAMP="$(date +%Y%m%d-%H%M%S)"
OUT_DIR="$ROOT_DIR/benchmarks/results"
OUT_JSON="$OUT_DIR/scarb-baseline-$STAMP.json"
OUT_MD="$OUT_DIR/scarb-baseline-$STAMP.md"
TMP_DIR="$(mktemp -d)"

cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

usage() {
  cat <<USAGE
Usage: $(basename "$0") [options]

Options:
  --matrix <research|smoke>   Scenario matrix to run (default: research)
  --workspace-root <path>     Path containing local cloned repos
  --runs <n>                  Runs for warm/offline scenarios (default: 5)
  --cold-runs <n>             Runs for cold scenarios (default: 3)
  --help                      Show this help
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --matrix)
      MATRIX="$2"
      shift 2
      ;;
    --workspace-root)
      WORKSPACE_ROOT="$2"
      shift 2
      ;;
    --runs)
      RUNS="$2"
      shift 2
      ;;
    --cold-runs)
      COLD_RUNS="$2"
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

require_cmd() {
  local cmd="$1"
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "Required command missing: $cmd" >&2
    exit 1
  fi
}

require_cmd scarb
require_cmd jq
require_cmd awk
require_cmd sort

mkdir -p "$OUT_DIR"
: > "$TMP_DIR/scenarios.ndjson"

measure_command_ms() {
  local cwd="$1"
  local command="$2"
  local stderr_file="$TMP_DIR/stderr.log"

  local start="$EPOCHREALTIME"
  if ! (cd "$cwd" && eval "$command" >/dev/null 2>"$stderr_file"); then
    echo "Command failed in $cwd: $command" >&2
    cat "$stderr_file" >&2
    exit 1
  fi
  local end="$EPOCHREALTIME"

  awk -v s="$start" -v e="$end" 'BEGIN { printf "%.3f\n", (e - s) * 1000 }'
}

stats_json_from_samples() {
  local samples_file="$1"
  jq -s '
    sort as $s
    | def pidx($p): (((length * $p) | ceil) - 1);
    {
      min_ms: ($s | min),
      max_ms: ($s | max),
      mean_ms: (add / length),
      p50_ms: $s[pidx(0.50)],
      p95_ms: $s[pidx(0.95)]
    }
  ' "$samples_file"
}

append_result() {
  local scenario="$1"
  local workload="$2"
  local command="$3"
  local samples_file="$4"
  local runs="$5"
  local stats_json
  local samples_json

  stats_json="$(stats_json_from_samples "$samples_file")"
  samples_json="$(jq -s '.' "$samples_file")"

  jq -n \
    --arg scenario "$scenario" \
    --arg workload "$workload" \
    --arg command "$command" \
    --argjson runs "$runs" \
    --argjson samples_ms "$samples_json" \
    --argjson stats "$stats_json" \
    '{
      scenario: $scenario,
      workload: $workload,
      command: $command,
      runs: $runs,
      samples_ms: $samples_ms,
      stats: $stats
    }' >> "$TMP_DIR/scenarios.ndjson"
}

run_build_cold() {
  local workload="$1"
  local cwd="$2"
  local command="$3"
  local runs="$4"
  local samples_file="$TMP_DIR/${workload//\//_}-build-cold.samples"
  : > "$samples_file"

  for i in $(seq 1 "$runs"); do
    rm -rf "$cwd/target" "$cwd/.scarb"
    measure_command_ms "$cwd" "$command" >> "$samples_file"
  done

  append_result "build.cold" "$workload" "$command" "$samples_file" "$runs"
}

run_build_warm_noop() {
  local workload="$1"
  local cwd="$2"
  local command="$3"
  local runs="$4"
  local samples_file="$TMP_DIR/${workload//\//_}-build-warm-noop.samples"
  : > "$samples_file"

  measure_command_ms "$cwd" "$command" > /dev/null
  for i in $(seq 1 "$runs"); do
    measure_command_ms "$cwd" "$command" >> "$samples_file"
  done

  append_result "build.warm_noop" "$workload" "$command" "$samples_file" "$runs"
}

run_build_warm_edit() {
  local workload="$1"
  local cwd="$2"
  local command="$3"
  local edit_file="$4"
  local runs="$5"
  local samples_file="$TMP_DIR/${workload//\//_}-build-warm-edit.samples"
  local backup_file="$TMP_DIR/${workload//\//_}-edit.backup"
  : > "$samples_file"

  cp "$edit_file" "$backup_file"
  measure_command_ms "$cwd" "$command" > /dev/null

  for i in $(seq 1 "$runs"); do
    cp "$backup_file" "$edit_file"
    printf "\n// uc benchmark edit %s %s\n" "$i" "$STAMP" >> "$edit_file"
    measure_command_ms "$cwd" "$command" >> "$samples_file"
  done

  cp "$backup_file" "$edit_file"
  append_result "build.warm_edit" "$workload" "$command" "$samples_file" "$runs"
}

run_metadata_online_cold() {
  local workload="$1"
  local cwd="$2"
  local cache_dir="$3"
  local runs="$4"
  local samples_file="$TMP_DIR/${workload//\//_}-metadata-online-cold.samples"
  : > "$samples_file"

  local command="scarb --global-cache-dir '$cache_dir' metadata --format-version 1"

  for i in $(seq 1 "$runs"); do
    rm -rf "$cache_dir"
    measure_command_ms "$cwd" "$command" >> "$samples_file"
  done

  append_result "metadata.online_cold" "$workload" "$command" "$samples_file" "$runs"
}

run_metadata_offline_warm() {
  local workload="$1"
  local cwd="$2"
  local cache_dir="$3"
  local runs="$4"
  local samples_file="$TMP_DIR/${workload//\//_}-metadata-offline-warm.samples"
  : > "$samples_file"

  local warm_command="scarb --global-cache-dir '$cache_dir' metadata --format-version 1"
  local command="scarb --global-cache-dir '$cache_dir' --offline metadata --format-version 1"

  rm -rf "$cache_dir"
  measure_command_ms "$cwd" "$warm_command" > /dev/null

  for i in $(seq 1 "$runs"); do
    measure_command_ms "$cwd" "$command" >> "$samples_file"
  done

  append_result "metadata.offline_warm" "$workload" "$command" "$samples_file" "$runs"
}

if [[ "$MATRIX" == "research" ]]; then
  HELLO_DIR="$WORKSPACE_ROOT/scarb/examples/hello_world"
  WS_DIR="$WORKSPACE_ROOT/scarb/examples/workspaces"
  DEPS_DIR="$WORKSPACE_ROOT/scarb/examples/dependencies"

  if [[ ! -d "$HELLO_DIR" || ! -d "$WS_DIR" || ! -d "$DEPS_DIR" ]]; then
    echo "Research matrix directories not found under: $WORKSPACE_ROOT" >&2
    echo "Expected: scarb/examples/hello_world, scarb/examples/workspaces, scarb/examples/dependencies" >&2
    exit 1
  fi

  run_build_cold "hello_world" "$HELLO_DIR" "scarb build" "$COLD_RUNS"
  run_build_warm_noop "hello_world" "$HELLO_DIR" "scarb build" "$RUNS"
  run_build_warm_edit "hello_world" "$HELLO_DIR" "scarb build" "$HELLO_DIR/src/lib.cairo" "$RUNS"

  run_build_cold "workspaces" "$WS_DIR" "scarb build" "$COLD_RUNS"
  run_build_warm_noop "workspaces" "$WS_DIR" "scarb build" "$RUNS"
  run_build_warm_edit "workspaces" "$WS_DIR" "scarb build" "$WS_DIR/crates/fibonacci/src/lib.cairo" "$RUNS"

  run_metadata_online_cold "dependencies" "$DEPS_DIR" "$TMP_DIR/deps-cache" "$COLD_RUNS"
  run_metadata_offline_warm "dependencies" "$DEPS_DIR" "$TMP_DIR/deps-cache" "$RUNS"
elif [[ "$MATRIX" == "smoke" ]]; then
  SMOKE_DIR="$ROOT_DIR/benchmarks/fixtures/scarb_smoke"
  run_build_cold "scarb_smoke" "$SMOKE_DIR" "scarb build" 1
  run_build_warm_noop "scarb_smoke" "$SMOKE_DIR" "scarb build" 2
  run_build_warm_edit "scarb_smoke" "$SMOKE_DIR" "scarb build" "$SMOKE_DIR/src/lib.cairo" 2
else
  echo "Unsupported matrix: $MATRIX" >&2
  exit 1
fi

SCARB_VERSION="$(scarb --version | head -n1)"
GENERATED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
HOST_NAME="$(hostname)"
UNAME_STR="$(uname -a)"

jq -s \
  --arg generated_at "$GENERATED_AT" \
  --arg matrix "$MATRIX" \
  --arg host "$HOST_NAME" \
  --arg uname "$UNAME_STR" \
  --arg scarb_version "$SCARB_VERSION" \
  --arg workspace_root "$WORKSPACE_ROOT" \
  --argjson runs "$RUNS" \
  --argjson cold_runs "$COLD_RUNS" \
  '{
    generated_at: $generated_at,
    matrix: $matrix,
    host: $host,
    uname: $uname,
    tool: "scarb",
    scarb_version: $scarb_version,
    workspace_root: $workspace_root,
    runs: $runs,
    cold_runs: $cold_runs,
    scenarios: .
  }' "$TMP_DIR/scenarios.ndjson" > "$OUT_JSON"

{
  echo "# Scarb Baseline Benchmark ($STAMP)"
  echo
  echo "## Environment"
  echo "- Generated at: $GENERATED_AT"
  echo "- Matrix: $MATRIX"
  echo "- Host: $HOST_NAME"
  echo "- Tool: $SCARB_VERSION"
  echo "- Workspace root: $WORKSPACE_ROOT"
  echo
  echo "## Summary"
  echo "| Scenario | Workload | Runs | p50 (ms) | p95 (ms) | mean (ms) | min (ms) | max (ms) |"
  echo "|---|---|---:|---:|---:|---:|---:|---:|"
  jq -r '
    def r3: ((. * 1000 | round) / 1000);
    .scenarios[]
    | "| \(.scenario) | \(.workload) | \(.runs) | \(.stats.p50_ms | r3) | \(.stats.p95_ms | r3) | \(.stats.mean_ms | r3) | \(.stats.min_ms | r3) | \(.stats.max_ms | r3) |"
  ' "$OUT_JSON"
} > "$OUT_MD"

echo "Benchmark JSON: $OUT_JSON"
echo "Benchmark Markdown: $OUT_MD"

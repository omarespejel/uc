#!/usr/bin/env zsh
# Requires zsh (arrays, extended globs, and zsh/datetime module).
set -euo pipefail
zmodload zsh/datetime
typeset -gi MONO_LAST_US=0
typeset -gi MONO_OFFSET_US=0

SCRIPT_DIR="$(cd "$(dirname "${(%):-%N}")" && pwd -P)"
ROOT_DIR="$(git -C "$SCRIPT_DIR/../.." rev-parse --show-toplevel 2>/dev/null || (cd "$SCRIPT_DIR/../.." && pwd -P))"
WORKSPACE_ROOT="${WORKSPACE_ROOT:-}"
MATRIX="${MATRIX:-research}"
TOOL="${TOOL:-scarb}"
RUNS="${RUNS:-5}"
COLD_RUNS="${COLD_RUNS:-3}"
STAMP="$(date +%Y%m%d-%H%M%S)"
OUT_DIR="$ROOT_DIR/benchmarks/results"
OUT_JSON=""
OUT_MD=""
TMP_DIR="$(mktemp -d)"
UC_BIN="$ROOT_DIR/target/debug/uc"
UC_DAEMON_SOCKET_PATH="${UC_DAEMON_SOCKET_PATH:-$TMP_DIR/uc-daemon.sock}"
export UC_DAEMON_SOCKET_PATH

cleanup() {
  if [[ "$TOOL" == "uc" && -x "$UC_BIN" ]]; then
    UC_DAEMON_SOCKET_PATH="$UC_DAEMON_SOCKET_PATH" "$UC_BIN" daemon stop >/dev/null 2>&1 || true
  fi
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

usage() {
  cat <<USAGE
Usage: $(basename "$0") [options]

Options:
  --matrix <research|smoke>   Scenario matrix to run (default: research)
  --tool <scarb|uc>           Tool under test (default: scarb)
  --workspace-root <path>     Path containing local cloned repos
  --runs <n>                  Runs for warm/offline scenarios (default: 5)
  --cold-runs <n>             Runs for cold scenarios (default: 3)
  --help                      Show this help
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
    --tool)
      require_option_value "$1" "${2-}"
      TOOL="$2"
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

if [[ "$RUNS" != <-> || "$RUNS" -le 0 ]]; then
  echo "--runs must be a positive integer, got: $RUNS" >&2
  exit 1
fi
if [[ "$COLD_RUNS" != <-> || "$COLD_RUNS" -le 0 ]]; then
  echo "--cold-runs must be a positive integer, got: $COLD_RUNS" >&2
  exit 1
fi

OUT_JSON="$OUT_DIR/${TOOL}-baseline-$STAMP.json"
OUT_MD="$OUT_DIR/${TOOL}-baseline-$STAMP.md"

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

if [[ "$TOOL" != "scarb" && "$TOOL" != "uc" ]]; then
  echo "Unsupported tool: $TOOL" >&2
  exit 1
fi

if [[ "$TOOL" == "uc" ]]; then
  require_cmd cargo
  (cd "$ROOT_DIR" && cargo build -p uc-cli >/dev/null)
  UC_DAEMON_SOCKET_PATH="$UC_DAEMON_SOCKET_PATH" "$UC_BIN" daemon start >/dev/null
fi

mkdir -p "$OUT_DIR"
: > "$TMP_DIR/scenarios.ndjson"

sanitize_for_report() {
  local value="$1"
  value="${value//$TMP_DIR/<tmp-dir>}"
  value="${value//$ROOT_DIR/<repo-root>}"
  if [[ -n "$WORKSPACE_ROOT" ]]; then
    value="${value//$WORKSPACE_ROOT/<workspace-root>}"
  fi
  printf "%s" "$value"
}

workspace_root_for_report() {
  if [[ -n "$WORKSPACE_ROOT" ]]; then
    sanitize_for_report "$WORKSPACE_ROOT"
  else
    printf "%s" "<workspace-root-unset>"
  fi
}

command_to_string() {
  local -a argv=("$@")
  local -a escaped=()
  local arg
  for arg in "${argv[@]}"; do
    escaped+=("${(q)arg}")
  done
  printf "%s" "${(j: :)escaped}"
}

monotonic_now_us() {
  # Use zsh's high-resolution clock to avoid per-sample subprocess overhead.
  local now="$EPOCHREALTIME"
  local sec="${now%%.*}"
  local frac="${now#*.}"
  frac="${frac}000000"
  frac="${frac[1,6]}"
  local raw_us=$((10#$sec * 1000000 + 10#$frac))
  local adjusted_us=$((raw_us + MONO_OFFSET_US))
  if (( adjusted_us < MONO_LAST_US )); then
    MONO_OFFSET_US=$((MONO_OFFSET_US + MONO_LAST_US - adjusted_us))
    adjusted_us=$((raw_us + MONO_OFFSET_US))
  fi
  if (( adjusted_us < MONO_LAST_US )); then
    adjusted_us=$MONO_LAST_US
  fi
  MONO_LAST_US=$adjusted_us
  printf "%s" "$adjusted_us"
}

measure_command_ms() {
  local cwd="$1"
  shift
  local -a argv=("$@")
  local stderr_file="$TMP_DIR/stderr.log"
  local display

  if [[ ${#argv[@]} -eq 0 ]]; then
    echo "Command parse failed: empty argv" >&2
    return 1
  fi

  display="$(command_to_string "${argv[@]}")"

  local start_us
  local end_us
  local original_dir
  original_dir="$(pwd -P)"
  start_us="$(monotonic_now_us)"
  if ! cd "$cwd"; then
    echo "Failed to enter benchmark directory: $cwd" >&2
    return 1
  fi
  if ! "${argv[@]}" >/dev/null 2>"$stderr_file"; then
    cd "$original_dir" >/dev/null 2>&1 || true
    echo "Command failed in $cwd: $display" >&2
    cat "$stderr_file" >&2
    return 1
  fi
  cd "$original_dir" >/dev/null 2>&1 || true
  end_us="$(monotonic_now_us)"

  awk -v s="$start_us" -v e="$end_us" 'BEGIN { printf "%.3f\n", (e - s) / 1000 }'
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
  local report_command

  stats_json="$(stats_json_from_samples "$samples_file")"
  samples_json="$(jq -s '.' "$samples_file")"
  report_command="$(sanitize_for_report "$command")"

  jq -n \
    --arg scenario "$scenario" \
    --arg workload "$workload" \
    --arg command "$report_command" \
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

build_command_for_manifest() {
  local manifest="$1"
  if [[ "$TOOL" == "scarb" ]]; then
    reply=(scarb --manifest-path "$manifest" build)
  else
    reply=("$UC_BIN" build --engine uc --daemon-mode require --manifest-path "$manifest")
  fi
}

metadata_online_command_for_manifest() {
  local manifest="$1"
  local cache_dir="$2"
  if [[ "$TOOL" == "scarb" ]]; then
    reply=(scarb --manifest-path "$manifest" --global-cache-dir "$cache_dir" metadata --format-version 1)
  else
    reply=("$UC_BIN" metadata --daemon-mode require --manifest-path "$manifest" --global-cache-dir "$cache_dir" --format-version 1)
  fi
}

metadata_offline_command_for_manifest() {
  local manifest="$1"
  local cache_dir="$2"
  if [[ "$TOOL" == "scarb" ]]; then
    reply=(scarb --manifest-path "$manifest" --global-cache-dir "$cache_dir" --offline metadata --format-version 1)
  else
    reply=("$UC_BIN" metadata --daemon-mode require --manifest-path "$manifest" --offline --global-cache-dir "$cache_dir" --format-version 1)
  fi
}

prepare_workload_copy() {
  local workload="$1"
  local source_dir="$2"
  local isolated_dir="$TMP_DIR/workloads/$workload"
  mkdir -p "$isolated_dir"
  cp -PR "$source_dir/." "$isolated_dir"
  rm -rf "$isolated_dir/.uc" "$isolated_dir/target" "$isolated_dir/.scarb"
  printf "%s" "$isolated_dir"
}

ensure_isolated_workload_dir() {
  local path="$1"
  case "$path" in
    "$TMP_DIR"/workloads/*) ;;
    *)
      echo "Refusing destructive benchmark cleanup outside isolated workspace: $path" >&2
      exit 1
      ;;
  esac
}

run_build_cold() {
  local workload="$1"
  local cwd="$2"
  local runs="$3"
  shift 3
  local -a command=("$@")
  local command_string="$(command_to_string "${command[@]}")"
  local samples_file="$TMP_DIR/${TOOL}-${workload//\//_}-build-cold.samples"
  local baseline_dir="$TMP_DIR/cold-baselines/${workload//\//_}"
  : > "$samples_file"

  ensure_isolated_workload_dir "$cwd"
  mkdir -p "$(dirname "$baseline_dir")"
  rm -rf "$baseline_dir"
  cp -PR "$cwd" "$baseline_dir"

  for i in $(seq 1 "$runs"); do
    rm -rf "$cwd"
    cp -PR "$baseline_dir" "$cwd"
    measure_command_ms "$cwd" "${command[@]}" >> "$samples_file"
  done

  append_result "build.cold" "$workload" "$command_string" "$samples_file" "$runs"
}

run_build_warm_noop() {
  local workload="$1"
  local cwd="$2"
  local runs="$3"
  shift 3
  local -a command=("$@")
  local command_string="$(command_to_string "${command[@]}")"
  local samples_file="$TMP_DIR/${TOOL}-${workload//\//_}-build-warm-noop.samples"
  : > "$samples_file"

  measure_command_ms "$cwd" "${command[@]}" > /dev/null
  for i in $(seq 1 "$runs"); do
    measure_command_ms "$cwd" "${command[@]}" >> "$samples_file"
  done

  append_result "build.warm_noop" "$workload" "$command_string" "$samples_file" "$runs"
}

run_build_warm_edit() {
  local workload="$1"
  local cwd="$2"
  local edit_file="$3"
  local runs="$4"
  shift 4
  local -a command=("$@")
  local command_string="$(command_to_string "${command[@]}")"
  local samples_file="$TMP_DIR/${TOOL}-${workload//\//_}-build-warm-edit.samples"
  local backup_file="$TMP_DIR/${TOOL}-${workload//\//_}-edit.backup"
  : > "$samples_file"

  cp "$edit_file" "$backup_file"
  {
    measure_command_ms "$cwd" "${command[@]}" > /dev/null

    for i in $(seq 1 "$runs"); do
      cp "$backup_file" "$edit_file"
      printf "\n// uc benchmark edit %s %s\n" "$i" "$STAMP" >> "$edit_file"
      measure_command_ms "$cwd" "${command[@]}" >> "$samples_file"
    done
  } always {
    cp "$backup_file" "$edit_file" >/dev/null 2>&1 || true
  }

  append_result "build.warm_edit" "$workload" "$command_string" "$samples_file" "$runs"
}

run_metadata_online_cold() {
  local workload="$1"
  local cwd="$2"
  local cache_dir="$3"
  local runs="$4"
  shift 4
  local -a command=("$@")
  local command_string="$(command_to_string "${command[@]}")"
  local samples_file="$TMP_DIR/${TOOL}-${workload//\//_}-metadata-online-cold.samples"
  : > "$samples_file"

  for i in $(seq 1 "$runs"); do
    rm -rf "$cache_dir"
    measure_command_ms "$cwd" "${command[@]}" >> "$samples_file"
  done

  append_result "metadata.online_cold" "$workload" "$command_string" "$samples_file" "$runs"
}

run_metadata_offline_warm() {
  local workload="$1"
  local cwd="$2"
  local cache_dir="$3"
  local runs="$4"
  shift 4
  local -a warm_command=()
  while [[ $# -gt 0 ]]; do
    if [[ "$1" == "--" ]]; then
      shift
      break
    fi
    warm_command+=("$1")
    shift
  done
  local -a command=("$@")
  local command_string="$(command_to_string "${command[@]}")"
  local samples_file="$TMP_DIR/${TOOL}-${workload//\//_}-metadata-offline-warm.samples"
  : > "$samples_file"

  rm -rf "$cache_dir"
  measure_command_ms "$cwd" "${warm_command[@]}" > /dev/null

  for i in $(seq 1 "$runs"); do
    measure_command_ms "$cwd" "${command[@]}" >> "$samples_file"
  done

  append_result "metadata.offline_warm" "$workload" "$command_string" "$samples_file" "$runs"
}

if [[ "$MATRIX" == "research" ]]; then
  if [[ -z "$WORKSPACE_ROOT" ]]; then
    echo "WORKSPACE_ROOT is required for research matrix runs." >&2
    echo "Set WORKSPACE_ROOT to a path where scarb/examples exists." >&2
    exit 1
  fi
  WORKSPACE_ROOT="$(cd "$WORKSPACE_ROOT" && pwd -P)"

  HELLO_SRC="$WORKSPACE_ROOT/scarb/examples/hello_world"
  WS_SRC="$WORKSPACE_ROOT/scarb/examples/workspaces"
  DEPS_SRC="$WORKSPACE_ROOT/scarb/examples/dependencies"

  if [[ ! -d "$HELLO_SRC" || ! -d "$WS_SRC" || ! -d "$DEPS_SRC" ]]; then
    echo "Research matrix directories not found under: $WORKSPACE_ROOT" >&2
    echo "Hint: pass --workspace-root <path> (or WORKSPACE_ROOT=<path>) where scarb/examples exists." >&2
    echo "Expected: scarb/examples/hello_world, scarb/examples/workspaces, scarb/examples/dependencies" >&2
    exit 1
  fi

  HELLO_DIR="$(prepare_workload_copy "hello_world" "$HELLO_SRC")"
  WS_DIR="$(prepare_workload_copy "workspaces" "$WS_SRC")"
  DEPS_DIR="$(prepare_workload_copy "dependencies" "$DEPS_SRC")"

  HELLO_MANIFEST="$HELLO_DIR/Scarb.toml"
  WS_MANIFEST="$WS_DIR/Scarb.toml"
  DEPS_MANIFEST="$DEPS_DIR/Scarb.toml"

  build_command_for_manifest "$HELLO_MANIFEST"
  HELLO_BUILD_CMD=("${reply[@]}")
  build_command_for_manifest "$WS_MANIFEST"
  WS_BUILD_CMD=("${reply[@]}")

  run_build_cold "hello_world" "$HELLO_DIR" "$COLD_RUNS" "${HELLO_BUILD_CMD[@]}"
  run_build_warm_noop "hello_world" "$HELLO_DIR" "$RUNS" "${HELLO_BUILD_CMD[@]}"
  run_build_warm_edit "hello_world" "$HELLO_DIR" "$HELLO_DIR/src/lib.cairo" "$RUNS" "${HELLO_BUILD_CMD[@]}"

  run_build_cold "workspaces" "$WS_DIR" "$COLD_RUNS" "${WS_BUILD_CMD[@]}"
  run_build_warm_noop "workspaces" "$WS_DIR" "$RUNS" "${WS_BUILD_CMD[@]}"
  run_build_warm_edit "workspaces" "$WS_DIR" "$WS_DIR/crates/fibonacci/src/lib.cairo" "$RUNS" "${WS_BUILD_CMD[@]}"

  DEPS_CACHE_DIR="$TMP_DIR/deps-cache"
  metadata_online_command_for_manifest "$DEPS_MANIFEST" "$DEPS_CACHE_DIR"
  DEPS_META_WARM_CMD=("${reply[@]}")
  metadata_offline_command_for_manifest "$DEPS_MANIFEST" "$DEPS_CACHE_DIR"
  DEPS_META_OFFLINE_CMD=("${reply[@]}")

  run_metadata_online_cold "dependencies" "$DEPS_DIR" "$DEPS_CACHE_DIR" "$COLD_RUNS" "${DEPS_META_WARM_CMD[@]}"
  run_metadata_offline_warm "dependencies" "$DEPS_DIR" "$DEPS_CACHE_DIR" "$RUNS" "${DEPS_META_WARM_CMD[@]}" -- "${DEPS_META_OFFLINE_CMD[@]}"
elif [[ "$MATRIX" == "smoke" ]]; then
  SMOKE_SRC="$ROOT_DIR/benchmarks/fixtures/scarb_smoke"
  SMOKE_DIR="$(prepare_workload_copy "scarb_smoke" "$SMOKE_SRC")"
  SMOKE_MANIFEST="$SMOKE_DIR/Scarb.toml"

  build_command_for_manifest "$SMOKE_MANIFEST"
  SMOKE_BUILD_CMD=("${reply[@]}")

  run_build_cold "scarb_smoke" "$SMOKE_DIR" 1 "${SMOKE_BUILD_CMD[@]}"
  run_build_warm_noop "scarb_smoke" "$SMOKE_DIR" 2 "${SMOKE_BUILD_CMD[@]}"
  run_build_warm_edit "scarb_smoke" "$SMOKE_DIR" "$SMOKE_DIR/src/lib.cairo" 2 "${SMOKE_BUILD_CMD[@]}"
else
  echo "Unsupported matrix: $MATRIX" >&2
  exit 1
fi

SCARB_VERSION="$(scarb --version | head -n1)"
if [[ "$TOOL" == "uc" ]]; then
  TOOL_VERSION="uc (local build; scarb backend version: $SCARB_VERSION)"
else
  TOOL_VERSION="$SCARB_VERSION"
fi
GENERATED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
WORKSPACE_ROOT_REPORT="$(workspace_root_for_report)"

jq -s \
  --arg generated_at "$GENERATED_AT" \
  --arg matrix "$MATRIX" \
  --arg host "<redacted>" \
  --arg uname "<redacted>" \
  --arg tool "$TOOL" \
  --arg tool_version "$TOOL_VERSION" \
  --arg scarb_version "$SCARB_VERSION" \
  --arg workspace_root "$WORKSPACE_ROOT_REPORT" \
  --argjson runs "$RUNS" \
  --argjson cold_runs "$COLD_RUNS" \
  '{
    generated_at: $generated_at,
    matrix: $matrix,
    host: $host,
    uname: $uname,
    tool: $tool,
    tool_version: $tool_version,
    scarb_version: $scarb_version,
    workspace_root: $workspace_root,
    runs: $runs,
    cold_runs: $cold_runs,
    scenarios: .
  }' "$TMP_DIR/scenarios.ndjson" > "$OUT_JSON"

{
  echo "# ${TOOL:u} Benchmark ($STAMP)"
  echo
  echo "## Environment"
  echo "- Generated at: $GENERATED_AT"
  echo "- Matrix: $MATRIX"
  echo "- Host: <redacted>"
  echo "- Tool: $TOOL_VERSION"
  echo "- Workspace root: $WORKSPACE_ROOT_REPORT"
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

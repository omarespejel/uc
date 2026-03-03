#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
ROOT_DIR="$(cd "$SCRIPT_DIR/../.." && pwd -P)"
WORKSPACE_ROOT="${WORKSPACE_ROOT:-}"
MATRIX="${MATRIX:-research}"
TOOL="${TOOL:-scarb}"
RUNS="${RUNS:-12}"
COLD_RUNS="${COLD_RUNS:-12}"
BUILD_OFFLINE="${BUILD_OFFLINE:-1}"
UC_DAEMON_MODE="${UC_DAEMON_MODE:-off}"
CPU_SET="${CPU_SET:-${UC_BENCH_CPU_SET:-}}"
NICE_LEVEL="${NICE_LEVEL:-${UC_BENCH_NICE_LEVEL:-0}}"
STRICT_PINNING="${STRICT_PINNING:-${UC_BENCH_STRICT_PINNING:-0}}"
WARM_SETTLE_SECONDS="${WARM_SETTLE_SECONDS:-2.2}"
STAMP="$(date +%Y%m%d-%H%M%S)"
OUT_DIR="$ROOT_DIR/benchmarks/results"
OUT_JSON=""
OUT_MD=""
TMP_DIR="$(mktemp -d)"
UC_BIN="$ROOT_DIR/target/debug/uc"
UC_DAEMON_SOCKET_PATH="${UC_DAEMON_SOCKET_PATH:-$TMP_DIR/uc-daemon.sock}"
UC_DAEMON_STARTED=0
TASKSET_ENABLED=0
CPU_GOVERNOR="unknown"

declare -a EXEC_PREFIX=()
declare -a CMD_REPLY=()
LAST_MEASURE_STDERR_FILE=""
LAST_MEASURE_ELAPSED_MS=""

export UC_DAEMON_SOCKET_PATH
# Keep language/runtime behavior stable across cycles.
export LC_ALL=C
export TZ=UTC
export CARGO_INCREMENTAL=0
export RUST_BACKTRACE=0

cleanup() {
  if [[ "$TOOL" == "uc" && "$UC_DAEMON_STARTED" == "1" && -x "$UC_BIN" ]]; then
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
  --runs <n>                  Runs for warm/offline scenarios (default: 12)
  --cold-runs <n>             Runs for cold scenarios (default: 12)
  --build-online              Measure build scenarios in online mode (default: offline)
  --uc-daemon-mode <mode>     UC daemon mode for uc tool (off|require, default: off)
  --cpu-set <list>            Optional CPU affinity list (e.g. 0 or 0-1)
  --nice-level <n>            Optional process nice level (default: 0)
  --warm-settle-seconds <n>   Wait after warm-up before warm-noop samples (default: 2.2)
  --strict-pinning            Fail if requested pinning cannot be applied
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
    --warm-settle-seconds)
      require_option_value "$1" "${2-}"
      WARM_SETTLE_SECONDS="$2"
      shift 2
      ;;
    --strict-pinning)
      STRICT_PINNING=1
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

if [[ ! "$RUNS" =~ ^[0-9]+$ || "$RUNS" -le 0 ]]; then
  echo "--runs must be a positive integer, got: $RUNS" >&2
  exit 1
fi
if [[ ! "$COLD_RUNS" =~ ^[0-9]+$ || "$COLD_RUNS" -le 0 ]]; then
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
if ! [[ "$WARM_SETTLE_SECONDS" =~ ^[0-9]+([.][0-9]+)?$ ]]; then
  echo "--warm-settle-seconds must be a positive number, got: $WARM_SETTLE_SECONDS" >&2
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
require_cmd python3
SCARB_VERSION="$(scarb --version | head -n1)"
export UC_SCARB_VERSION_LINE="$SCARB_VERSION"

if [[ "$TOOL" != "scarb" && "$TOOL" != "uc" ]]; then
  echo "Unsupported tool: $TOOL" >&2
  exit 1
fi

if [[ "$MATRIX" == "research" && -z "$WORKSPACE_ROOT" ]]; then
  echo "WORKSPACE_ROOT is required for research matrix runs." >&2
  echo "Set WORKSPACE_ROOT to a path where scarb/examples exists." >&2
  exit 1
fi

configure_execution_prefix() {
  EXEC_PREFIX=()

  if [[ -n "$CPU_SET" ]]; then
    if command -v taskset >/dev/null 2>&1; then
      EXEC_PREFIX+=(taskset -c "$CPU_SET")
      TASKSET_ENABLED=1
    elif [[ "$STRICT_PINNING" == "1" ]]; then
      echo "Requested --cpu-set but taskset is unavailable." >&2
      exit 1
    else
      echo "Benchmark warning: taskset unavailable; CPU pinning skipped." >&2
    fi
  fi

  if [[ "$NICE_LEVEL" != "0" ]]; then
    if command -v nice >/dev/null 2>&1; then
      EXEC_PREFIX+=(nice -n "$NICE_LEVEL")
    elif [[ "$STRICT_PINNING" == "1" ]]; then
      echo "Requested --nice-level but nice is unavailable." >&2
      exit 1
    else
      echo "Benchmark warning: nice unavailable; priority pinning skipped." >&2
    fi
  fi
}

capture_cpu_governor() {
  if [[ -r /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor ]]; then
    CPU_GOVERNOR="$(cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor)"
    if [[ "$CPU_GOVERNOR" != "performance" ]]; then
      if [[ "$STRICT_PINNING" == "1" ]]; then
        echo "CPU governor is '$CPU_GOVERNOR'; expected 'performance' in strict mode." >&2
        exit 1
      fi
      echo "Benchmark warning: CPU governor is '$CPU_GOVERNOR' (not 'performance')." >&2
    fi
  fi
}

with_exec_prefix() {
  local -a base=("$@")
  CMD_REPLY=("${EXEC_PREFIX[@]}" "${base[@]}")
}

if [[ "$TOOL" == "uc" ]]; then
  require_cmd cargo
  (cd "$ROOT_DIR" && cargo build -p uc-cli >/dev/null)
  export UC_PHASE_TIMING=1
  if [[ "$UC_DAEMON_MODE" != "off" ]]; then
    UC_DAEMON_SOCKET_PATH="$UC_DAEMON_SOCKET_PATH" "$UC_BIN" daemon start >/dev/null
    UC_DAEMON_STARTED=1
  fi
fi

configure_execution_prefix
capture_cpu_governor

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
  local -a escaped=()
  local arg quoted
  for arg in "$@"; do
    printf -v quoted '%q' "$arg"
    escaped+=("$quoted")
  done
  local joined="${escaped[*]}"
  printf "%s" "$joined"
}

measure_command_ms() {
  local cwd="$1"
  shift
  local -a argv=("$@")
  local stderr_file="$TMP_DIR/stderr-$$-$RANDOM.log"
  local display
  local elapsed_ms

  if [[ ${#argv[@]} -eq 0 ]]; then
    echo "Command parse failed: empty argv" >&2
    return 1
  fi

  display="$(command_to_string "${argv[@]}")"

  if ! elapsed_ms="$(python3 - "$cwd" "$stderr_file" "${argv[@]}" <<'PY'
import os
import subprocess
import sys
import time

cwd = sys.argv[1]
stderr_path = sys.argv[2]
command = sys.argv[3:]

start_ns = time.monotonic_ns()
with open(os.devnull, "wb") as devnull, open(stderr_path, "wb") as stderr:
    proc = subprocess.run(command, cwd=cwd, stdout=devnull, stderr=stderr)
elapsed_ms = (time.monotonic_ns() - start_ns) / 1_000_000
if proc.returncode != 0:
    raise SystemExit(proc.returncode)
print(f"{elapsed_ms:.3f}")
PY
  )"; then
    echo "Command failed in $cwd: $display" >&2
    cat "$stderr_file" >&2
    return 1
  fi
  LAST_MEASURE_STDERR_FILE="$stderr_file"
  LAST_MEASURE_ELAPSED_MS="$elapsed_ms"
  printf "%s\n" "$elapsed_ms"
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

record_uc_phase_sample() {
  local phase_file="$1"
  if [[ "$TOOL" != "uc" ]]; then
    return 0
  fi
  if [[ -z "$LAST_MEASURE_STDERR_FILE" || ! -f "$LAST_MEASURE_STDERR_FILE" ]]; then
    return 0
  fi
  local phase_line
  phase_line="$(grep -F "uc: phase timings (ms)" "$LAST_MEASURE_STDERR_FILE" | tail -n1 || true)"
  if [[ -z "$phase_line" ]]; then
    return 0
  fi
  python3 - "$phase_file" "$LAST_MEASURE_ELAPSED_MS" "$phase_line" <<'PY'
import json
import re
import sys

phase_file = sys.argv[1]
elapsed_ms = float(sys.argv[2])
line = sys.argv[3]
pattern = re.compile(
    r"fingerprint=(?P<fingerprint>[0-9.]+)\s+"
    r"cache_lookup=(?P<cache_lookup>[0-9.]+)\s+"
    r"cache_restore=(?P<cache_restore>[0-9.]+)\s+"
    r"compile=(?P<compile>[0-9.]+)\s+"
    r"cache_persist=(?P<cache_persist>[0-9.]+)\s+"
    r"async=(?P<async>\w+)\s+"
    r"scheduled=(?P<scheduled>\w+)\s+"
    r"daemon_used=(?P<daemon_used>\w+)\s+"
    r"cache_hit=(?P<cache_hit>\w+)"
)
match = pattern.search(line)
if not match:
    raise SystemExit(0)

def as_bool(value: str) -> bool:
    return value.strip().lower() in {"1", "true", "yes", "on"}

payload = {
    "elapsed_ms": elapsed_ms,
    "fingerprint_ms": float(match.group("fingerprint")),
    "cache_lookup_ms": float(match.group("cache_lookup")),
    "cache_restore_ms": float(match.group("cache_restore")),
    "compile_ms": float(match.group("compile")),
    "cache_persist_ms": float(match.group("cache_persist")),
    "async_persist": as_bool(match.group("async")),
    "persist_scheduled": as_bool(match.group("scheduled")),
    "daemon_used": as_bool(match.group("daemon_used")),
    "cache_hit": as_bool(match.group("cache_hit")),
}
with open(phase_file, "a", encoding="utf-8") as f:
    f.write(json.dumps(payload))
    f.write("\n")
PY
}

phase_stats_json_from_samples() {
  local phase_file="$1"
  if [[ ! -s "$phase_file" ]]; then
    printf "null"
    return 0
  fi
  jq -s '
    def quantile($arr; $p):
      ($arr | sort) as $s
      | ($s | length) as $n
      | if $n == 0 then null
        else
          ($n - 1) as $k
          | ($k * $p) as $i
          | ($i | floor) as $lo
          | ($i | ceil) as $hi
          | if $lo == $hi then $s[$lo]
            else ($s[$lo] + (($s[$hi] - $s[$lo]) * ($i - $lo)))
            end
        end;
    def metric($key):
      [.[].[$key]] as $values
      | if ($values | length) == 0 then null
        else {
          min_ms: ($values | min),
          max_ms: ($values | max),
          mean_ms: (($values | add) / ($values | length)),
          p50_ms: quantile($values; 0.50),
          p95_ms: quantile($values; 0.95)
        }
        end;
    if length == 0 then null else {
      sample_count: length,
      cache_hit_count: ([.[] | select(.cache_hit)] | length),
      cache_miss_count: ([.[] | select(.cache_hit | not)] | length),
      daemon_used_count: ([.[] | select(.daemon_used)] | length),
      async_persist_count: ([.[] | select(.async_persist)] | length),
      persist_scheduled_count: ([.[] | select(.persist_scheduled)] | length),
      elapsed_ms: metric("elapsed_ms"),
      fingerprint_ms: metric("fingerprint_ms"),
      cache_lookup_ms: metric("cache_lookup_ms"),
      cache_restore_ms: metric("cache_restore_ms"),
      compile_ms: metric("compile_ms"),
      cache_persist_ms: metric("cache_persist_ms")
    } end
  ' "$phase_file"
}

append_result() {
  local scenario="$1"
  local workload="$2"
  local command="$3"
  local samples_file="$4"
  local runs="$5"
  local phase_file="${6:-}"
  local stats_json
  local samples_json
  local phase_samples_json="null"
  local phase_stats_json="null"
  local report_command

  stats_json="$(stats_json_from_samples "$samples_file")"
  samples_json="$(jq -s '.' "$samples_file")"
  if [[ -n "$phase_file" && -s "$phase_file" ]]; then
    phase_samples_json="$(jq -s '.' "$phase_file")"
    phase_stats_json="$(phase_stats_json_from_samples "$phase_file")"
  fi
  report_command="$(sanitize_for_report "$command")"

  jq -n \
    --arg scenario "$scenario" \
    --arg workload "$workload" \
    --arg command "$report_command" \
    --argjson runs "$runs" \
    --argjson samples_ms "$samples_json" \
    --argjson stats "$stats_json" \
    --argjson phase_samples "$phase_samples_json" \
    --argjson phase_stats "$phase_stats_json" \
    '{
      scenario: $scenario,
      workload: $workload,
      command: $command,
      runs: $runs,
      samples_ms: $samples_ms,
      stats: $stats,
      phase_samples: $phase_samples,
      phase_stats: $phase_stats
    }' >> "$TMP_DIR/scenarios.ndjson"
}

build_command_for_manifest_with_mode() {
  local manifest="$1"
  local offline_mode="$2"
  local -a base=()
  if [[ "$TOOL" == "scarb" ]]; then
    base=(scarb --manifest-path "$manifest")
    if [[ "$offline_mode" == "1" ]]; then
      base+=(--offline)
    fi
    base+=(build)
  else
    base=("$UC_BIN" build --engine uc --daemon-mode "$UC_DAEMON_MODE" --manifest-path "$manifest")
    if [[ "$offline_mode" == "1" ]]; then
      base+=(--offline)
    fi
  fi
  with_exec_prefix "${base[@]}"
}

build_command_for_manifest() {
  local manifest="$1"
  build_command_for_manifest_with_mode "$manifest" "$BUILD_OFFLINE"
}

metadata_online_command_for_manifest() {
  local manifest="$1"
  local cache_dir="$2"
  local -a base=()
  if [[ "$TOOL" == "scarb" ]]; then
    base=(scarb --manifest-path "$manifest" --global-cache-dir "$cache_dir" metadata --format-version 1)
  else
    base=("$UC_BIN" metadata --daemon-mode "$UC_DAEMON_MODE" --manifest-path "$manifest" --global-cache-dir "$cache_dir" --format-version 1)
  fi
  with_exec_prefix "${base[@]}"
}

metadata_offline_command_for_manifest() {
  local manifest="$1"
  local cache_dir="$2"
  local -a base=()
  if [[ "$TOOL" == "scarb" ]]; then
    base=(scarb --manifest-path "$manifest" --global-cache-dir "$cache_dir" --offline metadata --format-version 1)
  else
    base=("$UC_BIN" metadata --daemon-mode "$UC_DAEMON_MODE" --manifest-path "$manifest" --offline --global-cache-dir "$cache_dir" --format-version 1)
  fi
  with_exec_prefix "${base[@]}"
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

reset_workload_outputs() {
  local cwd="$1"
  ensure_isolated_workload_dir "$cwd"
  rm -rf "$cwd/target" "$cwd/.uc" "$cwd/.scarb"
}

prime_build_dependencies_if_needed() {
  local workload="$1"
  local manifest="$2"
  if [[ "$BUILD_OFFLINE" != "1" ]]; then
    return 0
  fi
  local cwd
  cwd="$(dirname "$manifest")"
  build_command_for_manifest_with_mode "$manifest" "0"
  local -a warm_online_command=("${CMD_REPLY[@]}")
  echo "Priming online build cache for $workload before offline measurements..." >&2
  measure_command_ms "$cwd" "${warm_online_command[@]}" >/dev/null
  reset_workload_outputs "$cwd"
}

run_build_cold() {
  local workload="$1"
  local cwd="$2"
  local runs="$3"
  shift 3
  local -a command=("$@")
  local command_string="$(command_to_string "${command[@]}")"
  local samples_file="$TMP_DIR/${TOOL}-${workload//\//_}-build-cold.samples"
  local phase_file="$TMP_DIR/${TOOL}-${workload//\//_}-build-cold.phases.ndjson"
  local baseline_dir="$TMP_DIR/cold-baselines/${workload//\//_}"
  : > "$samples_file"
  : > "$phase_file"

  ensure_isolated_workload_dir "$cwd"
  mkdir -p "$(dirname "$baseline_dir")"
  rm -rf "$baseline_dir"
  cp -PR "$cwd" "$baseline_dir"

  for _ in $(seq 1 "$runs"); do
    rm -rf "$cwd"
    cp -PR "$baseline_dir" "$cwd"
    measure_command_ms "$cwd" "${command[@]}" >> "$samples_file"
    record_uc_phase_sample "$phase_file"
  done

  append_result "build.cold" "$workload" "$command_string" "$samples_file" "$runs" "$phase_file"
}

run_build_warm_noop() {
  local workload="$1"
  local cwd="$2"
  local runs="$3"
  shift 3
  local -a command=("$@")
  local command_string="$(command_to_string "${command[@]}")"
  local samples_file="$TMP_DIR/${TOOL}-${workload//\//_}-build-warm-noop.samples"
  local phase_file="$TMP_DIR/${TOOL}-${workload//\//_}-build-warm-noop.phases.ndjson"
  : > "$samples_file"
  : > "$phase_file"

  measure_command_ms "$cwd" "${command[@]}" >/dev/null
  sleep "$WARM_SETTLE_SECONDS"
  for _ in $(seq 1 "$runs"); do
    measure_command_ms "$cwd" "${command[@]}" >> "$samples_file"
    record_uc_phase_sample "$phase_file"
  done

  append_result "build.warm_noop" "$workload" "$command_string" "$samples_file" "$runs" "$phase_file"
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
  local phase_file="$TMP_DIR/${TOOL}-${workload//\//_}-build-warm-edit.phases.ndjson"
  local backup_file="$TMP_DIR/${TOOL}-${workload//\//_}-edit.backup"
  local failed=0
  : > "$samples_file"
  : > "$phase_file"

  cp "$edit_file" "$backup_file"
  if ! measure_command_ms "$cwd" "${command[@]}" >/dev/null; then
    failed=1
  fi

  if [[ "$failed" -eq 0 ]]; then
    for i in $(seq 1 "$runs"); do
      cp "$backup_file" "$edit_file"
      printf "\n// uc benchmark edit %s %s\n" "$i" "$STAMP" >> "$edit_file"
      if ! measure_command_ms "$cwd" "${command[@]}" >> "$samples_file"; then
        failed=1
        break
      fi
      record_uc_phase_sample "$phase_file"
    done
  fi

  cp "$backup_file" "$edit_file" >/dev/null 2>&1 || true
  if [[ "$failed" -ne 0 ]]; then
    return 1
  fi

  append_result "build.warm_edit" "$workload" "$command_string" "$samples_file" "$runs" "$phase_file"
}

rewrite_smoke_semantic_edit() {
  local file="$1"
  local value="$2"
  local tmp_file="$TMP_DIR/semantic-edit-$$.tmp"
  sed -E "s/(const BENCH_EDIT_SEED_BIAS: felt252 = )[0-9]+;/\\1${value};/" "$file" > "$tmp_file"
  if cmp -s "$file" "$tmp_file"; then
    echo "Failed to apply semantic benchmark edit marker in $file" >&2
    rm -f "$tmp_file"
    exit 1
  fi
  mv "$tmp_file" "$file"
}

run_build_warm_edit_semantic() {
  local workload="$1"
  local cwd="$2"
  local edit_file="$3"
  local runs="$4"
  shift 4
  local -a command=("$@")
  local command_string="$(command_to_string "${command[@]}")"
  local samples_file="$TMP_DIR/${TOOL}-${workload//\//_}-build-warm-edit-semantic.samples"
  local phase_file="$TMP_DIR/${TOOL}-${workload//\//_}-build-warm-edit-semantic.phases.ndjson"
  local backup_file="$TMP_DIR/${TOOL}-${workload//\//_}-semantic-edit.backup"
  local failed=0
  : > "$samples_file"
  : > "$phase_file"

  cp "$edit_file" "$backup_file"
  if ! measure_command_ms "$cwd" "${command[@]}" >/dev/null; then
    failed=1
  fi

  if [[ "$failed" -eq 0 ]]; then
    for i in $(seq 1 "$runs"); do
      cp "$backup_file" "$edit_file"
      rewrite_smoke_semantic_edit "$edit_file" "$i"
      if ! measure_command_ms "$cwd" "${command[@]}" >> "$samples_file"; then
        failed=1
        break
      fi
      record_uc_phase_sample "$phase_file"
    done
  fi

  cp "$backup_file" "$edit_file" >/dev/null 2>&1 || true
  if [[ "$failed" -ne 0 ]]; then
    return 1
  fi

  append_result "build.warm_edit_semantic" "$workload" "$command_string" "$samples_file" "$runs" "$phase_file"
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
  local phase_file="$TMP_DIR/${TOOL}-${workload//\//_}-metadata-online-cold.phases.ndjson"
  : > "$samples_file"
  : > "$phase_file"

  for _ in $(seq 1 "$runs"); do
    rm -rf "$cache_dir"
    measure_command_ms "$cwd" "${command[@]}" >> "$samples_file"
    record_uc_phase_sample "$phase_file"
  done

  append_result "metadata.online_cold" "$workload" "$command_string" "$samples_file" "$runs" "$phase_file"
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
  local phase_file="$TMP_DIR/${TOOL}-${workload//\//_}-metadata-offline-warm.phases.ndjson"
  : > "$samples_file"
  : > "$phase_file"

  rm -rf "$cache_dir"
  measure_command_ms "$cwd" "${warm_command[@]}" >/dev/null

  for _ in $(seq 1 "$runs"); do
    measure_command_ms "$cwd" "${command[@]}" >> "$samples_file"
    record_uc_phase_sample "$phase_file"
  done

  append_result "metadata.offline_warm" "$workload" "$command_string" "$samples_file" "$runs" "$phase_file"
}

if [[ "$MATRIX" == "research" ]]; then
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
  HELLO_BUILD_CMD=("${CMD_REPLY[@]}")
  build_command_for_manifest "$WS_MANIFEST"
  WS_BUILD_CMD=("${CMD_REPLY[@]}")

  prime_build_dependencies_if_needed "hello_world" "$HELLO_MANIFEST"
  prime_build_dependencies_if_needed "workspaces" "$WS_MANIFEST"

  run_build_cold "hello_world" "$HELLO_DIR" "$COLD_RUNS" "${HELLO_BUILD_CMD[@]}"
  run_build_warm_noop "hello_world" "$HELLO_DIR" "$RUNS" "${HELLO_BUILD_CMD[@]}"
  run_build_warm_edit "hello_world" "$HELLO_DIR" "$HELLO_DIR/src/lib.cairo" "$RUNS" "${HELLO_BUILD_CMD[@]}"

  run_build_cold "workspaces" "$WS_DIR" "$COLD_RUNS" "${WS_BUILD_CMD[@]}"
  run_build_warm_noop "workspaces" "$WS_DIR" "$RUNS" "${WS_BUILD_CMD[@]}"
  run_build_warm_edit "workspaces" "$WS_DIR" "$WS_DIR/crates/fibonacci/src/lib.cairo" "$RUNS" "${WS_BUILD_CMD[@]}"

  DEPS_CACHE_DIR="$TMP_DIR/deps-cache"
  metadata_online_command_for_manifest "$DEPS_MANIFEST" "$DEPS_CACHE_DIR"
  DEPS_META_WARM_CMD=("${CMD_REPLY[@]}")
  metadata_offline_command_for_manifest "$DEPS_MANIFEST" "$DEPS_CACHE_DIR"
  DEPS_META_OFFLINE_CMD=("${CMD_REPLY[@]}")

  run_metadata_online_cold "dependencies" "$DEPS_DIR" "$DEPS_CACHE_DIR" "$COLD_RUNS" "${DEPS_META_WARM_CMD[@]}"
  run_metadata_offline_warm "dependencies" "$DEPS_DIR" "$DEPS_CACHE_DIR" "$RUNS" "${DEPS_META_WARM_CMD[@]}" -- "${DEPS_META_OFFLINE_CMD[@]}"
elif [[ "$MATRIX" == "smoke" ]]; then
  SMOKE_SRC="$ROOT_DIR/benchmarks/fixtures/scarb_smoke"
  SMOKE_DIR="$(prepare_workload_copy "scarb_smoke" "$SMOKE_SRC")"
  SMOKE_MANIFEST="$SMOKE_DIR/Scarb.toml"

  build_command_for_manifest "$SMOKE_MANIFEST"
  SMOKE_BUILD_CMD=("${CMD_REPLY[@]}")

  prime_build_dependencies_if_needed "scarb_smoke" "$SMOKE_MANIFEST"

  run_build_cold "scarb_smoke" "$SMOKE_DIR" "$COLD_RUNS" "${SMOKE_BUILD_CMD[@]}"
  run_build_warm_noop "scarb_smoke" "$SMOKE_DIR" "$RUNS" "${SMOKE_BUILD_CMD[@]}"
  run_build_warm_edit "scarb_smoke" "$SMOKE_DIR" "$SMOKE_DIR/src/lib.cairo" "$RUNS" "${SMOKE_BUILD_CMD[@]}"
  run_build_warm_edit_semantic "scarb_smoke" "$SMOKE_DIR" "$SMOKE_DIR/src/lib.cairo" "$RUNS" "${SMOKE_BUILD_CMD[@]}"
else
  echo "Unsupported matrix: $MATRIX" >&2
  exit 1
fi

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
  --arg cpu_set "$CPU_SET" \
  --arg nice_level "$NICE_LEVEL" \
  --arg build_offline "$BUILD_OFFLINE" \
  --arg uc_daemon_mode "$UC_DAEMON_MODE" \
  --arg strict_pinning "$STRICT_PINNING" \
  --arg taskset_enabled "$TASKSET_ENABLED" \
  --arg cpu_governor "$CPU_GOVERNOR" \
  --arg warm_settle_seconds "$WARM_SETTLE_SECONDS" \
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
    pinned_conditions: {
      cpu_set: (if $cpu_set == "" then null else $cpu_set end),
      nice_level: ($nice_level | tonumber),
      build_offline: ($build_offline == "1"),
      uc_daemon_mode: $uc_daemon_mode,
      strict_pinning: ($strict_pinning == "1"),
      taskset_enabled: ($taskset_enabled == "1"),
      cpu_governor: $cpu_governor,
      warm_settle_seconds: ($warm_settle_seconds | tonumber),
      env: {
        lc_all: "C",
        tz: "UTC",
        cargo_incremental: "0"
      }
    },
    scenarios: .
  }' "$TMP_DIR/scenarios.ndjson" > "$OUT_JSON"

{
  echo "# ${TOOL^^} Benchmark ($STAMP)"
  echo
  echo "## Environment"
  echo "- Generated at: $GENERATED_AT"
  echo "- Matrix: $MATRIX"
  echo "- Host: <redacted>"
  echo "- Tool: $TOOL_VERSION"
  echo "- Workspace root: $WORKSPACE_ROOT_REPORT"
  echo "- CPU set: ${CPU_SET:-<none>}"
  echo "- Nice level: $NICE_LEVEL"
  echo "- Build mode: $(if [[ "$BUILD_OFFLINE" == "1" ]]; then echo "offline"; else echo "online"; fi)"
  echo "- UC daemon mode: $UC_DAEMON_MODE"
  echo "- CPU governor: $CPU_GOVERNOR"
  echo "- Warm settle seconds: $WARM_SETTLE_SECONDS"
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

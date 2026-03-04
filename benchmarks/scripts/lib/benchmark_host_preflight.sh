#!/usr/bin/env bash

# Collect process lines that are known to introduce benchmark jitter.
# Output format: "<pid>\t<command>" (one match per line).
uc_bench_collect_noisy_processes_from_snapshot() {
  local snapshot_file="$1"
  if [[ -z "$snapshot_file" || ! -f "$snapshot_file" ]]; then
    return 0
  fi

  awk '
    {
      pid = $1
      $1 = ""
      sub(/^[[:space:]]+/, "", $0)
      cmd = $0
      lower = tolower(cmd)
      noisy = 0

      if (lower ~ /(^|[[:space:][:punct:]])scarb-cairo-language-server([[:space:]]|$)/) {
        noisy = 1
      } else if (lower ~ /(^|[[:space:][:punct:]])cairo-language-server([[:space:]]|$)/) {
        noisy = 1
      } else if (lower ~ /scarb/ && lower ~ /proc-macro-server/) {
        noisy = 1
      }

      if (noisy == 1 && pid ~ /^[0-9]+$/) {
        print pid "\t" cmd
      }
    }
  ' "$snapshot_file"
}

uc_bench_capture_process_snapshot() {
  local snapshot_file="$1"
  if [[ -z "$snapshot_file" ]]; then
    return 1
  fi

  if [[ -n "${UC_BENCH_PS_SNAPSHOT_FILE:-}" ]]; then
    if [[ ! -f "$UC_BENCH_PS_SNAPSHOT_FILE" ]]; then
      echo "UC_BENCH_PS_SNAPSHOT_FILE does not exist: $UC_BENCH_PS_SNAPSHOT_FILE" >&2
      return 1
    fi
    cp "$UC_BENCH_PS_SNAPSHOT_FILE" "$snapshot_file"
    return 0
  fi

  if ! command -v ps >/dev/null 2>&1; then
    echo "Unable to run benchmark host preflight: missing 'ps' command." >&2
    return 1
  fi

  ps -axo pid=,command= > "$snapshot_file"
}

# Mode:
#   off     -> always succeeds
#   warn    -> prints warning when noisy processes are present, succeeds
#   require -> prints error when noisy processes are present, returns non-zero
uc_bench_preflight_host_noise() {
  local mode="${1:-warn}"
  local provided_snapshot="${2:-}"
  local snapshot_file=""
  local noisy_lines=""
  local cleanup_snapshot=0

  if [[ "$mode" == "off" ]]; then
    return 0
  fi
  if [[ "$mode" != "warn" && "$mode" != "require" ]]; then
    echo "Invalid host preflight mode: $mode (expected: off, warn, require)." >&2
    return 2
  fi

  if [[ -n "$provided_snapshot" ]]; then
    snapshot_file="$provided_snapshot"
  else
    snapshot_file="$(mktemp)"
    cleanup_snapshot=1
    if ! uc_bench_capture_process_snapshot "$snapshot_file"; then
      if [[ "$cleanup_snapshot" == "1" ]]; then
        rm -f "$snapshot_file" >/dev/null 2>&1 || true
      fi
      if [[ "$mode" == "require" ]]; then
        return 1
      fi
      echo "Benchmark warning: host preflight could not capture process snapshot." >&2
      return 0
    fi
  fi

  noisy_lines="$(uc_bench_collect_noisy_processes_from_snapshot "$snapshot_file")"
  if [[ "$cleanup_snapshot" == "1" ]]; then
    rm -f "$snapshot_file" >/dev/null 2>&1 || true
  fi
  if [[ -z "$noisy_lines" ]]; then
    return 0
  fi

  if [[ "$mode" == "require" ]]; then
    echo "Detected background processes known to skew benchmark variance:" >&2
    echo "$noisy_lines" | sed 's/^/  - /' >&2
    echo "Stop these processes (or use --allow-noisy-host to bypass) and rerun benchmarks." >&2
    return 1
  fi

  echo "Benchmark warning: detected background processes known to skew benchmark variance:" >&2
  echo "$noisy_lines" | sed 's/^/  - /' >&2
  echo "Results may be noisy; rerun on a clean host for stable numbers." >&2
  return 0
}

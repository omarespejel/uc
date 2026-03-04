#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
LIB_PATH="$SCRIPT_DIR/../lib/benchmark_host_preflight.sh"
LOCAL_BENCH_SCRIPT="$SCRIPT_DIR/../run_local_benchmarks.sh"
STABILITY_BENCH_SCRIPT="$SCRIPT_DIR/../run_stability_benchmarks.sh"

if [[ ! -f "$LIB_PATH" ]]; then
  echo "missing preflight library at $LIB_PATH" >&2
  exit 1
fi

# shellcheck source=/dev/null
source "$LIB_PATH"

TEST_TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TEST_TMP_DIR"' EXIT

assert_contains() {
  local haystack="$1"
  local needle="$2"
  if [[ "$haystack" != *"$needle"* ]]; then
    echo "assert_contains failed: expected to find '$needle'" >&2
    echo "actual: $haystack" >&2
    return 1
  fi
}

assert_empty() {
  local value="$1"
  if [[ -n "$value" ]]; then
    echo "assert_empty failed: expected empty value" >&2
    echo "actual: $value" >&2
    return 1
  fi
}

assert_not_contains() {
  local haystack="$1"
  local needle="$2"
  if [[ "$haystack" == *"$needle"* ]]; then
    echo "assert_not_contains failed: did not expect '$needle'" >&2
    echo "actual: $haystack" >&2
    return 1
  fi
}

run_test() {
  local name="$1"
  shift
  echo "[test] $name"
  "$@"
}

write_snapshot() {
  local path="$1"
  shift
  cat > "$path" <<SNAP
$*
SNAP
}

has_local_runner_prerequisites() {
  command -v scarb >/dev/null 2>&1 &&
    command -v jq >/dev/null 2>&1 &&
    command -v awk >/dev/null 2>&1 &&
    command -v sort >/dev/null 2>&1 &&
    command -v python3 >/dev/null 2>&1
}

test_detects_proc_macro_server_noise() {
  local snapshot="$TEST_TMP_DIR/proc-macro.ps"
  write_snapshot "$snapshot" \
" 101 /usr/bin/zsh" \
" 4242 /usr/local/bin/scarb --quiet proc-macro-server --manifest-path /tmp/ws/Scarb.toml"

  local result
  result="$(uc_bench_collect_noisy_processes_from_snapshot "$snapshot")"
  assert_contains "$result" "4242"
  assert_contains "$result" "proc-macro-server"
}

test_detects_language_server_noise() {
  local snapshot="$TEST_TMP_DIR/language-server.ps"
  write_snapshot "$snapshot" \
" 313 /usr/bin/cairo-language-server" \
" 314 /usr/bin/anything-else"

  local result
  result="$(uc_bench_collect_noisy_processes_from_snapshot "$snapshot")"
  assert_contains "$result" "313"
  assert_contains "$result" "cairo-language-server"
}

test_clean_snapshot_has_no_noise() {
  local snapshot="$TEST_TMP_DIR/clean.ps"
  write_snapshot "$snapshot" \
" 11 /usr/bin/zsh" \
" 12 /usr/bin/python3 /tmp/worker.py"

  local result
  result="$(uc_bench_collect_noisy_processes_from_snapshot "$snapshot")"
  assert_empty "$result"
}

test_preflight_require_fails_with_noise() {
  local snapshot="$TEST_TMP_DIR/require-noise.ps"
  write_snapshot "$snapshot" \
" 500 /usr/local/bin/scarb-cairo-language-server"

  local stderr_file="$TEST_TMP_DIR/require.err"
  if uc_bench_preflight_host_noise "require" "$snapshot" >"$TEST_TMP_DIR/require.out" 2>"$stderr_file"; then
    echo "expected preflight require mode to fail on noisy host" >&2
    return 1
  fi

  local stderr_text
  stderr_text="$(cat "$stderr_file")"
  assert_contains "$stderr_text" "Detected background processes"
  assert_contains "$stderr_text" "scarb-cairo-language-server"
}

test_preflight_warn_passes_with_noise() {
  local snapshot="$TEST_TMP_DIR/warn-noise.ps"
  write_snapshot "$snapshot" \
" 600 /usr/local/bin/scarb --quiet proc-macro-server"

  local stderr_file="$TEST_TMP_DIR/warn.err"
  uc_bench_preflight_host_noise "warn" "$snapshot" >"$TEST_TMP_DIR/warn.out" 2>"$stderr_file"

  local stderr_text
  stderr_text="$(cat "$stderr_file")"
  assert_contains "$stderr_text" "Benchmark warning"
}

test_preflight_off_skips_noise_check() {
  local snapshot="$TEST_TMP_DIR/off-noise.ps"
  write_snapshot "$snapshot" \
" 700 /usr/local/bin/scarb --quiet proc-macro-server"

  local stderr_file="$TEST_TMP_DIR/off.err"
  uc_bench_preflight_host_noise "off" "$snapshot" >"$TEST_TMP_DIR/off.out" 2>"$stderr_file"
  assert_empty "$(cat "$stderr_file")"
}

test_run_local_require_mode_fails_before_execution_on_noise() {
  if ! has_local_runner_prerequisites; then
    echo "[skip] local runner prerequisites unavailable (scarb/jq/awk/sort/python3)"
    return 0
  fi

  local snapshot="$TEST_TMP_DIR/local-require-noise.ps"
  write_snapshot "$snapshot" \
" 808 /usr/local/bin/scarb --quiet proc-macro-server"

  local stdout_file="$TEST_TMP_DIR/local-require.out"
  local stderr_file="$TEST_TMP_DIR/local-require.err"
  if UC_BENCH_PS_SNAPSHOT_FILE="$snapshot" \
    "$LOCAL_BENCH_SCRIPT" \
      --matrix smoke \
      --tool scarb \
      --runs 1 \
      --cold-runs 1 \
      --host-preflight require \
      >"$stdout_file" 2>"$stderr_file"; then
    echo "expected run_local_benchmarks.sh to fail in require mode on noisy host" >&2
    return 1
  fi

  local stderr_text
  stderr_text="$(cat "$stderr_file")"
  assert_contains "$stderr_text" "Detected background processes"
  assert_contains "$stderr_text" "proc-macro-server"
}

test_run_local_allow_noisy_host_bypasses_preflight_failure() {
  if ! has_local_runner_prerequisites; then
    echo "[skip] local runner prerequisites unavailable (scarb/jq/awk/sort/python3)"
    return 0
  fi

  local snapshot="$TEST_TMP_DIR/local-allow-noise.ps"
  write_snapshot "$snapshot" \
" 909 /usr/local/bin/scarb-cairo-language-server"

  local stdout_file="$TEST_TMP_DIR/local-allow.out"
  local stderr_file="$TEST_TMP_DIR/local-allow.err"
  if UC_BENCH_PS_SNAPSHOT_FILE="$snapshot" \
    "$LOCAL_BENCH_SCRIPT" \
      --matrix invalid \
      --tool scarb \
      --runs 1 \
      --cold-runs 1 \
      --host-preflight require \
      --allow-noisy-host \
      >"$stdout_file" 2>"$stderr_file"; then
    echo "expected run_local_benchmarks.sh to fail with unsupported matrix" >&2
    return 1
  fi

  local stderr_text
  stderr_text="$(cat "$stderr_file")"
  assert_not_contains "$stderr_text" "Detected background processes"
  assert_contains "$stderr_text" "Unsupported matrix: invalid"
}

test_run_stability_defaults_to_require_host_preflight() {
  if ! has_local_runner_prerequisites; then
    echo "[skip] local runner prerequisites unavailable (scarb/jq/awk/sort/python3)"
    return 0
  fi

  local snapshot="$TEST_TMP_DIR/stability-require-noise.ps"
  write_snapshot "$snapshot" \
" 1001 /usr/local/bin/scarb-cairo-language-server"

  local stdout_file="$TEST_TMP_DIR/stability-require.out"
  local stderr_file="$TEST_TMP_DIR/stability-require.err"
  if UC_BENCH_PS_SNAPSHOT_FILE="$snapshot" \
    "$STABILITY_BENCH_SCRIPT" \
      --matrix smoke \
      --runs 12 \
      --cold-runs 12 \
      --cycles 1 \
      --allow-unpinned \
      >"$stdout_file" 2>"$stderr_file"; then
    echo "expected run_stability_benchmarks.sh to fail in default require mode on noisy host" >&2
    return 1
  fi

  local stderr_text
  stderr_text="$(cat "$stderr_file")"
  assert_contains "$stderr_text" "Detected background processes"
  assert_contains "$stderr_text" "Benchmark run failed for tool 'scarb' in cycle 1."
}

test_run_stability_allow_noisy_host_bypasses_preflight_failure() {
  if ! has_local_runner_prerequisites; then
    echo "[skip] local runner prerequisites unavailable (scarb/jq/awk/sort/python3)"
    return 0
  fi

  local snapshot="$TEST_TMP_DIR/stability-allow-noise.ps"
  write_snapshot "$snapshot" \
" 1002 /usr/local/bin/scarb --quiet proc-macro-server"

  local stdout_file="$TEST_TMP_DIR/stability-allow.out"
  local stderr_file="$TEST_TMP_DIR/stability-allow.err"
  if UC_BENCH_PS_SNAPSHOT_FILE="$snapshot" \
    "$STABILITY_BENCH_SCRIPT" \
      --matrix invalid \
      --runs 12 \
      --cold-runs 12 \
      --cycles 1 \
      --allow-unpinned \
      --allow-noisy-host \
      >"$stdout_file" 2>"$stderr_file"; then
    echo "expected run_stability_benchmarks.sh to fail with unsupported matrix" >&2
    return 1
  fi

  local stderr_text
  stderr_text="$(cat "$stderr_file")"
  assert_not_contains "$stderr_text" "Detected background processes"
  assert_contains "$stderr_text" "Unsupported matrix: invalid"
  assert_contains "$stderr_text" "Benchmark run failed for tool 'scarb' in cycle 1."
}

main() {
  run_test "detects proc-macro server noise" test_detects_proc_macro_server_noise
  run_test "detects language server noise" test_detects_language_server_noise
  run_test "clean snapshot has no noise" test_clean_snapshot_has_no_noise
  run_test "require mode fails with noise" test_preflight_require_fails_with_noise
  run_test "warn mode passes with warning" test_preflight_warn_passes_with_noise
  run_test "off mode skips check" test_preflight_off_skips_noise_check
  run_test "run_local require mode fails on noisy host" test_run_local_require_mode_fails_before_execution_on_noise
  run_test "run_local allow-noisy-host bypasses preflight failure" test_run_local_allow_noisy_host_bypasses_preflight_failure
  run_test "run_stability defaults to require host preflight" test_run_stability_defaults_to_require_host_preflight
  run_test "run_stability allow-noisy-host bypasses preflight failure" test_run_stability_allow_noisy_host_bypasses_preflight_failure
  echo "All benchmark host preflight tests passed."
}

main "$@"

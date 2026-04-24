#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
LIB_PATH="$SCRIPT_DIR/../lib/native_ci_gate.sh"
NATIVE_ONLY_SCRIPT="$SCRIPT_DIR/../run_native_only_gate.sh"
NATIVE_REAL_REPO_SMOKE_SCRIPT="$SCRIPT_DIR/../run_native_real_repo_smoke.sh"

if [[ ! -f "$LIB_PATH" ]]; then
  echo "missing native CI gate library at $LIB_PATH" >&2
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

run_test() {
  local name="$1"
  shift
  echo "[test] $name"
  "$@"
}

write_file() {
  local path="$1"
  local content="${2-}"
  # This helper intentionally writes a single multi-line string payload.
  printf '%s\n' "$content" > "$path"
}

write_mock_uc_bin() {
  local path="$1"
  cat > "$path" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

args_log="${MOCK_UC_ARGS_LOG:?}"
report_path=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --report-path)
      report_path="${2-}"
      shift 2
      ;;
    *)
      printf '%s\n' "$1" >> "$args_log"
      shift
      ;;
  esac
done

if [[ -z "$report_path" ]]; then
  echo "mock uc missing --report-path" >&2
  exit 1
fi

mkdir -p "$(dirname "$report_path")"
printf '%s\n' "$report_path" >> "$args_log"
printf '{"exit_code":0,"command":["uc-native","build"]}\n' > "$report_path"
EOF
  chmod +x "$path"
}

write_mock_scarb_bin() {
  local path="$1"
  cat > "$path" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

args_log="${MOCK_SCARB_ARGS_LOG:?}"
printf 'cwd=%s cmd=%s\n' "$PWD" "$*" >> "$args_log"
exit 0
EOF
  chmod +x "$path"
}

write_failing_mock_scarb_bin() {
  local path="$1"
  cat > "$path" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

echo "mock scarb fetch failure" >&2
exit 23
EOF
  chmod +x "$path"
}

write_mock_sleeping_uc_bin() {
  local path="$1"
  cat > "$path" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

sleep 2
exit 0
EOF
  chmod +x "$path"
}

test_detects_unsupported_executable_fixture_log() {
  local log_path="$TEST_TMP_DIR/executable.log"
  write_file "$log_path" \
"error[E2200]: Plugin diagnostic: Unsupported attribute.
 --> /tmp/ws/src/hello_world.cairo:1:1
#[executable]
^^^^^^^^^^^^^
native compile manifest includes non-starknet dependencies ([dependencies].cairo_execute)"

  uc_native_ci_log_indicates_unsupported "$log_path"
}

test_detects_generic_unsupported_capability_log() {
  local log_path="$TEST_TMP_DIR/generic-unsupported.log"
  write_file "$log_path" \
"Error: native compile does not support executable targets yet"

  uc_native_ci_log_indicates_unsupported "$log_path"
}

test_ignores_generic_failure_log() {
  local log_path="$TEST_TMP_DIR/generic-failure.log"
  write_file "$log_path" \
"Error: native compile failed

Caused by:
    Compilation failed."

  if uc_native_ci_log_indicates_unsupported "$log_path"; then
    echo "unsupported classifier should ignore generic failures" >&2
    return 1
  fi
}

test_verify_report_accepts_uc_native_backend() {
  local report_path="$TEST_TMP_DIR/uc-native.json"
  write_file "$report_path" '{"exit_code":0,"command":["uc-native","build"]}'

  uc_native_ci_verify_report "$report_path" "strict-native" "uc-native" >/dev/null
}

test_verify_report_rejects_scarb_fallback_for_native_only() {
  local report_path="$TEST_TMP_DIR/scarb.json"
  local stderr_path="$TEST_TMP_DIR/scarb.err"
  write_file "$report_path" '{"exit_code":0,"command":["scarb","build"]}'

  if uc_native_ci_verify_report "$report_path" "strict-native" "uc-native" >"$TEST_TMP_DIR/scarb.out" 2>"$stderr_path"; then
    echo "strict native verification should reject scarb fallback" >&2
    return 1
  fi

  if ! grep -q "unexpected backend" "$stderr_path"; then
    echo "expected backend rejection message in stderr" >&2
    cat "$stderr_path" >&2
    return 1
  fi
}

test_verify_report_accepts_controlled_fallback_backend() {
  local report_path="$TEST_TMP_DIR/allowed-scarb.json"
  write_file "$report_path" '{"exit_code":0,"command":["scarb","build"]}'

  uc_native_ci_verify_report "$report_path" "controlled-fallback" "scarb,uc-native" >/dev/null
}

test_verify_report_rejects_non_zero_exit_code() {
  local report_path="$TEST_TMP_DIR/non-zero-exit.json"
  local stderr_path="$TEST_TMP_DIR/non-zero-exit.err"
  write_file "$report_path" '{"exit_code":1,"command":["uc-native","build"]}'

  if uc_native_ci_verify_report "$report_path" "non-zero-exit" "uc-native" \
    >"$TEST_TMP_DIR/non-zero-exit.out" 2>"$stderr_path"; then
    echo "verification should reject non-zero exit_code reports" >&2
    return 1
  fi

  if ! grep -q "non-zero report exit code" "$stderr_path"; then
    echo "expected non-zero exit_code rejection message" >&2
    cat "$stderr_path" >&2
    return 1
  fi
}

test_native_only_script_rejects_missing_uc_bin_value() {
  local stderr_path="$TEST_TMP_DIR/native-only-missing-uc-bin.err"
  if "$NATIVE_ONLY_SCRIPT" --uc-bin >"$TEST_TMP_DIR/native-only-missing-uc-bin.out" 2>"$stderr_path"; then
    echo "expected native-only gate to reject missing --uc-bin value" >&2
    return 1
  fi

  if ! grep -q "Missing value for --uc-bin" "$stderr_path"; then
    echo "expected missing --uc-bin error" >&2
    cat "$stderr_path" >&2
    return 1
  fi
}

test_native_real_repo_smoke_rejects_missing_results_dir_value() {
  local stderr_path="$TEST_TMP_DIR/native-real-missing-results.err"
  if "$NATIVE_REAL_REPO_SMOKE_SCRIPT" --results-dir >"$TEST_TMP_DIR/native-real-missing-results.out" 2>"$stderr_path"; then
    echo "expected native real repo smoke script to reject missing --results-dir value" >&2
    return 1
  fi

  if ! grep -q "Missing value for --results-dir" "$stderr_path"; then
    echo "expected missing --results-dir error" >&2
    cat "$stderr_path" >&2
    return 1
  fi
}

test_native_real_repo_smoke_rejects_flag_shaped_case_operands() {
  local mock_uc="$TEST_TMP_DIR/mock-uc-flag-operands"
  write_mock_uc_bin "$mock_uc"
  local stderr_path="$TEST_TMP_DIR/native-real-flag-operands.err"
  if MOCK_UC_ARGS_LOG="$TEST_TMP_DIR/native-real-flag-operands.args" \
    "$NATIVE_REAL_REPO_SMOKE_SCRIPT" \
      --uc-bin "$mock_uc" \
      --results-dir "$TEST_TMP_DIR/results-flag-operands" \
      --strict-case --backend-case fake-tag \
      >"$TEST_TMP_DIR/native-real-flag-operands.out" 2>"$stderr_path"; then
    echo "expected native real repo smoke script to reject flag-shaped case operands" >&2
    return 1
  fi

  if ! grep -q "Missing value for --strict-case manifest-path" "$stderr_path"; then
    echo "expected strict-case manifest-path validation error" >&2
    cat "$stderr_path" >&2
    return 1
  fi
}

test_native_real_repo_smoke_requires_cases() {
  local mock_uc="$TEST_TMP_DIR/mock-uc-no-cases"
  write_mock_uc_bin "$mock_uc"
  local stderr_path="$TEST_TMP_DIR/native-real-no-cases.err"
  if MOCK_UC_ARGS_LOG="$TEST_TMP_DIR/native-real-no-cases.args" \
    "$NATIVE_REAL_REPO_SMOKE_SCRIPT" \
      --uc-bin "$mock_uc" \
      --results-dir "$TEST_TMP_DIR/results-no-cases" \
      >"$TEST_TMP_DIR/native-real-no-cases.out" 2>"$stderr_path"; then
    echo "expected native real repo smoke script to reject empty case configuration" >&2
    return 1
  fi

  if ! grep -q "requires at least one --strict-case or --backend-case" "$stderr_path"; then
    echo "expected empty case configuration error" >&2
    cat "$stderr_path" >&2
    return 1
  fi
}

test_native_real_repo_smoke_rejects_duplicate_tags() {
  local mock_uc="$TEST_TMP_DIR/mock-uc-duplicate-tags"
  write_mock_uc_bin "$mock_uc"
  local stderr_path="$TEST_TMP_DIR/native-real-duplicate-tags.err"
  if MOCK_UC_ARGS_LOG="$TEST_TMP_DIR/native-real-duplicate-tags.args" \
    "$NATIVE_REAL_REPO_SMOKE_SCRIPT" \
      --uc-bin "$mock_uc" \
      --results-dir "$TEST_TMP_DIR/results-duplicate-tags" \
      --strict-case "$TEST_TMP_DIR/a/Scarb.toml" same-tag \
      --backend-case "$TEST_TMP_DIR/b/Scarb.toml" same-tag uc-native \
      >"$TEST_TMP_DIR/native-real-duplicate-tags.out" 2>"$stderr_path"; then
    echo "expected native real repo smoke script to reject duplicate tags" >&2
    return 1
  fi

  if ! grep -q "Duplicate case tag: same-tag" "$stderr_path"; then
    echo "expected duplicate tag error" >&2
    cat "$stderr_path" >&2
    return 1
  fi
}

test_native_real_repo_smoke_rejects_invalid_tags() {
  local mock_uc="$TEST_TMP_DIR/mock-uc-invalid-tags"
  write_mock_uc_bin "$mock_uc"
  local stderr_path="$TEST_TMP_DIR/native-real-invalid-tags.err"
  if MOCK_UC_ARGS_LOG="$TEST_TMP_DIR/native-real-invalid-tags.args" \
    "$NATIVE_REAL_REPO_SMOKE_SCRIPT" \
      --uc-bin "$mock_uc" \
      --results-dir "$TEST_TMP_DIR/results-invalid-tags" \
      --strict-case "$TEST_TMP_DIR/a/Scarb.toml" ../escape \
      >"$TEST_TMP_DIR/native-real-invalid-tags.out" 2>"$stderr_path"; then
    echo "expected native real repo smoke script to reject invalid tags" >&2
    return 1
  fi

  if ! grep -q "Invalid case tag: ../escape" "$stderr_path"; then
    echo "expected invalid tag error" >&2
    cat "$stderr_path" >&2
    return 1
  fi
}

test_native_real_repo_smoke_passes_offline_to_uc() {
  local mock_uc="$TEST_TMP_DIR/mock-uc-offline"
  local args_log="$TEST_TMP_DIR/native-real-offline.args"
  local mock_bin_dir="$TEST_TMP_DIR/mock-bin-offline"
  local fake_manifest_dir="$TEST_TMP_DIR/fake"
  local fake_manifest_dir_abs
  mkdir -p "$mock_bin_dir"
  mkdir -p "$fake_manifest_dir"
  : > "$fake_manifest_dir/Scarb.toml"
  fake_manifest_dir_abs="$(cd "$fake_manifest_dir" && pwd -P)"
  write_mock_scarb_bin "$mock_bin_dir/scarb"
  write_mock_uc_bin "$mock_uc"

  PATH="$mock_bin_dir:$PATH" \
  MOCK_SCARB_ARGS_LOG="$TEST_TMP_DIR/native-real-offline.scarb.args" \
  MOCK_UC_ARGS_LOG="$args_log" \
    "$NATIVE_REAL_REPO_SMOKE_SCRIPT" \
      --uc-bin "$mock_uc" \
      --results-dir "$TEST_TMP_DIR/results-offline" \
      --backend-case "$fake_manifest_dir/Scarb.toml" fake-case uc-native \
      >"$TEST_TMP_DIR/native-real-offline.out" 2>"$TEST_TMP_DIR/native-real-offline.err"

  assert_contains "$(cat "$args_log")" "--offline"
  assert_contains "$(cat "$args_log")" "--daemon-mode"
  assert_contains "$(cat "$args_log")" "off"
  assert_contains "$(cat "$TEST_TMP_DIR/native-real-offline.scarb.args")" "cwd=$fake_manifest_dir_abs"
  assert_contains "$(cat "$TEST_TMP_DIR/native-real-offline.scarb.args")" "cmd=fetch"
}

test_native_real_repo_smoke_reports_prefetch_failure_context() {
  local mock_uc="$TEST_TMP_DIR/mock-uc-prefetch"
  local mock_bin_dir="$TEST_TMP_DIR/mock-bin-prefetch"
  local fake_manifest_dir="$TEST_TMP_DIR/fake-prefetch"
  local stderr_path="$TEST_TMP_DIR/native-real-prefetch.err"
  mkdir -p "$mock_bin_dir"
  mkdir -p "$fake_manifest_dir"
  : > "$fake_manifest_dir/Scarb.toml"
  write_failing_mock_scarb_bin "$mock_bin_dir/scarb"
  write_mock_uc_bin "$mock_uc"

  if PATH="$mock_bin_dir:$PATH" \
    MOCK_UC_ARGS_LOG="$TEST_TMP_DIR/native-real-prefetch.args" \
    "$NATIVE_REAL_REPO_SMOKE_SCRIPT" \
      --uc-bin "$mock_uc" \
      --results-dir "$TEST_TMP_DIR/results-prefetch" \
      --backend-case "$fake_manifest_dir/Scarb.toml" prefetch-case uc-native \
      >"$TEST_TMP_DIR/native-real-prefetch.out" 2>"$stderr_path"; then
    echo "expected native real repo smoke script to fail on scarb fetch" >&2
    return 1
  fi

  if ! grep -q "mock scarb fetch failure" "$stderr_path"; then
    echo "expected scarb fetch stderr to be preserved" >&2
    cat "$stderr_path" >&2
    return 1
  fi

  if ! grep -q "manifest_path=$fake_manifest_dir/Scarb.toml" "$stderr_path"; then
    echo "expected manifest path context in scarb fetch failure" >&2
    cat "$stderr_path" >&2
    return 1
  fi
}

test_native_real_repo_smoke_times_out_with_diagnostic() {
  local mock_uc="$TEST_TMP_DIR/mock-uc-timeout"
  local mock_bin_dir="$TEST_TMP_DIR/mock-bin-timeout"
  local fake_manifest_dir="$TEST_TMP_DIR/fake-timeout"
  local stderr_path="$TEST_TMP_DIR/native-real-timeout.err"
  mkdir -p "$mock_bin_dir"
  mkdir -p "$fake_manifest_dir"
  : > "$fake_manifest_dir/Scarb.toml"
  write_mock_scarb_bin "$mock_bin_dir/scarb"
  write_mock_sleeping_uc_bin "$mock_uc"

  if PATH="$mock_bin_dir:$PATH" \
    MOCK_SCARB_ARGS_LOG="$TEST_TMP_DIR/native-real-timeout.scarb.args" \
    MOCK_UC_ARGS_LOG="$TEST_TMP_DIR/native-real-timeout.args" \
    "$NATIVE_REAL_REPO_SMOKE_SCRIPT" \
      --uc-bin "$mock_uc" \
      --results-dir "$TEST_TMP_DIR/results-timeout" \
      --timeout-secs 1 \
      --backend-case "$fake_manifest_dir/Scarb.toml" timeout-case uc-native \
      >"$TEST_TMP_DIR/native-real-timeout.out" 2>"$stderr_path"; then
    echo "expected native real repo smoke script to time out" >&2
    return 1
  fi

  if ! grep -q "exit code 124" "$stderr_path"; then
    echo "expected timeout exit code in stderr" >&2
    cat "$stderr_path" >&2
    return 1
  fi

  if ! grep -q "timed out after 1s" "$TEST_TMP_DIR/results-timeout/native-real-timeout-case.log"; then
    echo "expected timeout marker in case log" >&2
    cat "$TEST_TMP_DIR/results-timeout/native-real-timeout-case.log" >&2
    return 1
  fi
}

run_test "detects unsupported executable fixture log" test_detects_unsupported_executable_fixture_log
run_test "detects generic unsupported capability log" test_detects_generic_unsupported_capability_log
run_test "ignores generic failure log" test_ignores_generic_failure_log
run_test "verify report accepts uc-native backend" test_verify_report_accepts_uc_native_backend
run_test "verify report rejects scarb fallback for native-only" test_verify_report_rejects_scarb_fallback_for_native_only
run_test "verify report accepts controlled fallback backend" test_verify_report_accepts_controlled_fallback_backend
run_test "verify report rejects non-zero exit_code" test_verify_report_rejects_non_zero_exit_code
run_test "native-only script rejects missing uc-bin value" test_native_only_script_rejects_missing_uc_bin_value
run_test "native real repo smoke rejects missing results-dir value" test_native_real_repo_smoke_rejects_missing_results_dir_value
run_test "native real repo smoke rejects flag-shaped case operands" test_native_real_repo_smoke_rejects_flag_shaped_case_operands
run_test "native real repo smoke requires cases" test_native_real_repo_smoke_requires_cases
run_test "native real repo smoke rejects duplicate tags" test_native_real_repo_smoke_rejects_duplicate_tags
run_test "native real repo smoke rejects invalid tags" test_native_real_repo_smoke_rejects_invalid_tags
run_test "native real repo smoke passes offline to uc" test_native_real_repo_smoke_passes_offline_to_uc
run_test "native real repo smoke reports prefetch failure context" test_native_real_repo_smoke_reports_prefetch_failure_context
run_test "native real repo smoke times out with diagnostic" test_native_real_repo_smoke_times_out_with_diagnostic

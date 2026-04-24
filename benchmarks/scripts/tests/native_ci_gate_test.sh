#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
LIB_PATH="$SCRIPT_DIR/../lib/native_ci_gate.sh"

if [[ ! -f "$LIB_PATH" ]]; then
  echo "missing native CI gate library at $LIB_PATH" >&2
  exit 1
fi

# shellcheck source=/dev/null
source "$LIB_PATH"

TEST_TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TEST_TMP_DIR"' EXIT

run_test() {
  local name="$1"
  shift
  echo "[test] $name"
  "$@"
}

write_file() {
  local path="$1"
  shift
  cat > "$path" <<EOF
$*
EOF
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

run_test "detects unsupported executable fixture log" test_detects_unsupported_executable_fixture_log
run_test "detects generic unsupported capability log" test_detects_generic_unsupported_capability_log
run_test "ignores generic failure log" test_ignores_generic_failure_log
run_test "verify report accepts uc-native backend" test_verify_report_accepts_uc_native_backend
run_test "verify report rejects scarb fallback for native-only" test_verify_report_rejects_scarb_fallback_for_native_only
run_test "verify report accepts controlled fallback backend" test_verify_report_accepts_controlled_fallback_backend

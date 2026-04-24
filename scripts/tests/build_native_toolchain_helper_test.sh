#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
ROOT="$(cd "$SCRIPT_DIR/../.." && pwd -P)"
HELPER_SCRIPT="$ROOT/scripts/build_native_toolchain_helper.sh"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

run_test() {
  echo "[test] $1"
  shift
  "$@"
}

test_prepare_only_rewrites_workspace_manifest_for_cairo214() {
  local stage_dir="$TMP_DIR/stage"
  local stdout_path="$TMP_DIR/prepare.out"
  "$HELPER_SCRIPT" --lane 2.14 --staging-dir "$stage_dir" --prepare-only >"$stdout_path"

  grep -q "Prepared helper staging tree:" "$stdout_path"
  grep -q 'cairo-lang-compiler = "=2.14.0"' "$stage_dir/Cargo.toml"
  grep -q 'salsa = "0.24.0"' "$stage_dir/Cargo.toml"
  if grep -q '^\[patch\.crates-io\]' "$stage_dir/Cargo.toml"; then
    echo "expected helper staging Cargo.toml to drop [patch.crates-io]" >&2
    return 1
  fi
  cmp "$ROOT/toolchains/cairo-2.14/Cargo.lock" "$stage_dir/Cargo.lock" >/dev/null
}

test_prepare_only_and_check_only_are_mutually_exclusive() {
  local stdout_path="$TMP_DIR/mutually-exclusive.out"
  if "$HELPER_SCRIPT" --lane 2.14 --prepare-only --check-only >"$stdout_path" 2>&1; then
    echo "expected mutually exclusive helper modes to fail" >&2
    return 1
  fi
  grep -q -- '--prepare-only and --check-only cannot be used together' "$stdout_path"
}

test_unsupported_lane_reports_actionable_error() {
  local stdout_path="$TMP_DIR/unsupported-lane.out"
  if "$HELPER_SCRIPT" --lane 9.99 --prepare-only >"$stdout_path" 2>&1; then
    echo "expected unsupported helper lane to fail" >&2
    return 1
  fi
  grep -q 'unsupported helper lane: 9.99' "$stdout_path"
  grep -q 'Available lanes: 2.14' "$stdout_path"
  if grep -q 'Traceback' "$stdout_path"; then
    echo "unsupported helper lane should not emit a Python traceback" >&2
    cat "$stdout_path" >&2
    return 1
  fi
}

test_existing_staging_dir_is_not_removed_on_failure() {
  local stage_dir="$TMP_DIR/preexisting-stage"
  local stdout_path="$TMP_DIR/preexisting-stage.out"
  mkdir -p "$stage_dir"
  printf 'do not delete\n' > "$stage_dir/sentinel.txt"

  if "$HELPER_SCRIPT" --lane 2.14 --staging-dir "$stage_dir" --check-only >"$stdout_path" 2>&1; then
    echo "expected pre-existing staging dir to fail" >&2
    return 1
  fi
  grep -q 'staging dir already exists:' "$stdout_path"
  if [[ ! -f "$stage_dir/sentinel.txt" ]]; then
    echo "pre-existing staging dir was removed by cleanup trap" >&2
    return 1
  fi
}

run_test "prepare_only_rewrites_workspace_manifest_for_cairo214" \
  test_prepare_only_rewrites_workspace_manifest_for_cairo214
run_test "prepare_only_and_check_only_are_mutually_exclusive" \
  test_prepare_only_and_check_only_are_mutually_exclusive
run_test "unsupported_lane_reports_actionable_error" \
  test_unsupported_lane_reports_actionable_error
run_test "existing_staging_dir_is_not_removed_on_failure" \
  test_existing_staging_dir_is_not_removed_on_failure

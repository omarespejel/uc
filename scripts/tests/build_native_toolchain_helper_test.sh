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

run_test "prepare_only_rewrites_workspace_manifest_for_cairo214" \
  test_prepare_only_rewrites_workspace_manifest_for_cairo214
run_test "prepare_only_and_check_only_are_mutually_exclusive" \
  test_prepare_only_and_check_only_are_mutually_exclusive

#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

run_case() {
  local name="$1"
  local changed="$2"
  local expected="$3"
  local changed_file="$TMP_DIR/$name.changed"
  local capture_file="$TMP_DIR/$name.capture"
  local expected_file="$TMP_DIR/$name.expected"

  printf '%s' "$changed" >"$changed_file"
  : >"$capture_file"
  printf '%s' "$expected" >"$expected_file"

  UC_LOCAL_CI_CHANGED_FILES_FILE="$changed_file" \
    UC_LOCAL_CI_CAPTURE_PATH="$capture_file" \
    "$ROOT/scripts/local_ci_gate.sh"

  diff -u "$expected_file" "$capture_file"
}

run_case \
  "docs-only" \
  $'AGENTS.md\n' \
  $'make doctor\nmake agent-validate\n'

run_case \
  "bench-only" \
  $'benchmarks/scripts/run_local_benchmarks.sh\n' \
  $'make doctor\nmake validate-bench-scripts\n'

run_case \
  "rust-fast" \
  $'crates/uc-core/src/lib.rs\n' \
  $'make doctor\nmake validate-fast\n'

run_case \
  "native" \
  $'crates/uc-cli/src/main.rs\n' \
  $'make doctor\nmake validate-native\n'

run_case \
  "empty" \
  '' \
  $'make doctor\nmake agent-validate\n'

printf 'local_ci_gate tests passed\n'

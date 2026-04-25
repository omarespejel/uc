#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
BUILDER_SCRIPT="$SCRIPT_DIR/../build_deployed_contract_source_index.sh"
GENERATOR_SCRIPT="$SCRIPT_DIR/../generate_deployed_contract_corpus.sh"
CORPUS_SCRIPT="$SCRIPT_DIR/../run_deployed_contract_corpus.sh"

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

extract_labeled_path() {
  local label="$1"
  awk -v prefix="$label: " 'index($0, prefix) == 1 {sub(prefix, ""); print}' | tail -n 1
}

write_manifest_case() {
  local root="$1"
  local name="$2"
  mkdir -p "$root/$name/src"
  cat > "$root/$name/Scarb.toml" <<MANIFEST
[package]
name = "$name"
version = "0.1.0"
edition = "2024_07"
cairo-version = "2.14.0"
MANIFEST
  cat > "$root/$name/src/lib.cairo" <<'SRC'
fn main() -> felt252 {
    1
}
SRC
}

inventory_record_json() {
  local tag="$1"
  local manifest_path="$2"
  local class_hash="$3"
  local cairo_version="$4"
  local source_package_id="${5-}"
  jq -n \
    --arg tag "$tag" \
    --arg manifest_path "$manifest_path" \
    --arg class_hash "$class_hash" \
    --arg cairo_version "$cairo_version" \
    --arg source_package_id "$source_package_id" \
    '{
      tag: $tag,
      source_kind: "deployed_contract",
      contract_address: "0x123",
      class_hash: $class_hash,
      source_ref: "local inventory fixture",
      manifest_path: $manifest_path,
      cairo_version: $cairo_version,
      scarb_version: "2.14.0",
      license: "fixture",
      notes: "test record"
    } + (if $source_package_id == "" then {} else {source_package_id: $source_package_id} end)'
}

write_inventory_file() {
  local path="$1"
  local coverage="$2"
  local dedupe_key="$3"
  shift 3
  local records_json
  records_json="[$(IFS=,; echo "$*")]"
  cat > "$path" <<JSON
{
  "schema_version": 1,
  "corpus_id": "test-inventory-$coverage",
  "chain": "starknet-mainnet",
  "selection": {
    "source": "local test fixture",
    "snapshot_id": "test-snapshot",
    "from_block": 1,
    "to_block": 2,
    "coverage": "$coverage",
    "notes": "test selection"
  },
  "deduplication": {
    "key": "$dedupe_key",
    "rules": "keep first row per dedupe key"
  },
  "license_policy": "test fixtures only",
  "source_availability": {
    "policy": "local_manifest_paths",
    "notes": "all rows point at local manifests"
  },
  "records": $records_json
}
JSON
}

mutate_inventory() {
  local inventory_path="$1"
  local filter="$2"
  jq "$filter" "$inventory_path" > "$inventory_path.tmp"
  mv "$inventory_path.tmp" "$inventory_path"
}

expect_builder_failure() {
  local inventory_path="$1"
  local expected="$2"
  local out_path="${3:-$(dirname "$inventory_path")/failure-source-index.json}"
  local stderr_path="$TEST_TMP_DIR/failure.err"
  if "$BUILDER_SCRIPT" --inventory "$inventory_path" --out "$out_path" >"$TEST_TMP_DIR/failure.out" 2>"$stderr_path"; then
    echo "expected builder to fail" >&2
    cat "$TEST_TMP_DIR/failure.out" >&2
    return 1
  fi
  if ! grep -Fq -- "$expected" "$stderr_path"; then
    echo "expected builder error: $expected" >&2
    cat "$stderr_path" >&2
    return 1
  fi
}

test_builds_deduped_source_index_and_corpus() {
  local inventory_dir="$TEST_TMP_DIR/build/inventory"
  local case_root="$inventory_dir/cases"
  local out_dir="$inventory_dir"
  local results_dir="$TEST_TMP_DIR/build/results"
  mkdir -p "$inventory_dir" "$out_dir" "$results_dir"
  write_manifest_case "$case_root" "first"
  write_manifest_case "$case_root" "dupe"
  local record_a record_b stdout_text source_index_path corpus_stdout corpus_path plan_stdout plan_json
  record_a="$(inventory_record_json first "cases/first/Scarb.toml" "0xsame" "2.14.0")"
  record_b="$(inventory_record_json dupe "cases/dupe/Scarb.toml" "0xsame" "2.14.0")"
  write_inventory_file "$inventory_dir/inventory.json" sample class_hash "$record_a" "$record_b"

  stdout_text="$("$BUILDER_SCRIPT" --inventory "$inventory_dir/inventory.json" --out "$out_dir/source-index.json")"
  assert_contains "$stdout_text" "Source index JSON:"
  source_index_path="$(extract_labeled_path "Source index JSON" <<<"$stdout_text")"
  [[ -f "$source_index_path" ]] || { echo "missing source index: $source_index_path" >&2; return 1; }

  if [[ "$(jq -r '.deduplication.input_count' "$source_index_path")" != "2" || "$(jq -r '.deduplication.deduped_count' "$source_index_path")" != "1" ]]; then
    echo "unexpected dedupe counts" >&2
    cat "$source_index_path" >&2
    return 1
  fi
  if [[ "$(jq -r '.items[0].tag' "$source_index_path")" != "first" ]]; then
    echo "expected first source record to win dedupe" >&2
    cat "$source_index_path" >&2
    return 1
  fi
  if [[ "$(jq -r '.items[0].manifest_path' "$source_index_path")" = /* ]]; then
    echo "source-index manifest path must stay relative" >&2
    cat "$source_index_path" >&2
    return 1
  fi

  corpus_stdout="$("$GENERATOR_SCRIPT" --source-index "$source_index_path" --out "$out_dir/corpus.json")"
  assert_contains "$corpus_stdout" "Corpus JSON:"
  corpus_path="$(extract_labeled_path "Corpus JSON" <<<"$corpus_stdout")"
  plan_stdout="$("$CORPUS_SCRIPT" --corpus "$corpus_path" --results-dir "$results_dir" --plan-only)"
  assert_contains "$plan_stdout" "Corpus plan JSON:"
  plan_json="$(extract_labeled_path "Corpus plan JSON" <<<"$plan_stdout")"
  [[ "$(jq -r '.corpus.summary.item_count' "$plan_json")" == "1" ]] || { cat "$plan_json" >&2; return 1; }
}

test_rejects_unknown_keys_and_boolean_ints() {
  local inventory_dir="$TEST_TMP_DIR/invalid/inventory"
  local case_root="$inventory_dir/cases"
  mkdir -p "$inventory_dir"
  write_manifest_case "$case_root" "a"
  local record
  record="$(inventory_record_json a "cases/a/Scarb.toml" "0x01" "2.14.0")"
  write_inventory_file "$inventory_dir/inventory.json" sample class_hash "$record"
  mutate_inventory "$inventory_dir/inventory.json" '.unexpected = true'
  expect_builder_failure "$inventory_dir/inventory.json" "inventory has unsupported field(s): unexpected"

  write_inventory_file "$inventory_dir/inventory-schema-bool.json" sample class_hash "$record"
  mutate_inventory "$inventory_dir/inventory-schema-bool.json" '.schema_version = true'
  expect_builder_failure "$inventory_dir/inventory-schema-bool.json" "inventory.schema_version must be 1"

  write_inventory_file "$inventory_dir/inventory-bool.json" sample class_hash "$record"
  mutate_inventory "$inventory_dir/inventory-bool.json" '.selection.from_block = true'
  expect_builder_failure "$inventory_dir/inventory-bool.json" "inventory.selection.from_block must be a non-negative integer"
}

test_rejects_duplicate_tags_and_absolute_manifest() {
  local inventory_dir="$TEST_TMP_DIR/dupes/inventory"
  local case_root="$inventory_dir/cases"
  mkdir -p "$inventory_dir"
  write_manifest_case "$case_root" "a"
  write_manifest_case "$case_root" "b"
  local record_a record_b
  record_a="$(inventory_record_json same "cases/a/Scarb.toml" "0x01" "2.14.0")"
  record_b="$(inventory_record_json same "cases/b/Scarb.toml" "0x02" "2.14.0")"
  write_inventory_file "$inventory_dir/duplicate-tags.json" sample none "$record_a" "$record_b"
  expect_builder_failure "$inventory_dir/duplicate-tags.json" "duplicate inventory record tag: same"

  record_a="$(inventory_record_json absolute "$case_root/a/Scarb.toml" "0x01" "2.14.0")"
  write_inventory_file "$inventory_dir/absolute.json" sample class_hash "$record_a"
  expect_builder_failure "$inventory_dir/absolute.json" "manifest_path for absolute must be relative to the inventory"
}

test_rejects_manifest_path_traversal() {
  local inventory_dir="$TEST_TMP_DIR/traversal/inventory"
  local escape_root="$TEST_TMP_DIR/traversal/outside"
  mkdir -p "$inventory_dir"
  write_manifest_case "$escape_root" "a"
  local record
  record="$(inventory_record_json escape "../outside/a/Scarb.toml" "0x01" "2.14.0")"
  write_inventory_file "$inventory_dir/inventory.json" sample class_hash "$record"
  expect_builder_failure "$inventory_dir/inventory.json" "manifest_path for escape must stay under the inventory directory"
}

test_rejects_source_package_dedupe_without_id() {
  local inventory_dir="$TEST_TMP_DIR/source-package/inventory"
  local case_root="$inventory_dir/cases"
  mkdir -p "$inventory_dir"
  write_manifest_case "$case_root" "a"
  local record
  record="$(inventory_record_json a "cases/a/Scarb.toml" "0x01" "2.14.0")"
  write_inventory_file "$inventory_dir/inventory.json" sample source_package "$record"
  expect_builder_failure "$inventory_dir/inventory.json" "source_package_id must be a non-empty string when deduplication.key is source_package"
}

test_rejects_null_source_package_id_when_present() {
  local inventory_dir="$TEST_TMP_DIR/source-package-null/inventory"
  local case_root="$inventory_dir/cases"
  mkdir -p "$inventory_dir"
  write_manifest_case "$case_root" "a"
  local record
  record="$(inventory_record_json a "cases/a/Scarb.toml" "0x01" "2.14.0")"
  write_inventory_file "$inventory_dir/inventory.json" sample class_hash "$record"
  mutate_inventory "$inventory_dir/inventory.json" '.records[0].source_package_id = null'
  expect_builder_failure "$inventory_dir/inventory.json" "inventory.records[0].source_package_id must be a non-empty string"
}

test_accepts_declared_class_without_contract_address() {
  local inventory_dir="$TEST_TMP_DIR/declared-class/inventory"
  local case_root="$inventory_dir/cases"
  mkdir -p "$inventory_dir"
  write_manifest_case "$case_root" "a"
  local record source_index_path stdout_text
  record="$(inventory_record_json class_only "cases/a/Scarb.toml" "0xclass" "2.14.0")"
  write_inventory_file "$inventory_dir/inventory.json" sample class_hash "$record"
  mutate_inventory "$inventory_dir/inventory.json" '.records[0].source_kind = "declared_class" | del(.records[0].contract_address)'

  stdout_text="$("$BUILDER_SCRIPT" --inventory "$inventory_dir/inventory.json" --out "$inventory_dir/source-index.json")"
  source_index_path="$(extract_labeled_path "Source index JSON" <<<"$stdout_text")"
  if [[ "$(jq -r '.items[0].source_kind' "$source_index_path")" != "declared_class" || "$(jq -r '.items[0] | has("contract_address")' "$source_index_path")" != "false" ]]; then
    echo "declared class source-index row should not require or synthesize contract_address" >&2
    cat "$source_index_path" >&2
    return 1
  fi
}

test_rejects_empty_declared_class_contract_address() {
  local inventory_dir="$TEST_TMP_DIR/declared-class-empty/inventory"
  local case_root="$inventory_dir/cases"
  mkdir -p "$inventory_dir"
  write_manifest_case "$case_root" "a"
  local record
  record="$(inventory_record_json class_only "cases/a/Scarb.toml" "0xclass" "2.14.0")"
  write_inventory_file "$inventory_dir/inventory.json" sample class_hash "$record"
  mutate_inventory "$inventory_dir/inventory.json" '.records[0].source_kind = "declared_class" | .records[0].contract_address = ""'
  expect_builder_failure "$inventory_dir/inventory.json" "inventory.records[0].contract_address must be a non-empty string"
}

test_normalizes_legacy_missing_source_kind_as_deployed_contract() {
  local inventory_dir="$TEST_TMP_DIR/legacy-source-kind/inventory"
  local case_root="$inventory_dir/cases"
  mkdir -p "$inventory_dir"
  write_manifest_case "$case_root" "a"
  local record source_index_path stdout_text
  record="$(inventory_record_json legacy "cases/a/Scarb.toml" "0xlegacy" "2.14.0")"
  write_inventory_file "$inventory_dir/inventory.json" sample class_hash "$record"
  mutate_inventory "$inventory_dir/inventory.json" 'del(.records[0].source_kind)'

  stdout_text="$("$BUILDER_SCRIPT" --inventory "$inventory_dir/inventory.json" --out "$inventory_dir/source-index.json")"
  source_index_path="$(extract_labeled_path "Source index JSON" <<<"$stdout_text")"
  if [[ "$(jq -r '.items[0].source_kind' "$source_index_path")" != "deployed_contract" ]]; then
    echo "legacy missing source_kind should normalize to deployed_contract" >&2
    cat "$source_index_path" >&2
    return 1
  fi
}

test_rejects_outside_output_directory() {
  local inventory_dir="$TEST_TMP_DIR/outside-out/inventory"
  local case_root="$inventory_dir/cases"
  local out_dir="$TEST_TMP_DIR/outside-out/out"
  mkdir -p "$inventory_dir" "$out_dir"
  write_manifest_case "$case_root" "a"
  local record
  record="$(inventory_record_json a "cases/a/Scarb.toml" "0x01" "2.14.0")"
  write_inventory_file "$inventory_dir/inventory.json" sample class_hash "$record"
  expect_builder_failure "$inventory_dir/inventory.json" "--out must be written next to --inventory" "$out_dir/source-index.json"
}

test_rejects_inventory_output_overwrite_aliases() {
  local inventory_dir="$TEST_TMP_DIR/overwrite/inventory"
  local case_root="$inventory_dir/cases"
  mkdir -p "$inventory_dir"
  write_manifest_case "$case_root" "a"
  local record inventory_path out_path stderr_path exit_code
  record="$(inventory_record_json a "cases/a/Scarb.toml" "0x01" "2.14.0")"
  inventory_path="$inventory_dir/inventory.json"
  write_inventory_file "$inventory_path" sample class_hash "$record"
  expect_builder_failure "$inventory_path" "Refusing to overwrite source inventory with generated source index" "$inventory_path"

  out_path="$inventory_dir/inventory-alias.json"
  ln -s "$inventory_path" "$out_path"
  stderr_path="$TEST_TMP_DIR/overwrite-symlink.err"
  set +e
  "$BUILDER_SCRIPT" --inventory "$inventory_path" --out "$out_path" >"$TEST_TMP_DIR/overwrite-symlink.out" 2>"$stderr_path"
  exit_code=$?
  set -e
  if [[ "$exit_code" -ne 2 ]]; then
    echo "expected symlink overwrite to exit with code 2, got: $exit_code" >&2
    cat "$TEST_TMP_DIR/overwrite-symlink.out" >&2
    cat "$stderr_path" >&2
    return 1
  fi
  if ! grep -Fq -- "Refusing to overwrite source inventory with generated source index" "$stderr_path"; then
    echo "expected symlink overwrite guard" >&2
    cat "$stderr_path" >&2
    return 1
  fi
}

run_test "builds deduped source index and corpus" test_builds_deduped_source_index_and_corpus
run_test "rejects unknown keys and boolean ints" test_rejects_unknown_keys_and_boolean_ints
run_test "rejects duplicate tags and absolute manifest" test_rejects_duplicate_tags_and_absolute_manifest
run_test "rejects manifest path traversal" test_rejects_manifest_path_traversal
run_test "rejects source_package dedupe without id" test_rejects_source_package_dedupe_without_id
run_test "rejects null source_package_id when present" test_rejects_null_source_package_id_when_present
run_test "accepts declared_class without contract_address" test_accepts_declared_class_without_contract_address
run_test "rejects empty declared_class contract_address" test_rejects_empty_declared_class_contract_address
run_test "normalizes legacy missing source_kind as deployed_contract" test_normalizes_legacy_missing_source_kind_as_deployed_contract
run_test "rejects outside output directory" test_rejects_outside_output_directory
run_test "rejects inventory output overwrite aliases" test_rejects_inventory_output_overwrite_aliases

echo "build_deployed_contract_source_index_test.sh: ok"

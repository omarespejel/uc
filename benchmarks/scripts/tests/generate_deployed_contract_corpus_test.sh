#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
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

source_item_json() {
  local tag="$1"
  local manifest_path="$2"
  local class_hash="$3"
  local cairo_version="$4"
  jq -n \
    --arg tag "$tag" \
    --arg manifest_path "$manifest_path" \
    --arg class_hash "$class_hash" \
    --arg cairo_version "$cairo_version" \
    '{
      tag: $tag,
      source_kind: "deployed_contract",
      contract_address: "0x123",
      class_hash: $class_hash,
      source_ref: "local test fixture",
      manifest_path: $manifest_path,
      cairo_version: $cairo_version,
      scarb_version: "2.14.0",
      license: "fixture",
      notes: "test item"
    }'
}

write_source_index_file() {
  local path="$1"
  local coverage="$2"
  local dedupe_key="$3"
  shift 3
  local items_json
  items_json="[$(IFS=,; echo "$*")]"
  cat > "$path" <<JSON
{
  "schema_version": 1,
  "corpus_id": "test-source-index-$coverage",
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
    "input_count": $(jq 'length' <<<"$items_json"),
    "deduped_count": $(jq 'length' <<<"$items_json"),
    "rules": "test rows"
  },
  "license_policy": "test fixtures only",
  "source_availability": {
    "policy": "local_manifest_paths",
    "notes": "all rows point at local manifests"
  },
  "items": $items_json
}
JSON
}

expect_generator_failure() {
  local source_index_path="$1"
  local expected="$2"
  local out_path="${3:-$TEST_TMP_DIR/failure-out.json}"
  local stderr_path="$TEST_TMP_DIR/failure.err"
  if "$GENERATOR_SCRIPT" --source-index "$source_index_path" --out "$out_path" >"$TEST_TMP_DIR/failure.out" 2>"$stderr_path"; then
    echo "expected generator to fail" >&2
    cat "$TEST_TMP_DIR/failure.out" >&2
    return 1
  fi
  if ! grep -Fq -- "$expected" "$stderr_path"; then
    echo "expected generator error: $expected" >&2
    cat "$stderr_path" >&2
    return 1
  fi
}

mutate_source_index() {
  local source_index_path="$1"
  local filter="$2"
  jq "$filter" "$source_index_path" > "$source_index_path.tmp"
  mv "$source_index_path.tmp" "$source_index_path"
}

test_generates_corpus_and_plan_only_accepts_it() {
  local index_dir="$TEST_TMP_DIR/generate/index"
  local case_root="$index_dir/cases"
  local results_dir="$TEST_TMP_DIR/generate/results"
  local out_dir="$TEST_TMP_DIR/generate/out"
  mkdir -p "$index_dir" "$results_dir" "$out_dir"
  write_manifest_case "$case_root" "sample"
  local item stdout_text corpus_path plan_stdout plan_json manifest_path source_availability
  item="$(source_item_json sample "cases/sample/Scarb.toml" "0x01" "2.14.0")"
  write_source_index_file "$index_dir/source-index.json" sample class_hash "$item"

  stdout_text="$(
    "$GENERATOR_SCRIPT" \
      --source-index "$index_dir/source-index.json" \
      --out "$out_dir/corpus.json"
  )"
  assert_contains "$stdout_text" "Corpus JSON:"
  corpus_path="$(extract_labeled_path "Corpus JSON" <<<"$stdout_text")"
  [[ -f "$corpus_path" ]] || { echo "missing generated corpus: $corpus_path" >&2; return 1; }

  manifest_path="$(jq -r '.items[0].manifest_path' "$corpus_path")"
  source_availability="$(jq -r 'has("source_availability")' "$corpus_path")"
  if [[ "$manifest_path" != /* || "$source_availability" != "false" ]]; then
    echo "unexpected generated corpus shape" >&2
    cat "$corpus_path" >&2
    return 1
  fi

  plan_stdout="$(
    "$CORPUS_SCRIPT" \
      --corpus "$corpus_path" \
      --results-dir "$results_dir" \
      --plan-only
  )"
  assert_contains "$plan_stdout" "Corpus plan JSON:"
  plan_json="$(extract_labeled_path "Corpus plan JSON" <<<"$plan_stdout")"
  [[ -f "$plan_json" ]] || { echo "missing plan artifact: $plan_json" >&2; return 1; }
  if [[ "$(jq -r '.corpus.summary.item_count' "$plan_json")" != "1" ]]; then
    echo "unexpected plan artifact" >&2
    cat "$plan_json" >&2
    return 1
  fi
}

test_rejects_unknown_top_level_keys() {
  local index_dir="$TEST_TMP_DIR/unknown/index"
  local case_root="$index_dir/cases"
  mkdir -p "$index_dir"
  write_manifest_case "$case_root" "a"
  local item
  item="$(source_item_json a "cases/a/Scarb.toml" "0x01" "2.14.0")"
  write_source_index_file "$index_dir/source-index.json" sample class_hash "$item"
  mutate_source_index "$index_dir/source-index.json" '.unexpected = true'
  expect_generator_failure "$index_dir/source-index.json" "source_index has unsupported field(s): unexpected"
}

test_rejects_boolean_integer_fields() {
  local index_dir="$TEST_TMP_DIR/bool/index"
  local case_root="$index_dir/cases"
  mkdir -p "$index_dir"
  write_manifest_case "$case_root" "a"
  local item
  item="$(source_item_json a "cases/a/Scarb.toml" "0x01" "2.14.0")"
  write_source_index_file "$index_dir/source-index.json" sample class_hash "$item"
  mutate_source_index "$index_dir/source-index.json" '.selection.from_block = true'
  expect_generator_failure "$index_dir/source-index.json" "source_index.selection.from_block must be a non-negative integer"
}

test_rejects_non_string_optional_fields() {
  local index_dir="$TEST_TMP_DIR/optional/index"
  local case_root="$index_dir/cases"
  mkdir -p "$index_dir"
  write_manifest_case "$case_root" "a"
  local item
  item="$(source_item_json a "cases/a/Scarb.toml" "0x01" "2.14.0")"

  local -a filters=(
    '.selection.notes = {}'
    '.deduplication.rules = []'
    '.source_availability.notes = false'
    '.items[0].scarb_version = false'
    '.items[0].license = {}'
    '.items[0].notes = []'
  )
  local -a messages=(
    'source_index.selection.notes must be a string'
    'source_index.deduplication.rules must be a string'
    'source_index.source_availability.notes must be a string'
    'source_index.items[0].scarb_version must be a string'
    'source_index.items[0].license must be a string'
    'source_index.items[0].notes must be a string'
  )

  local index source_index_path
  for index in "${!filters[@]}"; do
    source_index_path="$index_dir/source-index-$index.json"
    write_source_index_file "$source_index_path" sample class_hash "$item"
    mutate_source_index "$source_index_path" "${filters[$index]}"
    expect_generator_failure "$source_index_path" "${messages[$index]}"
  done
}

test_rejects_duplicate_tags() {
  local index_dir="$TEST_TMP_DIR/dupe-tag/index"
  local case_root="$index_dir/cases"
  mkdir -p "$index_dir"
  write_manifest_case "$case_root" "a"
  write_manifest_case "$case_root" "b"
  local item_a item_b
  item_a="$(source_item_json same "cases/a/Scarb.toml" "0x01" "2.14.0")"
  item_b="$(source_item_json same "cases/b/Scarb.toml" "0x02" "2.14.0")"
  write_source_index_file "$index_dir/source-index.json" sample none "$item_a" "$item_b"
  expect_generator_failure "$index_dir/source-index.json" "duplicate source index item tag: same"
}

test_rejects_duplicate_class_hash_when_class_deduped() {
  local index_dir="$TEST_TMP_DIR/dupe-class/index"
  local case_root="$index_dir/cases"
  mkdir -p "$index_dir"
  write_manifest_case "$case_root" "a"
  write_manifest_case "$case_root" "b"
  local item_a item_b
  item_a="$(source_item_json a "cases/a/Scarb.toml" "0xsame" "2.14.0")"
  item_b="$(source_item_json b "cases/b/Scarb.toml" "0xsame" "2.14.0")"
  write_source_index_file "$index_dir/source-index.json" sample class_hash "$item_a" "$item_b"
  expect_generator_failure "$index_dir/source-index.json" "duplicate class_hash in class_hash-deduped source index: 0xsame"
}

test_generates_declared_class_without_contract_address() {
  local index_dir="$TEST_TMP_DIR/declared-class/index"
  local case_root="$index_dir/cases"
  local out_dir="$TEST_TMP_DIR/declared-class/out"
  mkdir -p "$index_dir" "$out_dir"
  write_manifest_case "$case_root" "a"
  local item stdout_text corpus_path
  item="$(source_item_json class_only "cases/a/Scarb.toml" "0xclass" "2.14.0")"
  item="$(jq '.source_kind = "declared_class" | del(.contract_address)' <<<"$item")"
  write_source_index_file "$index_dir/source-index.json" sample class_hash "$item"

  stdout_text="$("$GENERATOR_SCRIPT" --source-index "$index_dir/source-index.json" --out "$out_dir/corpus.json")"
  corpus_path="$(extract_labeled_path "Corpus JSON" <<<"$stdout_text")"
  if [[ "$(jq -r '.items[0].source_kind' "$corpus_path")" != "declared_class" || "$(jq -r '.items[0] | has("contract_address")' "$corpus_path")" != "false" ]]; then
    echo "declared class corpus row should not require or synthesize contract_address" >&2
    cat "$corpus_path" >&2
    return 1
  fi
}

test_rejects_empty_declared_class_contract_address() {
  local index_dir="$TEST_TMP_DIR/declared-class-empty/index"
  local case_root="$index_dir/cases"
  mkdir -p "$index_dir"
  write_manifest_case "$case_root" "a"
  local item
  item="$(source_item_json class_only "cases/a/Scarb.toml" "0xclass" "2.14.0")"
  item="$(jq '.source_kind = "declared_class" | .contract_address = ""' <<<"$item")"
  write_source_index_file "$index_dir/source-index.json" sample class_hash "$item"
  expect_generator_failure "$index_dir/source-index.json" "source_index.items[0].contract_address must be a non-empty string"
}

test_normalizes_legacy_missing_source_kind_as_deployed_contract() {
  local index_dir="$TEST_TMP_DIR/legacy-source-kind/index"
  local case_root="$index_dir/cases"
  local out_dir="$TEST_TMP_DIR/legacy-source-kind/out"
  mkdir -p "$index_dir" "$out_dir"
  write_manifest_case "$case_root" "a"
  local item stdout_text corpus_path
  item="$(source_item_json legacy "cases/a/Scarb.toml" "0xlegacy" "2.14.0")"
  item="$(jq 'del(.source_kind)' <<<"$item")"
  write_source_index_file "$index_dir/source-index.json" sample class_hash "$item"

  stdout_text="$("$GENERATOR_SCRIPT" --source-index "$index_dir/source-index.json" --out "$out_dir/corpus.json")"
  corpus_path="$(extract_labeled_path "Corpus JSON" <<<"$stdout_text")"
  if [[ "$(jq -r '.items[0].source_kind' "$corpus_path")" != "deployed_contract" ]]; then
    echo "legacy missing source_kind should normalize to deployed_contract" >&2
    cat "$corpus_path" >&2
    return 1
  fi
}

test_complete_coverage_requires_deployed_contract_rows() {
  local index_dir="$TEST_TMP_DIR/complete-source-kind/index"
  local case_root="$index_dir/cases"
  mkdir -p "$index_dir"
  write_manifest_case "$case_root" "a"
  local item
  item="$(source_item_json class_only "cases/a/Scarb.toml" "0xclass" "2.14.0")"
  item="$(jq '.source_kind = "declared_class" | del(.contract_address)' <<<"$item")"
  write_source_index_file "$index_dir/declared.json" complete_deployed_contracts class_hash "$item"
  expect_generator_failure "$index_dir/declared.json" "source_index.items[0].source_kind must be explicitly deployed_contract when source_index.selection.coverage is complete_deployed_contracts"

  item="$(source_item_json missing "cases/a/Scarb.toml" "0xclass" "2.14.0")"
  item="$(jq 'del(.source_kind)' <<<"$item")"
  write_source_index_file "$index_dir/missing.json" complete_deployed_contracts class_hash "$item"
  expect_generator_failure "$index_dir/missing.json" "source_index.items[0].source_kind must be explicitly deployed_contract when source_index.selection.coverage is complete_deployed_contracts"
}

test_rejects_missing_manifest_path() {
  local index_dir="$TEST_TMP_DIR/missing-manifest/index"
  mkdir -p "$index_dir"
  local item
  item="$(source_item_json missing "cases/missing/Scarb.toml" "0x01" "2.14.0")"
  write_source_index_file "$index_dir/source-index.json" sample class_hash "$item"
  expect_generator_failure "$index_dir/source-index.json" "manifest_path does not exist for missing"
}

test_rejects_absolute_manifest_path() {
  local index_dir="$TEST_TMP_DIR/absolute-manifest/index"
  local case_root="$index_dir/cases"
  mkdir -p "$index_dir"
  write_manifest_case "$case_root" "a"
  local item
  item="$(source_item_json absolute "$case_root/a/Scarb.toml" "0x01" "2.14.0")"
  write_source_index_file "$index_dir/source-index.json" sample class_hash "$item"
  expect_generator_failure "$index_dir/source-index.json" "manifest_path for absolute must be relative to the source index"
}

test_rejects_manifest_path_traversal() {
  local index_dir="$TEST_TMP_DIR/traversal/index"
  local escape_root="$TEST_TMP_DIR/traversal/outside"
  mkdir -p "$index_dir"
  write_manifest_case "$escape_root" "a"
  local item
  item="$(source_item_json escape "../outside/a/Scarb.toml" "0x01" "2.14.0")"
  write_source_index_file "$index_dir/source-index.json" sample class_hash "$item"
  expect_generator_failure "$index_dir/source-index.json" "manifest_path for escape must stay under the source index directory"
}

test_rejects_manifest_not_named_scarb_toml() {
  local index_dir="$TEST_TMP_DIR/wrong-name/index"
  local case_root="$index_dir/cases"
  mkdir -p "$index_dir" "$case_root/a"
  cat > "$case_root/a/NotScarb.toml" <<'MANIFEST'
[package]
name = "wrong_name"
version = "0.1.0"
edition = "2024_07"
MANIFEST
  local item
  item="$(source_item_json wrong_name "cases/a/NotScarb.toml" "0x01" "2.14.0")"
  write_source_index_file "$index_dir/source-index.json" sample class_hash "$item"
  expect_generator_failure "$index_dir/source-index.json" "manifest_path for wrong_name must point to Scarb.toml"
}

test_rejects_deduped_count_mismatch() {
  local index_dir="$TEST_TMP_DIR/dedup-mismatch/index"
  local case_root="$index_dir/cases"
  mkdir -p "$index_dir"
  write_manifest_case "$case_root" "a"
  local item
  item="$(source_item_json a "cases/a/Scarb.toml" "0x01" "2.14.0")"
  write_source_index_file "$index_dir/source-index.json" sample class_hash "$item"
  mutate_source_index "$index_dir/source-index.json" '.deduplication.deduped_count = 2'
  expect_generator_failure "$index_dir/source-index.json" "source_index.deduplication.deduped_count (2) must equal items length (1)"
}

test_rejects_input_count_less_than_deduped() {
  local index_dir="$TEST_TMP_DIR/input-lt-deduped/index"
  local case_root="$index_dir/cases"
  mkdir -p "$index_dir"
  write_manifest_case "$case_root" "a"
  write_manifest_case "$case_root" "b"
  local item_a item_b
  item_a="$(source_item_json a "cases/a/Scarb.toml" "0x01" "2.14.0")"
  item_b="$(source_item_json b "cases/b/Scarb.toml" "0x02" "2.14.0")"
  write_source_index_file "$index_dir/source-index.json" sample class_hash "$item_a" "$item_b"
  mutate_source_index "$index_dir/source-index.json" '.deduplication.input_count = 1'
  expect_generator_failure "$index_dir/source-index.json" "source_index.deduplication.input_count must be >= deduped_count"
}

test_rejects_none_dedupe_count_mismatch() {
  local index_dir="$TEST_TMP_DIR/none-count-mismatch/index"
  local case_root="$index_dir/cases"
  mkdir -p "$index_dir"
  write_manifest_case "$case_root" "a"
  local item
  item="$(source_item_json a "cases/a/Scarb.toml" "0x01" "2.14.0")"
  write_source_index_file "$index_dir/source-index.json" complete_deployed_contracts none "$item"
  mutate_source_index "$index_dir/source-index.json" '.deduplication.input_count = 2'
  expect_generator_failure "$index_dir/source-index.json" "source_index.deduplication.input_count must equal deduped_count when deduplication.key is none"
}

test_rejects_source_index_output_overwrite() {
  local index_dir="$TEST_TMP_DIR/overwrite/index"
  local case_root="$index_dir/cases"
  mkdir -p "$index_dir"
  write_manifest_case "$case_root" "a"
  local item source_index_path
  item="$(source_item_json a "cases/a/Scarb.toml" "0x01" "2.14.0")"
  source_index_path="$index_dir/source-index.json"
  write_source_index_file "$source_index_path" sample class_hash "$item"
  expect_generator_failure "$source_index_path" "Refusing to overwrite source index with generated corpus" "$source_index_path"
}

test_rejects_source_index_output_symlink_overwrite() {
  local index_dir="$TEST_TMP_DIR/overwrite-symlink/index"
  local case_root="$index_dir/cases"
  local out_dir="$TEST_TMP_DIR/overwrite-symlink/out"
  mkdir -p "$index_dir" "$out_dir"
  write_manifest_case "$case_root" "a"
  local item source_index_path out_path stderr_path exit_code
  item="$(source_item_json a "cases/a/Scarb.toml" "0x01" "2.14.0")"
  source_index_path="$index_dir/source-index.json"
  out_path="$out_dir/source-index-alias.json"
  write_source_index_file "$source_index_path" sample class_hash "$item"
  ln -s "$source_index_path" "$out_path"

  stderr_path="$TEST_TMP_DIR/overwrite-symlink.err"
  set +e
  "$GENERATOR_SCRIPT" --source-index "$source_index_path" --out "$out_path" >"$TEST_TMP_DIR/overwrite-symlink.out" 2>"$stderr_path"
  exit_code=$?
  set -e
  if [[ "$exit_code" -ne 2 ]]; then
    echo "expected symlink overwrite to exit with code 2, got: $exit_code" >&2
    cat "$TEST_TMP_DIR/overwrite-symlink.out" >&2
    cat "$stderr_path" >&2
    return 1
  fi
  if ! grep -Fq -- "Refusing to overwrite source index with generated corpus" "$stderr_path"; then
    echo "expected symlink overwrite refusal" >&2
    cat "$stderr_path" >&2
    return 1
  fi
}

test_rejects_output_directory() {
  local index_dir="$TEST_TMP_DIR/out-dir/index"
  local case_root="$index_dir/cases"
  local out_dir="$TEST_TMP_DIR/out-dir/generated"
  mkdir -p "$index_dir" "$out_dir"
  write_manifest_case "$case_root" "a"
  local item source_index_path
  item="$(source_item_json a "cases/a/Scarb.toml" "0x01" "2.14.0")"
  source_index_path="$index_dir/source-index.json"
  write_source_index_file "$source_index_path" sample class_hash "$item"
  expect_generator_failure "$source_index_path" "--out must be a file path, got directory" "$out_dir"
}

run_test "generates_corpus_and_plan_only_accepts_it" \
  test_generates_corpus_and_plan_only_accepts_it
run_test "rejects_unknown_top_level_keys" \
  test_rejects_unknown_top_level_keys
run_test "rejects_boolean_integer_fields" \
  test_rejects_boolean_integer_fields
run_test "rejects_non_string_optional_fields" \
  test_rejects_non_string_optional_fields
run_test "rejects_duplicate_tags" \
  test_rejects_duplicate_tags
run_test "rejects_duplicate_class_hash_when_class_deduped" \
  test_rejects_duplicate_class_hash_when_class_deduped
run_test "generates_declared_class_without_contract_address" \
  test_generates_declared_class_without_contract_address
run_test "rejects_empty_declared_class_contract_address" \
  test_rejects_empty_declared_class_contract_address
run_test "normalizes_legacy_missing_source_kind_as_deployed_contract" \
  test_normalizes_legacy_missing_source_kind_as_deployed_contract
run_test "complete_coverage_requires_deployed_contract_rows" \
  test_complete_coverage_requires_deployed_contract_rows
run_test "rejects_missing_manifest_path" \
  test_rejects_missing_manifest_path
run_test "rejects_absolute_manifest_path" \
  test_rejects_absolute_manifest_path
run_test "rejects_manifest_path_traversal" \
  test_rejects_manifest_path_traversal
run_test "rejects_manifest_not_named_scarb_toml" \
  test_rejects_manifest_not_named_scarb_toml
run_test "rejects_deduped_count_mismatch" \
  test_rejects_deduped_count_mismatch
run_test "rejects_input_count_less_than_deduped" \
  test_rejects_input_count_less_than_deduped
run_test "rejects_none_dedupe_count_mismatch" \
  test_rejects_none_dedupe_count_mismatch
run_test "rejects_source_index_output_overwrite" \
  test_rejects_source_index_output_overwrite
run_test "rejects_source_index_output_symlink_overwrite" \
  test_rejects_source_index_output_symlink_overwrite
run_test "rejects_output_directory" \
  test_rejects_output_directory

#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
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

write_mock_uc_bin() {
  local path="$1"
  cat > "$path" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail

args_log="${MOCK_UC_ARGS_LOG:?}"
if [[ "$1" == "support" && "${2-}" == "native" ]]; then
  manifest=""
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --manifest-path)
        manifest="${2-}"
        shift 2
        ;;
      --format)
        shift 2
        ;;
      *)
        shift
        ;;
    esac
  done
  printf 'support %s\n' "$manifest" >> "$args_log"
  if [[ "$manifest" == *"unsupported"* ]]; then
    printf '{"manifest_path":"%s","status":"unsupported","supported":false,"reason":"native cairo-lang 2.16.0 is incompatible with package cairo-version 2.14.0","compiler_version":"2.16.0","package_cairo_version":"2.14.0","issue_kind":"compiler_version_mismatch"}\n' "$manifest"
  else
    printf '{"manifest_path":"%s","status":"supported","supported":true,"compiler_version":"2.16.0","package_cairo_version":"2.16.0"}\n' "$manifest"
  fi
  exit 0
fi

if [[ "$1" == "build" ]]; then
  manifest=""
  report_path=""
  seen_offline=0
  seen_daemon_off=0
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --manifest-path)
        manifest="${2-}"
        shift 2
        ;;
      --report-path)
        report_path="${2-}"
        shift 2
        ;;
      --daemon-mode)
        if [[ "${2-}" != "off" ]]; then
          echo "expected uc --daemon-mode off, got: ${2-}" >&2
          exit 22
        fi
        seen_daemon_off=1
        shift 2
        ;;
      --offline)
        seen_offline=1
        shift
        ;;
      *)
        shift
        ;;
    esac
  done
  if [[ "$seen_offline" -ne 1 ]]; then
    echo "missing uc --offline" >&2
    exit 23
  fi
  if [[ "$seen_daemon_off" -ne 1 ]]; then
    echo "missing uc --daemon-mode off" >&2
    exit 24
  fi
  printf 'build %s disallow=%s report=%s\n' "$manifest" "${UC_NATIVE_DISALLOW_SCARB_FALLBACK:-}" "$report_path" >> "$args_log"
  compile_backend="uc_native"
  diagnostics="[]"
  if [[ "$manifest" == *"fallback-used"* ]]; then
    compile_backend="scarb_fallback"
    diagnostics='[{"code":"UCN2002","category":"native_fallback_local_native_error","severity":"warn","title":"Native local build downgraded to Scarb","what_happened":"native failed","why":"native failed","how_to_fix":["fix native"],"retryable":true,"fallback_used":true,"toolchain_expected":"2.16.0","toolchain_found":"2.16.0"}]'
  fi
  if [[ -n "$report_path" ]]; then
    mkdir -p "$(dirname "$report_path")"
    cat > "$report_path" <<REPORT
{
  "generated_at_epoch_ms": 1,
  "engine": "uc",
  "daemon_used": false,
  "manifest_path": "$manifest",
  "workspace_root": "$(dirname "$manifest")",
  "profile": "dev",
  "session_key": "session-$manifest",
  "command": ["uc", "build"],
  "exit_code": 0,
  "elapsed_ms": 1.0,
  "cache_hit": false,
  "fingerprint": "fp-$manifest",
  "artifact_count": 1,
  "phase_telemetry": null,
  "compile_backend": "$compile_backend",
  "native_toolchain": {
    "requested_version": "2.16.0",
    "requested_major_minor": "2.16",
    "request_source": "package_cairo_version",
    "source": "builtin",
    "compiler_version": "2.16.0",
    "helper_path": null,
    "helper_env": null
  },
  "diagnostics": $diagnostics
}
REPORT
  fi
  exit 0
fi

echo "unexpected uc invocation: $*" >&2
exit 1
MOCK
  chmod +x "$path"
}

write_mock_scarb_bin() {
  local path="$1"
  cat > "$path" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail

args_log="${MOCK_SCARB_ARGS_LOG:?}"
manifest=""
subcommand=""
seen_offline=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --manifest-path)
      manifest="${2-}"
      shift 2
      ;;
    build|fetch)
      subcommand="$1"
      shift
      ;;
    --offline)
      seen_offline=1
      shift
      ;;
    *)
      echo "unexpected scarb invocation: $*" >&2
      exit 19
      ;;
  esac
done
if [[ -z "$subcommand" ]]; then
  echo "missing scarb subcommand" >&2
  exit 20
fi
if [[ "$seen_offline" -ne 1 ]]; then
  echo "missing scarb --offline" >&2
  exit 21
fi
if [[ -z "$manifest" && "$subcommand" == "fetch" ]]; then
  manifest="$PWD/Scarb.toml"
fi
if [[ -z "$manifest" ]]; then
  echo "missing scarb manifest path" >&2
  exit 20
fi
printf 'cwd=%s subcommand=%s manifest=%s\n' "$PWD" "$subcommand" "$manifest" >> "$args_log"
exit 0
MOCK
  chmod +x "$path"
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
MANIFEST
  cat > "$root/$name/src/lib.cairo" <<'SRC'
fn main() -> felt252 {
    1
}
SRC
}

write_corpus_file() {
  local path="$1"
  local coverage="$2"
  local dedupe_key="$3"
  shift 3
  local items_json
  items_json="[$(IFS=,; echo "$*")]"
  cat > "$path" <<JSON
{
  "schema_version": 1,
  "corpus_id": "test-corpus-$coverage",
  "chain": "starknet-mainnet",
  "selection": {
    "source": "local test fixture",
    "snapshot_id": "test-snapshot",
    "from_block": 1,
    "to_block": 2,
    "coverage": "$coverage"
  },
  "deduplication": {
    "key": "$dedupe_key",
    "input_count": $(jq 'length' <<<"$items_json"),
    "deduped_count": $(jq 'length' <<<"$items_json"),
    "rules": "test rows"
  },
  "license_policy": "test fixtures only",
  "items": $items_json
}
JSON
}

item_json() {
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
      license: "fixture"
    }'
}

test_plan_only_normalizes_sample_corpus() {
  local case_root="$TEST_TMP_DIR/plan/cases"
  local corpus_dir="$TEST_TMP_DIR/plan/corpora"
  local results_dir="$TEST_TMP_DIR/plan/results"
  mkdir -p "$corpus_dir" "$results_dir"
  write_manifest_case "$case_root" "sample"
  local item
  item="$(item_json sample "../cases/sample/Scarb.toml" "0x01" "2.16.0")"
  write_corpus_file "$corpus_dir/corpus.json" sample class_hash "$item"

  local stdout_text
  stdout_text="$(
    "$CORPUS_SCRIPT" \
      --corpus "$corpus_dir/corpus.json" \
      --results-dir "$results_dir" \
      --runs 3 \
      --cold-runs 2 \
      --timeout-secs 9 \
      --warm-settle-seconds 0.5 \
      --plan-only
  )"
  assert_contains "$stdout_text" "Corpus plan JSON:"

  local json_path
  json_path="$(extract_labeled_path "Corpus plan JSON" <<<"$stdout_text")"
  [[ -f "$json_path" ]] || { echo "missing plan json: $json_path" >&2; return 1; }

  local plan_only item_count coverage manifest_path runs cold_runs timeout_secs warm_settle_seconds
  plan_only="$(jq -r '.plan_only' "$json_path")"
  item_count="$(jq -r '.corpus.summary.item_count' "$json_path")"
  coverage="$(jq -r '.corpus.selection.coverage' "$json_path")"
  manifest_path="$(jq -r '.corpus.items[0].manifest_path' "$json_path")"
  runs="$(jq -r '.run_config.runs' "$json_path")"
  cold_runs="$(jq -r '.run_config.cold_runs' "$json_path")"
  timeout_secs="$(jq -r '.run_config.timeout_secs' "$json_path")"
  warm_settle_seconds="$(jq -r '.run_config.warm_settle_seconds' "$json_path")"
  if [[ "$plan_only" != "true" || "$item_count" != "1" || "$coverage" != "sample" || "$manifest_path" != /* || "$runs" != "3" || "$cold_runs" != "2" || "$timeout_secs" != "9" || "$warm_settle_seconds" != "0.5" ]]; then
    echo "unexpected plan-only corpus artifact" >&2
    cat "$json_path" >&2
    return 1
  fi
}

test_rejects_duplicate_tags() {
  local case_root="$TEST_TMP_DIR/dupe-tag/cases"
  local corpus_dir="$TEST_TMP_DIR/dupe-tag/corpora"
  local results_dir="$TEST_TMP_DIR/dupe-tag/results"
  mkdir -p "$corpus_dir" "$results_dir"
  write_manifest_case "$case_root" "a"
  write_manifest_case "$case_root" "b"
  local item_a item_b
  item_a="$(item_json same-tag "../cases/a/Scarb.toml" "0x01" "2.14.0")"
  item_b="$(item_json same-tag "../cases/b/Scarb.toml" "0x02" "2.16.0")"
  write_corpus_file "$corpus_dir/corpus.json" sample none "$item_a" "$item_b"

  local stderr_path="$TEST_TMP_DIR/dupe-tag.err"
  if "$CORPUS_SCRIPT" --corpus "$corpus_dir/corpus.json" --results-dir "$results_dir" --plan-only >"$TEST_TMP_DIR/dupe-tag.out" 2>"$stderr_path"; then
    echo "expected duplicate tags to be rejected" >&2
    return 1
  fi
  if ! grep -q "duplicate corpus item tag" "$stderr_path"; then
    echo "expected duplicate tag error" >&2
    cat "$stderr_path" >&2
    return 1
  fi
}

test_rejects_duplicate_class_hash_when_class_deduped() {
  local case_root="$TEST_TMP_DIR/dupe-class/cases"
  local corpus_dir="$TEST_TMP_DIR/dupe-class/corpora"
  local results_dir="$TEST_TMP_DIR/dupe-class/results"
  mkdir -p "$corpus_dir" "$results_dir"
  write_manifest_case "$case_root" "a"
  write_manifest_case "$case_root" "b"
  local item_a item_b
  item_a="$(item_json a "../cases/a/Scarb.toml" "0xsame" "2.14.0")"
  item_b="$(item_json b "../cases/b/Scarb.toml" "0xsame" "2.16.0")"
  write_corpus_file "$corpus_dir/corpus.json" sample class_hash "$item_a" "$item_b"

  local stderr_path="$TEST_TMP_DIR/dupe-class.err"
  if "$CORPUS_SCRIPT" --corpus "$corpus_dir/corpus.json" --results-dir "$results_dir" --plan-only >"$TEST_TMP_DIR/dupe-class.out" 2>"$stderr_path"; then
    echo "expected duplicate class hash to be rejected" >&2
    return 1
  fi
  if ! grep -q "duplicate class_hash" "$stderr_path"; then
    echo "expected duplicate class_hash error" >&2
    cat "$stderr_path" >&2
    return 1
  fi
}

test_rejects_unknown_top_level_keys() {
  local case_root="$TEST_TMP_DIR/unknown-key/cases"
  local corpus_dir="$TEST_TMP_DIR/unknown-key/corpora"
  local results_dir="$TEST_TMP_DIR/unknown-key/results"
  mkdir -p "$corpus_dir" "$results_dir"
  write_manifest_case "$case_root" "a"
  local item
  item="$(item_json a "../cases/a/Scarb.toml" "0x01" "2.14.0")"
  write_corpus_file "$corpus_dir/corpus.json" sample class_hash "$item"
  jq '.unexpected = true' "$corpus_dir/corpus.json" > "$corpus_dir/corpus.tmp"
  mv "$corpus_dir/corpus.tmp" "$corpus_dir/corpus.json"

  local stderr_path="$TEST_TMP_DIR/unknown-key.err"
  if "$CORPUS_SCRIPT" --corpus "$corpus_dir/corpus.json" --results-dir "$results_dir" --plan-only >"$TEST_TMP_DIR/unknown-key.out" 2>"$stderr_path"; then
    echo "expected unknown top-level keys to be rejected" >&2
    return 1
  fi
  if ! grep -q "corpus has unsupported field(s): unexpected" "$stderr_path"; then
    echo "expected unknown-key validation error" >&2
    cat "$stderr_path" >&2
    return 1
  fi
}

test_rejects_boolean_integer_fields() {
  local case_root="$TEST_TMP_DIR/bool-int/cases"
  local corpus_dir="$TEST_TMP_DIR/bool-int/corpora"
  local results_dir="$TEST_TMP_DIR/bool-int/results"
  mkdir -p "$corpus_dir" "$results_dir"
  write_manifest_case "$case_root" "a"
  local item
  item="$(item_json a "../cases/a/Scarb.toml" "0x01" "2.14.0")"
  write_corpus_file "$corpus_dir/corpus.json" sample class_hash "$item"
  jq '.selection.from_block = true' "$corpus_dir/corpus.json" > "$corpus_dir/corpus.tmp"
  mv "$corpus_dir/corpus.tmp" "$corpus_dir/corpus.json"

  local stderr_path="$TEST_TMP_DIR/bool-int.err"
  if "$CORPUS_SCRIPT" --corpus "$corpus_dir/corpus.json" --results-dir "$results_dir" --plan-only >"$TEST_TMP_DIR/bool-int.out" 2>"$stderr_path"; then
    echo "expected boolean integer fields to be rejected" >&2
    return 1
  fi
  if ! grep -q "corpus.selection.from_block must be a non-negative integer" "$stderr_path"; then
    echo "expected boolean integer validation error" >&2
    cat "$stderr_path" >&2
    return 1
  fi
}

test_rejects_non_string_optional_fields() {
  local case_root="$TEST_TMP_DIR/optional-strings/cases"
  local corpus_dir="$TEST_TMP_DIR/optional-strings/corpora"
  local results_dir="$TEST_TMP_DIR/optional-strings/results"
  mkdir -p "$corpus_dir" "$results_dir"
  write_manifest_case "$case_root" "a"
  local item
  item="$(item_json a "../cases/a/Scarb.toml" "0x01" "2.14.0")"

  local -a filters=(
    '.license_policy = false'
    '.selection.notes = {}'
    '.deduplication.rules = []'
    '.items[0].scarb_version = false'
    '.items[0].license = {}'
    '.items[0].notes = []'
  )
  local -a contexts=(
    'corpus.license_policy must be a string'
    'corpus.selection.notes must be a string'
    'corpus.deduplication.rules must be a string'
    'corpus.items[0].scarb_version must be a string'
    'corpus.items[0].license must be a string'
    'corpus.items[0].notes must be a string'
  )

  local index corpus_path stderr_path
  for index in "${!filters[@]}"; do
    corpus_path="$corpus_dir/corpus-$index.json"
    write_corpus_file "$corpus_path" sample class_hash "$item"
    jq "${filters[$index]}" "$corpus_path" > "$corpus_dir/corpus-$index.tmp"
    mv "$corpus_dir/corpus-$index.tmp" "$corpus_path"
    stderr_path="$TEST_TMP_DIR/optional-strings-$index.err"
    if "$CORPUS_SCRIPT" --corpus "$corpus_path" --results-dir "$results_dir/$index" --plan-only >"$TEST_TMP_DIR/optional-strings-$index.out" 2>"$stderr_path"; then
      echo "expected non-string optional field to be rejected for ${filters[$index]}" >&2
      return 1
    fi
    if ! grep -Fq "${contexts[$index]}" "$stderr_path"; then
      echo "expected optional string validation error for ${filters[$index]}" >&2
      cat "$stderr_path" >&2
      return 1
    fi
  done
}

run_corpus_benchmark() {
  local coverage="$1"
  local dedupe_key="${2:-class_hash}"
  local run_id="$coverage-$dedupe_key"
  local corpus_dir="$TEST_TMP_DIR/run-$run_id/corpora"
  local case_root="$TEST_TMP_DIR/run-$run_id/cases"
  local results_dir="$TEST_TMP_DIR/run-$run_id/results: dir"
  local mock_bin_dir="$TEST_TMP_DIR/run-$run_id/mock-bin"
  mkdir -p "$corpus_dir" "$case_root" "$results_dir" "$mock_bin_dir"
  write_manifest_case "$case_root" "cairo214"
  write_manifest_case "$case_root" "cairo216"
  write_mock_uc_bin "$mock_bin_dir/uc"
  write_mock_scarb_bin "$mock_bin_dir/scarb"

  local item_a item_b
  item_a="$(item_json cairo214 "../cases/cairo214/Scarb.toml" "0x214" "2.14.0")"
  item_b="$(item_json cairo216 "../cases/cairo216/Scarb.toml" "0x216" "2.16.0")"
  write_corpus_file "$corpus_dir/corpus.json" "$coverage" "$dedupe_key" "$item_a" "$item_b"

  PATH="$mock_bin_dir:$PATH" \
  MOCK_UC_ARGS_LOG="$TEST_TMP_DIR/run-$run_id/uc.args" \
  MOCK_SCARB_ARGS_LOG="$TEST_TMP_DIR/run-$run_id/scarb.args" \
  "$CORPUS_SCRIPT" \
    --uc-bin "$mock_bin_dir/uc" \
    --results-dir "$results_dir" \
    --runs 1 \
    --cold-runs 1 \
    --warm-settle-seconds 0 \
    --corpus "$corpus_dir/corpus.json"
}

test_complete_corpus_with_fallback_blocks_compiled_all_claim() {
  local corpus_dir="$TEST_TMP_DIR/fallback/corpora"
  local case_root="$TEST_TMP_DIR/fallback/cases"
  local results_dir="$TEST_TMP_DIR/fallback/results"
  local mock_bin_dir="$TEST_TMP_DIR/fallback/mock-bin"
  mkdir -p "$corpus_dir" "$case_root" "$results_dir" "$mock_bin_dir"
  write_manifest_case "$case_root" "cairo214"
  write_manifest_case "$case_root" "fallback-used"
  write_mock_uc_bin "$mock_bin_dir/uc"
  write_mock_scarb_bin "$mock_bin_dir/scarb"

  local item_a item_b
  item_a="$(item_json cairo214 "../cases/cairo214/Scarb.toml" "0x214" "2.14.0")"
  item_b="$(item_json fallback-used "../cases/fallback-used/Scarb.toml" "0xfallback" "2.16.0")"
  write_corpus_file "$corpus_dir/corpus.json" complete_deployed_contracts class_hash "$item_a" "$item_b"

  local stdout_text
  stdout_text="$(
    PATH="$mock_bin_dir:$PATH" \
    MOCK_UC_ARGS_LOG="$TEST_TMP_DIR/fallback/uc.args" \
    MOCK_SCARB_ARGS_LOG="$TEST_TMP_DIR/fallback/scarb.args" \
    "$CORPUS_SCRIPT" \
      --uc-bin "$mock_bin_dir/uc" \
      --results-dir "$results_dir" \
      --runs 1 \
      --cold-runs 1 \
      --warm-settle-seconds 0 \
      --corpus "$corpus_dir/corpus.json"
  )"

  local json_path safe reason fallback_used claim
  json_path="$(extract_labeled_path "Corpus Benchmark JSON" <<<"$stdout_text")"
  [[ -f "$json_path" ]] || { echo "missing corpus benchmark json: $json_path" >&2; return 1; }
  safe="$(jq -r '.claim_guard.safe_to_say_compiled_all_deployed_contracts_in_corpus' "$json_path")"
  reason="$(jq -r '.claim_guard.reason' "$json_path")"
  fallback_used="$(jq -r '.summary.support_matrix.fallback_used' "$json_path")"
  claim="$(jq -r '.claim_guard.compiled_all_claim_text // ""' "$json_path")"
  if [[ "$safe" != "false" || "$reason" != *"fallback"* || "$fallback_used" != "1" || -n "$claim" ]]; then
    echo "fallback-used complete corpus should not emit compiled-all launch claim" >&2
    cat "$json_path" >&2
    return 1
  fi
}

test_complete_corpus_with_unsupported_blocks_compiled_all_claim() {
  local corpus_dir="$TEST_TMP_DIR/native-block/corpora"
  local case_root="$TEST_TMP_DIR/native-block/cases"
  local results_dir="$TEST_TMP_DIR/native-block/results"
  local mock_bin_dir="$TEST_TMP_DIR/native-block/mock-bin"
  mkdir -p "$corpus_dir" "$case_root" "$results_dir" "$mock_bin_dir"
  write_manifest_case "$case_root" "cairo214"
  write_manifest_case "$case_root" "unsupported"
  write_mock_uc_bin "$mock_bin_dir/uc"
  write_mock_scarb_bin "$mock_bin_dir/scarb"

  local item_a item_b
  item_a="$(item_json cairo214 "../cases/cairo214/Scarb.toml" "0x214" "2.14.0")"
  item_b="$(item_json unsupported "../cases/unsupported/Scarb.toml" "0xunsupported" "2.14.0")"
  write_corpus_file "$corpus_dir/corpus.json" complete_deployed_contracts class_hash "$item_a" "$item_b"

  local stdout_text
  stdout_text="$(
    PATH="$mock_bin_dir:$PATH" \
    MOCK_UC_ARGS_LOG="$TEST_TMP_DIR/native-block/uc.args" \
    MOCK_SCARB_ARGS_LOG="$TEST_TMP_DIR/native-block/scarb.args" \
    "$CORPUS_SCRIPT" \
      --uc-bin "$mock_bin_dir/uc" \
      --results-dir "$results_dir" \
      --runs 1 \
      --cold-runs 1 \
      --warm-settle-seconds 0 \
      --corpus "$corpus_dir/corpus.json"
  )"

  local json_path safe reason native_unsupported claim
  json_path="$(extract_labeled_path "Corpus Benchmark JSON" <<<"$stdout_text")"
  [[ -f "$json_path" ]] || { echo "missing corpus benchmark json: $json_path" >&2; return 1; }
  safe="$(jq -r '.claim_guard.safe_to_say_compiled_all_deployed_contracts_in_corpus' "$json_path")"
  reason="$(jq -r '.claim_guard.reason' "$json_path")"
  native_unsupported="$(jq -r '.summary.support_matrix.native_unsupported' "$json_path")"
  claim="$(jq -r '.claim_guard.compiled_all_claim_text // ""' "$json_path")"
  if [[ "$safe" != "false" || "$reason" != *"native_unsupported"* || "$native_unsupported" != "1" || -n "$claim" ]]; then
    echo "native-unsupported complete corpus should not emit compiled-all launch claim" >&2
    cat "$json_path" >&2
    return 1
  fi
}

test_sample_corpus_blocks_compiled_all_claim() {
  local stdout_text
  stdout_text="$(run_corpus_benchmark sample)"
  assert_contains "$stdout_text" "Corpus Benchmark JSON:"
  assert_contains "$stdout_text" "Corpus Benchmark Markdown:"

  local json_path
  json_path="$(extract_labeled_path "Corpus Benchmark JSON" <<<"$stdout_text")"
  [[ -f "$json_path" ]] || { echo "missing corpus benchmark json: $json_path" >&2; return 1; }

  local safe reason native_supported claim
  safe="$(jq -r '.claim_guard.safe_to_say_compiled_all_deployed_contracts_in_corpus' "$json_path")"
  reason="$(jq -r '.claim_guard.reason' "$json_path")"
  native_supported="$(jq -r '.summary.support_matrix.native_supported' "$json_path")"
  claim="$(jq -r '.claim_guard.compiled_all_claim_text // ""' "$json_path")"
  if [[ "$safe" != "false" || "$reason" != *"coverage is sample"* || "$native_supported" != "2" || -n "$claim" ]]; then
    echo "sample corpus should not emit launch claim" >&2
    cat "$json_path" >&2
    return 1
  fi
}

test_complete_class_deduped_corpus_emits_selected_unit_claim_only() {
  local stdout_text
  stdout_text="$(run_corpus_benchmark complete_deployed_contracts)"
  local json_path md_path
  json_path="$(extract_labeled_path "Corpus Benchmark JSON" <<<"$stdout_text")"
  md_path="$(extract_labeled_path "Corpus Benchmark Markdown" <<<"$stdout_text")"
  [[ -f "$json_path" ]] || { echo "missing corpus benchmark json: $json_path" >&2; return 1; }
  [[ -f "$md_path" ]] || { echo "missing corpus benchmark markdown: $md_path" >&2; return 1; }

  local safe selected_safe all_supported claim selected_claim reason markdown_text
  safe="$(jq -r '.claim_guard.safe_to_say_compiled_all_deployed_contracts_in_corpus' "$json_path")"
  selected_safe="$(jq -r '.claim_guard.safe_to_say_compiled_all_selected_deployed_units_in_corpus' "$json_path")"
  all_supported="$(jq -r '.claim_guard.safe_to_say_all_items_native_supported' "$json_path")"
  claim="$(jq -r '.claim_guard.compiled_all_claim_text // ""' "$json_path")"
  selected_claim="$(jq -r '.claim_guard.selected_units_claim_text // ""' "$json_path")"
  reason="$(jq -r '.claim_guard.reason' "$json_path")"
  markdown_text="$(cat "$md_path")"
  if [[ "$safe" != "false" || "$selected_safe" != "true" || "$all_supported" != "true" || -n "$claim" ]]; then
    echo "class-deduped complete corpus should not emit deployed-address claim" >&2
    cat "$json_path" >&2
    return 1
  fi
  assert_contains "$reason" "deduplicated by class_hash"
  assert_contains "$selected_claim" "after class_hash deduplication"
  assert_contains "$selected_claim" "Cairo 2.14.0 through 2.16.0"
  assert_contains "$markdown_text" "Compiled-all claim: <not safe for this artifact>"
  assert_contains "$markdown_text" "Selected-unit claim: We compiled every selected deployed unit"
}

test_complete_non_deduped_corpus_emits_deployed_contract_claim() {
  local stdout_text
  stdout_text="$(run_corpus_benchmark complete_deployed_contracts none)"
  local json_path md_path
  json_path="$(extract_labeled_path "Corpus Benchmark JSON" <<<"$stdout_text")"
  md_path="$(extract_labeled_path "Corpus Benchmark Markdown" <<<"$stdout_text")"
  [[ -f "$json_path" ]] || { echo "missing corpus benchmark json: $json_path" >&2; return 1; }
  [[ -f "$md_path" ]] || { echo "missing corpus benchmark markdown: $md_path" >&2; return 1; }

  local safe selected_safe all_supported claim selected_claim reason markdown_text
  safe="$(jq -r '.claim_guard.safe_to_say_compiled_all_deployed_contracts_in_corpus' "$json_path")"
  selected_safe="$(jq -r '.claim_guard.safe_to_say_compiled_all_selected_deployed_units_in_corpus' "$json_path")"
  all_supported="$(jq -r '.claim_guard.safe_to_say_all_items_native_supported' "$json_path")"
  claim="$(jq -r '.claim_guard.compiled_all_claim_text // ""' "$json_path")"
  selected_claim="$(jq -r '.claim_guard.selected_units_claim_text // ""' "$json_path")"
  reason="$(jq -r '.claim_guard.reason' "$json_path")"
  markdown_text="$(cat "$md_path")"
  if [[ "$safe" != "true" || "$selected_safe" != "true" || "$all_supported" != "true" ]]; then
    echo "non-deduped complete corpus should emit guarded deployed-contract claim" >&2
    cat "$json_path" >&2
    return 1
  fi
  assert_contains "$reason" "bounded to this pinned corpus artifact"
  assert_contains "$claim" "Cairo 2.14.0 through 2.16.0"
  assert_contains "$selected_claim" "after none deduplication"
  assert_contains "$markdown_text" "Compiled-all claim: We compiled every contract"
}

test_complete_corpus_with_declared_class_blocks_deployed_claim() {
  local corpus_dir="$TEST_TMP_DIR/declared-class/corpora"
  local case_root="$TEST_TMP_DIR/declared-class/cases"
  local results_dir="$TEST_TMP_DIR/declared-class/results"
  local mock_bin_dir="$TEST_TMP_DIR/declared-class/mock-bin"
  mkdir -p "$corpus_dir" "$case_root" "$results_dir" "$mock_bin_dir"
  write_manifest_case "$case_root" "class-only"
  write_mock_uc_bin "$mock_bin_dir/uc"
  write_mock_scarb_bin "$mock_bin_dir/scarb"

  local item stdout_text json_path md_path safe reason source_kind_count claim markdown_text
  item="$(item_json class-only "../cases/class-only/Scarb.toml" "0xclass" "2.14.0")"
  item="$(jq '.source_kind = "declared_class" | del(.contract_address)' <<<"$item")"
  write_corpus_file "$corpus_dir/corpus.json" complete_deployed_contracts class_hash "$item"

  stdout_text="$(
    PATH="$mock_bin_dir:$PATH" \
    MOCK_UC_ARGS_LOG="$TEST_TMP_DIR/declared-class/uc.args" \
    MOCK_SCARB_ARGS_LOG="$TEST_TMP_DIR/declared-class/scarb.args" \
    "$CORPUS_SCRIPT" \
      --uc-bin "$mock_bin_dir/uc" \
      --results-dir "$results_dir" \
      --runs 1 \
      --cold-runs 1 \
      --warm-settle-seconds 0 \
      --corpus "$corpus_dir/corpus.json"
  )"

  json_path="$(extract_labeled_path "Corpus Benchmark JSON" <<<"$stdout_text")"
  md_path="$(extract_labeled_path "Corpus Benchmark Markdown" <<<"$stdout_text")"
  safe="$(jq -r '.claim_guard.safe_to_say_compiled_all_deployed_contracts_in_corpus' "$json_path")"
  reason="$(jq -r '.claim_guard.reason' "$json_path")"
  source_kind_count="$(jq -r '.summary.source_kind_counts.declared_class' "$json_path")"
  claim="$(jq -r '.claim_guard.compiled_all_claim_text // ""' "$json_path")"
  markdown_text="$(cat "$md_path")"
  if [[ "$safe" != "false" || "$reason" != "corpus contains non-deployed source_kind rows" || "$source_kind_count" != "1" || -n "$claim" ]]; then
    echo "declared_class corpus item should block deployed-contract launch claim" >&2
    cat "$json_path" >&2
    return 1
  fi
  assert_contains "$markdown_text" "| class-only | declared_class | declared-class:0xclass |"
}

test_rejects_empty_declared_class_contract_address() {
  local case_root="$TEST_TMP_DIR/declared-class-empty/cases"
  local corpus_dir="$TEST_TMP_DIR/declared-class-empty/corpora"
  local results_dir="$TEST_TMP_DIR/declared-class-empty/results"
  mkdir -p "$corpus_dir" "$results_dir"
  write_manifest_case "$case_root" "class-only"
  local item stderr_path
  item="$(item_json class-only "../cases/class-only/Scarb.toml" "0xclass" "2.14.0")"
  item="$(jq '.source_kind = "declared_class" | .contract_address = ""' <<<"$item")"
  write_corpus_file "$corpus_dir/corpus.json" sample class_hash "$item"
  stderr_path="$TEST_TMP_DIR/declared-class-empty.err"
  if "$CORPUS_SCRIPT" --corpus "$corpus_dir/corpus.json" --results-dir "$results_dir" --plan-only >"$TEST_TMP_DIR/declared-class-empty.out" 2>"$stderr_path"; then
    echo "expected declared_class empty contract_address to be rejected" >&2
    return 1
  fi
  if ! grep -Fq "corpus.items[0].contract_address must be a non-empty string" "$stderr_path"; then
    echo "expected empty contract_address validation error" >&2
    cat "$stderr_path" >&2
    return 1
  fi
}

test_normalizes_legacy_missing_source_kind_as_deployed_contract() {
  local case_root="$TEST_TMP_DIR/legacy-source-kind/cases"
  local corpus_dir="$TEST_TMP_DIR/legacy-source-kind/corpora"
  local results_dir="$TEST_TMP_DIR/legacy-source-kind/results"
  mkdir -p "$corpus_dir" "$results_dir"
  write_manifest_case "$case_root" "legacy"
  local item stdout_text json_path
  item="$(item_json legacy "../cases/legacy/Scarb.toml" "0xlegacy" "2.14.0")"
  item="$(jq 'del(.source_kind)' <<<"$item")"
  write_corpus_file "$corpus_dir/corpus.json" sample class_hash "$item"

  stdout_text="$("$CORPUS_SCRIPT" --corpus "$corpus_dir/corpus.json" --results-dir "$results_dir" --plan-only)"
  json_path="$(extract_labeled_path "Corpus plan JSON" <<<"$stdout_text")"
  if [[ "$(jq -r '.corpus.items[0].source_kind' "$json_path")" != "deployed_contract" || "$(jq -r '.corpus.summary.source_kind_counts.deployed_contract' "$json_path")" != "1" ]]; then
    echo "legacy missing source_kind should normalize to deployed_contract" >&2
    cat "$json_path" >&2
    return 1
  fi
}

run_test "plan_only_normalizes_sample_corpus" \
  test_plan_only_normalizes_sample_corpus
run_test "rejects_duplicate_tags" \
  test_rejects_duplicate_tags
run_test "rejects_duplicate_class_hash_when_class_deduped" \
  test_rejects_duplicate_class_hash_when_class_deduped
run_test "rejects_unknown_top_level_keys" \
  test_rejects_unknown_top_level_keys
run_test "rejects_boolean_integer_fields" \
  test_rejects_boolean_integer_fields
run_test "rejects_non_string_optional_fields" \
  test_rejects_non_string_optional_fields
run_test "sample_corpus_blocks_compiled_all_claim" \
  test_sample_corpus_blocks_compiled_all_claim
run_test "complete_corpus_with_fallback_blocks_compiled_all_claim" \
  test_complete_corpus_with_fallback_blocks_compiled_all_claim
run_test "complete_corpus_with_unsupported_blocks_compiled_all_claim" \
  test_complete_corpus_with_unsupported_blocks_compiled_all_claim
run_test "complete_class_deduped_corpus_emits_selected_unit_claim_only" \
  test_complete_class_deduped_corpus_emits_selected_unit_claim_only
run_test "complete_non_deduped_corpus_emits_deployed_contract_claim" \
  test_complete_non_deduped_corpus_emits_deployed_contract_claim
run_test "complete_corpus_with_declared_class_blocks_deployed_claim" \
  test_complete_corpus_with_declared_class_blocks_deployed_claim
run_test "rejects_empty_declared_class_contract_address" \
  test_rejects_empty_declared_class_contract_address
run_test "normalizes_legacy_missing_source_kind_as_deployed_contract" \
  test_normalizes_legacy_missing_source_kind_as_deployed_contract

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
      *)
        shift
        ;;
    esac
  done
  printf 'build %s disallow=%s report=%s\n' "$manifest" "${UC_NATIVE_DISALLOW_SCARB_FALLBACK:-}" "$report_path" >> "$args_log"
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
  "compile_backend": "uc_native",
  "native_toolchain": {
    "requested_version": "2.16.0",
    "requested_major_minor": "2.16",
    "request_source": "package_cairo_version",
    "source": "builtin",
    "compiler_version": "2.16.0",
    "helper_path": null,
    "helper_env": null
  },
  "diagnostics": []
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
  local items_json="[$(IFS=,; echo "$*")]"
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
  stdout_text="$($CORPUS_SCRIPT --corpus "$corpus_dir/corpus.json" --results-dir "$results_dir" --plan-only)"
  assert_contains "$stdout_text" "Corpus plan JSON:"

  local json_path
  json_path="$(awk -F': ' '/Corpus plan JSON:/ {print $2}' <<<"$stdout_text")"
  [[ -f "$json_path" ]] || { echo "missing plan json: $json_path" >&2; return 1; }

  local plan_only item_count coverage manifest_path
  plan_only="$(jq -r '.plan_only' "$json_path")"
  item_count="$(jq -r '.corpus.summary.item_count' "$json_path")"
  coverage="$(jq -r '.corpus.selection.coverage' "$json_path")"
  manifest_path="$(jq -r '.corpus.items[0].manifest_path' "$json_path")"
  if [[ "$plan_only" != "true" || "$item_count" != "1" || "$coverage" != "sample" || "$manifest_path" != /* ]]; then
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

run_corpus_benchmark() {
  local coverage="$1"
  local corpus_dir="$TEST_TMP_DIR/run-$coverage/corpora"
  local case_root="$TEST_TMP_DIR/run-$coverage/cases"
  local results_dir="$TEST_TMP_DIR/run-$coverage/results"
  local mock_bin_dir="$TEST_TMP_DIR/run-$coverage/mock-bin"
  mkdir -p "$corpus_dir" "$case_root" "$results_dir" "$mock_bin_dir"
  write_manifest_case "$case_root" "cairo214"
  write_manifest_case "$case_root" "cairo216"
  write_mock_uc_bin "$mock_bin_dir/uc"
  write_mock_scarb_bin "$mock_bin_dir/scarb"

  local item_a item_b
  item_a="$(item_json cairo214 "../cases/cairo214/Scarb.toml" "0x214" "2.14.0")"
  item_b="$(item_json cairo216 "../cases/cairo216/Scarb.toml" "0x216" "2.16.0")"
  write_corpus_file "$corpus_dir/corpus.json" "$coverage" class_hash "$item_a" "$item_b"

  PATH="$mock_bin_dir:$PATH" \
  MOCK_UC_ARGS_LOG="$TEST_TMP_DIR/run-$coverage/uc.args" \
  MOCK_SCARB_ARGS_LOG="$TEST_TMP_DIR/run-$coverage/scarb.args" \
  "$CORPUS_SCRIPT" \
    --uc-bin "$mock_bin_dir/uc" \
    --results-dir "$results_dir" \
    --runs 1 \
    --cold-runs 1 \
    --warm-settle-seconds 0 \
    --corpus "$corpus_dir/corpus.json"
}

test_sample_corpus_blocks_compiled_all_claim() {
  local stdout_text
  stdout_text="$(run_corpus_benchmark sample)"
  assert_contains "$stdout_text" "Corpus Benchmark JSON:"
  assert_contains "$stdout_text" "Corpus Benchmark Markdown:"

  local json_path
  json_path="$(awk -F': ' '/Corpus Benchmark JSON:/ {print $2}' <<<"$stdout_text")"
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

test_complete_supported_corpus_emits_bounded_claim() {
  local stdout_text
  stdout_text="$(run_corpus_benchmark complete_deployed_contracts)"
  local json_path md_path
  json_path="$(awk -F': ' '/Corpus Benchmark JSON:/ {print $2}' <<<"$stdout_text")"
  md_path="$(awk -F': ' '/Corpus Benchmark Markdown:/ {print $2}' <<<"$stdout_text")"
  [[ -f "$json_path" ]] || { echo "missing corpus benchmark json: $json_path" >&2; return 1; }
  [[ -f "$md_path" ]] || { echo "missing corpus benchmark markdown: $md_path" >&2; return 1; }

  local safe all_supported claim markdown_text
  safe="$(jq -r '.claim_guard.safe_to_say_compiled_all_deployed_contracts_in_corpus' "$json_path")"
  all_supported="$(jq -r '.claim_guard.safe_to_say_all_items_native_supported' "$json_path")"
  claim="$(jq -r '.claim_guard.compiled_all_claim_text // ""' "$json_path")"
  markdown_text="$(cat "$md_path")"
  if [[ "$safe" != "true" || "$all_supported" != "true" ]]; then
    echo "complete supported corpus should emit guarded claim" >&2
    cat "$json_path" >&2
    return 1
  fi
  assert_contains "$claim" "Cairo 2.14.0 through 2.16.0"
  assert_contains "$markdown_text" "Compiled-all claim: We compiled every contract"
}

run_test "plan_only_normalizes_sample_corpus" \
  test_plan_only_normalizes_sample_corpus
run_test "rejects_duplicate_tags" \
  test_rejects_duplicate_tags
run_test "rejects_duplicate_class_hash_when_class_deduped" \
  test_rejects_duplicate_class_hash_when_class_deduped
run_test "sample_corpus_blocks_compiled_all_claim" \
  test_sample_corpus_blocks_compiled_all_claim
run_test "complete_supported_corpus_emits_bounded_claim" \
  test_complete_supported_corpus_emits_bounded_claim

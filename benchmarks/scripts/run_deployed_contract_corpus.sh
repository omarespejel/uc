#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
ROOT_DIR="$(git -C "$SCRIPT_DIR/../.." rev-parse --show-toplevel 2>/dev/null || (cd "$SCRIPT_DIR/../.." && pwd -P))"
REAL_REPO_SCRIPT="$SCRIPT_DIR/run_real_repo_benchmarks.sh"
UC_BIN="${UC_BIN:-$ROOT_DIR/target/release/uc}"
RESULTS_DIR="$ROOT_DIR/benchmarks/results"
RUNS="${RUNS:-5}"
COLD_RUNS="${COLD_RUNS:-5}"
CASE_TIMEOUT_SECS="${UC_REAL_REPO_BENCH_TIMEOUT_SECS:-0}"
WARM_SETTLE_SECONDS="${WARM_SETTLE_SECONDS:-2.2}"
PLAN_ONLY=0
CORPUS_PATH=""
STAMP="$(date +%Y%m%d-%H%M%S)"
TMP_DIR="$(mktemp -d)"

cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

usage() {
  cat <<'USAGE'
Usage:
  run_deployed_contract_corpus.sh --corpus <corpus.json>
    [--uc-bin /abs/path/to/uc] [--results-dir /abs/path]
    [--runs <n>] [--cold-runs <n>] [--timeout-secs <seconds>]
    [--warm-settle-seconds <seconds>] [--plan-only]
USAGE
}

require_option_value() {
  local flag="$1"
  local value="${2-}"
  if [[ -z "$value" || "$value" == -* ]]; then
    echo "Missing value for $flag" >&2
    usage >&2
    exit 2
  fi
}

validate_positive_int() {
  local flag="$1"
  local value="$2"
  if [[ ! "$value" =~ ^[0-9]+$ || "$value" -le 0 ]]; then
    echo "$flag must be a positive integer, got: $value" >&2
    exit 2
  fi
}

validate_timeout_secs() {
  local value="$1"
  if [[ ! "$value" =~ ^[0-9]+$ ]]; then
    echo "Invalid timeout seconds: $value" >&2
    exit 2
  fi
}

validate_non_negative_number() {
  local flag="$1"
  local value="$2"
  if [[ ! "$value" =~ ^([0-9]+([.][0-9]+)?|[.][0-9]+)$ ]]; then
    echo "$flag must be a non-negative number, got: $value" >&2
    exit 2
  fi
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --corpus)
      require_option_value "$1" "${2-}"
      CORPUS_PATH="$2"
      shift 2
      ;;
    --uc-bin)
      require_option_value "$1" "${2-}"
      UC_BIN="$2"
      shift 2
      ;;
    --results-dir)
      require_option_value "$1" "${2-}"
      RESULTS_DIR="$2"
      shift 2
      ;;
    --runs)
      require_option_value "$1" "${2-}"
      validate_positive_int "$1" "$2"
      RUNS="$2"
      shift 2
      ;;
    --cold-runs)
      require_option_value "$1" "${2-}"
      validate_positive_int "$1" "$2"
      COLD_RUNS="$2"
      shift 2
      ;;
    --timeout-secs)
      require_option_value "$1" "${2-}"
      validate_timeout_secs "$2"
      CASE_TIMEOUT_SECS="$2"
      shift 2
      ;;
    --warm-settle-seconds)
      require_option_value "$1" "${2-}"
      validate_non_negative_number "$1" "$2"
      WARM_SETTLE_SECONDS="$2"
      shift 2
      ;;
    --plan-only)
      PLAN_ONLY=1
      shift
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

validate_positive_int "RUNS" "$RUNS"
validate_positive_int "COLD_RUNS" "$COLD_RUNS"
validate_timeout_secs "$CASE_TIMEOUT_SECS"
validate_non_negative_number "WARM_SETTLE_SECONDS" "$WARM_SETTLE_SECONDS"

if [[ -z "$CORPUS_PATH" ]]; then
  echo "run_deployed_contract_corpus.sh requires --corpus" >&2
  usage >&2
  exit 2
fi
if [[ ! -f "$CORPUS_PATH" ]]; then
  echo "Corpus file not found: $CORPUS_PATH" >&2
  exit 1
fi
if [[ ! -x "$REAL_REPO_SCRIPT" ]]; then
  echo "Real repo benchmark script is missing or not executable: $REAL_REPO_SCRIPT" >&2
  exit 1
fi
if (( PLAN_ONLY == 0 )) && [[ ! -x "$UC_BIN" ]]; then
  echo "UC binary is missing or not executable: $UC_BIN" >&2
  exit 1
fi
if ! command -v jq >/dev/null 2>&1; then
  echo "jq is required for deployed contract corpus benchmarks" >&2
  exit 1
fi
if ! command -v python3 >/dev/null 2>&1; then
  echo "python3 is required for deployed contract corpus benchmarks" >&2
  exit 1
fi

mkdir -p "$RESULTS_DIR"
CORPUS_ABS="$(cd "$(dirname "$CORPUS_PATH")" && pwd -P)/$(basename "$CORPUS_PATH")"
NORMALIZED_CORPUS="$TMP_DIR/normalized-corpus.json"
CASES_TSV="$TMP_DIR/cases.tsv"

python3 - "$CORPUS_ABS" "$NORMALIZED_CORPUS" "$CASES_TSV" <<'PY'
import json
import re
import sys
from pathlib import Path

corpus_path = Path(sys.argv[1])
out_json = Path(sys.argv[2])
out_tsv = Path(sys.argv[3])
base_dir = corpus_path.parent

def fail(message):
    print(message, file=sys.stderr)
    raise SystemExit(1)

def require_obj(value, name):
    if not isinstance(value, dict):
        fail(f"{name} must be an object")
    return value

def require_str(obj, key, ctx):
    value = obj.get(key)
    if not isinstance(value, str) or not value:
        fail(f"{ctx}.{key} must be a non-empty string")
    return value

def optional_str(obj, key, ctx):
    if key in obj and not isinstance(obj.get(key), str):
        fail(f"{ctx}.{key} must be a string")

def optional_non_empty_str(obj, key, ctx):
    if key in obj:
        value = obj.get(key)
        if not isinstance(value, str) or not value:
            fail(f"{ctx}.{key} must be a non-empty string")

def require_int(obj, key, ctx):
    value = obj.get(key)
    if type(value) is not int or value < 0:
        fail(f"{ctx}.{key} must be a non-negative integer")
    return value

def reject_unknown_keys(obj, allowed, ctx):
    unknown = sorted(set(obj) - set(allowed))
    if unknown:
        fail(f"{ctx} has unsupported field(s): {', '.join(unknown)}")

def version_key(version):
    parts = []
    for part in re.split(r"[._+-]", version):
        if part.isdigit():
            parts.append((0, int(part)))
        else:
            parts.append((1, part))
    return parts

doc = require_obj(json.loads(corpus_path.read_text()), "corpus")
reject_unknown_keys(
    doc,
    {"schema_version", "corpus_id", "chain", "selection", "deduplication", "license_policy", "items"},
    "corpus",
)
if doc.get("schema_version") != 1:
    fail("corpus.schema_version must be 1")
corpus_id = require_str(doc, "corpus_id", "corpus")
chain = require_str(doc, "chain", "corpus")
optional_str(doc, "license_policy", "corpus")
selection = require_obj(doc.get("selection"), "corpus.selection")
reject_unknown_keys(
    selection,
    {"source", "snapshot_id", "from_block", "to_block", "coverage", "notes"},
    "corpus.selection",
)
for key in ["source", "snapshot_id"]:
    require_str(selection, key, "corpus.selection")
from_block = require_int(selection, "from_block", "corpus.selection")
to_block = require_int(selection, "to_block", "corpus.selection")
if to_block < from_block:
    fail("corpus.selection.to_block must be greater than or equal to from_block")
coverage = require_str(selection, "coverage", "corpus.selection")
if coverage not in {"sample", "complete_deployed_contracts"}:
    fail("corpus.selection.coverage must be sample or complete_deployed_contracts")
optional_str(selection, "notes", "corpus.selection")
dedup = require_obj(doc.get("deduplication"), "corpus.deduplication")
reject_unknown_keys(dedup, {"key", "input_count", "deduped_count", "rules"}, "corpus.deduplication")
dedupe_key = require_str(dedup, "key", "corpus.deduplication")
if dedupe_key not in {"class_hash", "source_package", "none"}:
    fail("corpus.deduplication.key must be class_hash, source_package, or none")
input_count = require_int(dedup, "input_count", "corpus.deduplication")
deduped_count = require_int(dedup, "deduped_count", "corpus.deduplication")
optional_str(dedup, "rules", "corpus.deduplication")
items = doc.get("items")
if not isinstance(items, list) or not items:
    fail("corpus.items must be a non-empty array")
if deduped_count != len(items):
    fail(f"corpus.deduplication.deduped_count ({deduped_count}) must equal items length ({len(items)})")
if input_count < deduped_count:
    fail("corpus.deduplication.input_count must be >= deduped_count")
if dedupe_key == "none" and input_count != deduped_count:
    fail("corpus.deduplication.input_count must equal deduped_count when deduplication.key is none")

tag_re = re.compile(r"^[A-Za-z0-9._-]+$")
seen_tags = set()
seen_class_hashes = set()
normalized_items = []
for index, item in enumerate(items):
    item = require_obj(item, f"corpus.items[{index}]")
    reject_unknown_keys(
        item,
        {
            "tag",
            "source_kind",
            "contract_address",
            "class_hash",
            "source_ref",
            "manifest_path",
            "cairo_version",
            "scarb_version",
            "license",
            "notes",
        },
        f"corpus.items[{index}]",
    )
    tag = require_str(item, "tag", f"corpus.items[{index}]")
    if not tag_re.match(tag):
        fail(f"corpus.items[{index}].tag has invalid characters: {tag}")
    if tag in seen_tags:
        fail(f"duplicate corpus item tag: {tag}")
    seen_tags.add(tag)
    class_hash = require_str(item, "class_hash", f"corpus.items[{index}]")
    source_kind_present = "source_kind" in item
    source_kind = item.get("source_kind", "deployed_contract")
    if not isinstance(source_kind, str) or not source_kind:
        fail(f"corpus.items[{index}].source_kind must be deployed_contract or declared_class")
    if source_kind not in {"deployed_contract", "declared_class"}:
        fail(f"corpus.items[{index}].source_kind must be deployed_contract or declared_class")
    if coverage == "complete_deployed_contracts" and (
        not source_kind_present or source_kind != "deployed_contract"
    ):
        fail(
            f"corpus.items[{index}].source_kind must be explicitly deployed_contract "
            "when corpus.selection.coverage is complete_deployed_contracts"
        )
    if source_kind == "deployed_contract":
        require_str(item, "contract_address", f"corpus.items[{index}]")
    else:
        optional_non_empty_str(item, "contract_address", f"corpus.items[{index}]")
    if dedupe_key == "class_hash":
        if class_hash in seen_class_hashes:
            fail(f"duplicate class_hash in class_hash-deduped corpus: {class_hash}")
        seen_class_hashes.add(class_hash)
    manifest_raw = require_str(item, "manifest_path", f"corpus.items[{index}]")
    manifest_path = Path(manifest_raw)
    if not manifest_path.is_absolute():
        manifest_path = base_dir / manifest_path
    manifest_path = manifest_path.resolve()
    if not manifest_path.is_file():
        fail(f"manifest_path does not exist for {tag}: {manifest_path}")
    normalized = dict(item)
    normalized["manifest_path"] = str(manifest_path)
    normalized["source_kind"] = source_kind
    normalized["source_ref"] = require_str(item, "source_ref", f"corpus.items[{index}]")
    normalized["cairo_version"] = require_str(item, "cairo_version", f"corpus.items[{index}]")
    for optional_key in ["scarb_version", "license", "notes"]:
        optional_str(item, optional_key, f"corpus.items[{index}]")
    normalized_items.append(normalized)

versions = sorted({item["cairo_version"] for item in normalized_items}, key=version_key)
normalized_doc = dict(doc)
normalized_doc["items"] = normalized_items
normalized_doc["summary"] = {
    "item_count": len(normalized_items),
    "unique_class_hash_count": len({item["class_hash"] for item in normalized_items}),
    "source_kind_counts": {
        kind: sum(1 for item in normalized_items if item["source_kind"] == kind)
        for kind in sorted({item["source_kind"] for item in normalized_items})
    },
    "cairo_version_min": versions[0],
    "cairo_version_max": versions[-1],
    "cairo_versions": versions,
}
out_json.write_text(json.dumps(normalized_doc, indent=2, sort_keys=True) + "\n")
with out_tsv.open("w") as handle:
    for item in normalized_items:
        handle.write(f"{item['manifest_path']}\t{item['tag']}\n")
PY

PLAN_JSON="$RESULTS_DIR/deployed-contract-corpus-plan-$STAMP.json"
GENERATED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
if (( PLAN_ONLY == 1 )); then
  jq -n \
    --arg generated_at "$GENERATED_AT" \
    --arg corpus_path "$CORPUS_ABS" \
    --arg uc_bin "$UC_BIN" \
    --arg results_dir "$RESULTS_DIR" \
    --argjson runs "$RUNS" \
    --argjson cold_runs "$COLD_RUNS" \
    --argjson timeout_secs "$CASE_TIMEOUT_SECS" \
    --argjson warm_settle_seconds "$WARM_SETTLE_SECONDS" \
    --slurpfile corpus "$NORMALIZED_CORPUS" \
    '{
      schema_version: 1,
      generated_at: $generated_at,
      corpus_path: $corpus_path,
      plan_only: true,
      corpus: $corpus[0],
      benchmark_command_shape: "run_real_repo_benchmarks.sh --cases-file <manifest-tag-tsv>",
      run_config: {
        uc_bin: $uc_bin,
        results_dir: $results_dir,
        runs: $runs,
        cold_runs: $cold_runs,
        timeout_secs: $timeout_secs,
        warm_settle_seconds: $warm_settle_seconds
      }
    }' > "$PLAN_JSON"
  echo "Corpus plan JSON: $PLAN_JSON"
  exit 0
fi

bench_stdout="$TMP_DIR/real-repo-benchmark.stdout"
"$REAL_REPO_SCRIPT" \
  --uc-bin "$UC_BIN" \
  --results-dir "$RESULTS_DIR" \
  --runs "$RUNS" \
  --cold-runs "$COLD_RUNS" \
  --timeout-secs "$CASE_TIMEOUT_SECS" \
  --warm-settle-seconds "$WARM_SETTLE_SECONDS" \
  --cases-file "$CASES_TSV" | tee "$bench_stdout"

REAL_JSON="$(awk '/^Benchmark JSON: / {sub(/^Benchmark JSON: /, ""); print}' "$bench_stdout" | tail -n 1)"
REAL_MD="$(awk '/^Benchmark Markdown: / {sub(/^Benchmark Markdown: /, ""); print}' "$bench_stdout" | tail -n 1)"
if [[ -z "$REAL_JSON" || ! -f "$REAL_JSON" ]]; then
  echo "real-repo benchmark did not produce a JSON artifact" >&2
  exit 1
fi
if [[ -z "$REAL_MD" || ! -f "$REAL_MD" ]]; then
  echo "real-repo benchmark did not produce a Markdown artifact" >&2
  exit 1
fi

OUT_JSON="$RESULTS_DIR/deployed-contract-corpus-bench-$STAMP.json"
OUT_MD="$RESULTS_DIR/deployed-contract-corpus-bench-$STAMP.md"

jq -n \
  --arg generated_at "$GENERATED_AT" \
  --arg corpus_path "$CORPUS_ABS" \
  --arg real_repo_json "$REAL_JSON" \
  --arg real_repo_markdown "$REAL_MD" \
  --slurpfile corpus "$NORMALIZED_CORPUS" \
  --slurpfile bench "$REAL_JSON" \
  'def counts: $bench[0].summary.support_matrix;
   def item_count: ($corpus[0].summary.item_count // 0);
   def non_deployed_item_count:
     ([ $corpus[0].items[] | select(.source_kind != "deployed_contract") ] | length);
   def failed_native_benchmarks:
     ([ $bench[0].cases[] | select(.benchmark_status == "failed") ] | length);
   def all_items_native_supported:
     (item_count > 0)
     and ((counts.native_supported // 0) == item_count)
     and ((counts.fallback_used // 0) == 0)
     and ((counts.native_unsupported // 0) == 0)
     and ((counts.build_failed // 0) == 0);
   def compiled_without_dedup_guard:
     ($corpus[0].selection.coverage == "complete_deployed_contracts")
     and (non_deployed_item_count == 0)
     and all_items_native_supported
     and (failed_native_benchmarks == 0);
   def valid_selected_unit_accounting:
     ($corpus[0].deduplication.key != "none")
     or (($corpus[0].deduplication.input_count // 0) == item_count);
   def compiled_selected_units:
     compiled_without_dedup_guard and valid_selected_unit_accounting;
   def compiled_all_contracts:
     compiled_without_dedup_guard
     and ($corpus[0].deduplication.key == "none")
     and (($corpus[0].deduplication.input_count // 0) == item_count);
   def native_all:
     all_items_native_supported and (failed_native_benchmarks == 0);
   def deduplication_phrase:
     if $corpus[0].deduplication.key == "none" then
       "without deduplication"
     else
       "after \($corpus[0].deduplication.key) deduplication"
     end;
   def quantity($n; $singular; $plural):
     "\($n) \(if (($n // 0) | tonumber) == 1 then $singular else $plural end)";
   {
     schema_version: 1,
     generated_at: $generated_at,
     corpus_path: $corpus_path,
     real_repo_json: $real_repo_json,
     real_repo_markdown: $real_repo_markdown,
     corpus: $corpus[0],
     benchmark: $bench[0],
     summary: {
       item_count: $corpus[0].summary.item_count,
       unique_class_hash_count: $corpus[0].summary.unique_class_hash_count,
       source_kind_counts: ($corpus[0].summary.source_kind_counts // {}),
       cairo_version_min: $corpus[0].summary.cairo_version_min,
       cairo_version_max: $corpus[0].summary.cairo_version_max,
       support_matrix: counts,
       unstable_lane_count: ($bench[0].summary.unstable_lane_count // 0)
     },
     claim_guard: {
       safe_to_say_compiled_all_deployed_contracts_in_corpus: compiled_all_contracts,
       safe_to_say_compiled_all_selected_deployed_units_in_corpus: compiled_selected_units,
       safe_to_say_all_items_native_supported: native_all,
       reason: (
         if $corpus[0].selection.coverage != "complete_deployed_contracts" then
           "corpus selection coverage is sample, not complete_deployed_contracts"
         elif non_deployed_item_count != 0 then
           "corpus contains non-deployed source_kind rows"
         elif compiled_without_dedup_guard and ($corpus[0].deduplication.key == "none") and (($corpus[0].deduplication.input_count // 0) != item_count) then
           "corpus uses deduplication.key=none but deduplication.input_count does not equal item_count; address coverage is incomplete"
         elif compiled_selected_units and ($corpus[0].deduplication.key != "none") then
           "corpus is deduplicated by \($corpus[0].deduplication.key); safe only for selected deduped deployed units, not every deployed contract address"
         elif (counts.fallback_used // 0) != 0 then
           "one or more corpus items used fallback"
         elif (counts.native_unsupported // 0) != 0 then
           "one or more corpus items are native_unsupported"
         elif (counts.build_failed // 0) != 0 then
           "one or more corpus items failed during auto-build classification"
         elif (counts.native_supported // 0) != item_count then
           "not every corpus item was native_supported"
         elif failed_native_benchmarks != 0 then
           "one or more native-supported benchmark cases failed"
         else
           "claim is bounded to this pinned corpus artifact"
         end
       ),
       compiled_all_claim_text: (
         if compiled_all_contracts then
           "We compiled every contract in the pinned \($corpus[0].chain) deployed-contract corpus (\(quantity($corpus[0].summary.item_count; "item"; "items")), \(quantity($corpus[0].summary.unique_class_hash_count; "unique class hash"; "unique class hashes")), Cairo \($corpus[0].summary.cairo_version_min) through \($corpus[0].summary.cairo_version_max)) and published support/benchmark artifacts."
         else null end
       ),
       selected_units_claim_text: (
         if compiled_selected_units then
           "We compiled every selected deployed unit in the pinned \($corpus[0].chain) deployed-contract corpus \(deduplication_phrase) (\(quantity($corpus[0].summary.item_count; "item"; "items")) from \(quantity($corpus[0].deduplication.input_count; "input record"; "input records")), \(quantity($corpus[0].summary.unique_class_hash_count; "unique class hash"; "unique class hashes")), Cairo \($corpus[0].summary.cairo_version_min) through \($corpus[0].summary.cairo_version_max)) and published support/benchmark artifacts."
         else null end
       ),
       native_supported_claim_text: (
         if native_all then
           "Every item in the pinned \($corpus[0].chain) corpus was native-supported in this run."
         else null end
       )
     }
   }' > "$OUT_JSON"

{
  echo "# Deployed Contract Corpus Benchmark ($STAMP)"
  echo
  jq -r '
    "- Generated at: \(.generated_at)",
    "- Corpus: \(.corpus.corpus_id)",
    "- Chain: \(.corpus.chain)",
    "- Coverage: \(.corpus.selection.coverage)",
    "- Block range: \(.corpus.selection.from_block)-\(.corpus.selection.to_block)",
    "- Snapshot: \(.corpus.selection.snapshot_id)",
    "- Items: \(.summary.item_count)",
    "- Unique class hashes: \(.summary.unique_class_hash_count)",
    "- Cairo versions: \(.summary.cairo_version_min) through \(.summary.cairo_version_max)",
    "- Real repo benchmark JSON: \(.real_repo_json)",
    "- Real repo benchmark Markdown: \(.real_repo_markdown)"
  ' "$OUT_JSON"
  echo
  echo "## Claim Guard"
  echo
  jq -r '
    "- Safe to say compiled every deployed contract in this corpus: \(.claim_guard.safe_to_say_compiled_all_deployed_contracts_in_corpus)",
    "- Safe to say compiled every selected deployed unit in this corpus: \(.claim_guard.safe_to_say_compiled_all_selected_deployed_units_in_corpus)",
    "- Safe to say every item was native-supported: \(.claim_guard.safe_to_say_all_items_native_supported)",
    "- Reason: \(.claim_guard.reason)",
    (if .claim_guard.compiled_all_claim_text then "- Compiled-all claim: \(.claim_guard.compiled_all_claim_text)" else "- Compiled-all claim: <not safe for this artifact>" end),
    (if .claim_guard.selected_units_claim_text then "- Selected-unit claim: \(.claim_guard.selected_units_claim_text)" else "- Selected-unit claim: <not safe for this artifact>" end),
    (if .claim_guard.native_supported_claim_text then "- Native-supported claim: \(.claim_guard.native_supported_claim_text)" else "- Native-supported claim: <not safe for this artifact>" end)
  ' "$OUT_JSON"
  echo
  echo "## Support Matrix Summary"
  echo "| Classification | Count |"
  echo "|---|---:|"
  jq -r '
    .summary.support_matrix
    | to_entries[]
    | "| \(.key) | \(.value) |"
  ' "$OUT_JSON"
  echo
  echo "## Source Kind Summary"
  echo "| Source Kind | Count |"
  echo "|---|---:|"
  jq -r '
    .summary.source_kind_counts
    | to_entries[]
    | "| \(.key) | \(.value) |"
  ' "$OUT_JSON"
  echo
  echo "## Corpus Items"
  echo "| Tag | Source Kind | Address/Class Ref | Class Hash | Cairo Version | Source Ref | Manifest |"
  echo "|---|---|---|---|---|---|---|"
  jq -r '
    .corpus.items[]
    | "| \(.tag) | \(.source_kind) | \(if .source_kind == "deployed_contract" then .contract_address else "declared-class:" + .class_hash end) | \(.class_hash) | \(.cairo_version) | \(.source_ref) | \(.manifest_path) |"
  ' "$OUT_JSON"
  echo
  echo "## Real Repo Benchmark Summary"
  echo
  cat "$REAL_MD"
} > "$OUT_MD"

echo "Corpus Benchmark JSON: $OUT_JSON"
echo "Corpus Benchmark Markdown: $OUT_MD"

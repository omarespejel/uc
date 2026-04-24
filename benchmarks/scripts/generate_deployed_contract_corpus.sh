#!/usr/bin/env bash
set -euo pipefail

SOURCE_INDEX_PATH=""
OUT_PATH=""

usage() {
  cat <<'USAGE'
Usage:
  generate_deployed_contract_corpus.sh --source-index <source-index.json> --out <corpus.json>

Validates a deployed-contract source index and writes a benchmark corpus that can
be passed to run_deployed_contract_corpus.sh. The source index is the durable
selection artifact; generated corpus files are run artifacts.
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

while [[ $# -gt 0 ]]; do
  case "$1" in
    --source-index)
      require_option_value "$1" "${2-}"
      SOURCE_INDEX_PATH="$2"
      shift 2
      ;;
    --out)
      require_option_value "$1" "${2-}"
      OUT_PATH="$2"
      shift 2
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

if [[ -z "$SOURCE_INDEX_PATH" ]]; then
  echo "generate_deployed_contract_corpus.sh requires --source-index" >&2
  usage >&2
  exit 2
fi
if [[ -z "$OUT_PATH" ]]; then
  echo "generate_deployed_contract_corpus.sh requires --out" >&2
  usage >&2
  exit 2
fi
if [[ ! -f "$SOURCE_INDEX_PATH" ]]; then
  echo "Source index file not found: $SOURCE_INDEX_PATH" >&2
  exit 1
fi
if ! command -v python3 >/dev/null 2>&1; then
  echo "python3 is required for deployed contract corpus generation" >&2
  exit 1
fi

SOURCE_INDEX_ABS="$(cd "$(dirname "$SOURCE_INDEX_PATH")" && pwd -P)/$(basename "$SOURCE_INDEX_PATH")"
mkdir -p "$(dirname "$OUT_PATH")"
OUT_ABS="$(cd "$(dirname "$OUT_PATH")" && pwd -P)/$(basename "$OUT_PATH")"
if [[ "$SOURCE_INDEX_ABS" == "$OUT_ABS" ]] || [[ -e "$OUT_ABS" && "$SOURCE_INDEX_ABS" -ef "$OUT_ABS" ]]; then
  echo "Refusing to overwrite source index with generated corpus: $SOURCE_INDEX_ABS" >&2
  exit 2
fi
if [[ -d "$OUT_ABS" ]]; then
  echo "--out must be a file path, got directory: $OUT_ABS" >&2
  exit 2
fi

python3 - "$SOURCE_INDEX_ABS" "$OUT_ABS" <<'PY'
import json
import os
import re
import sys
from pathlib import Path

source_index_path = Path(sys.argv[1])
out_path = Path(sys.argv[2])
base_dir = source_index_path.parent

TOP_KEYS = {
    "schema_version",
    "corpus_id",
    "chain",
    "selection",
    "deduplication",
    "license_policy",
    "source_availability",
    "items",
}
SELECTION_KEYS = {"source", "snapshot_id", "from_block", "to_block", "coverage", "notes"}
DEDUP_KEYS = {"key", "input_count", "deduped_count", "rules"}
SOURCE_AVAILABILITY_KEYS = {"policy", "notes"}
ITEM_KEYS = {
    "tag",
    "contract_address",
    "class_hash",
    "source_ref",
    "manifest_path",
    "cairo_version",
    "scarb_version",
    "license",
    "notes",
}
CORPUS_ITEM_KEYS = {
    "tag",
    "contract_address",
    "class_hash",
    "source_ref",
    "manifest_path",
    "cairo_version",
    "scarb_version",
    "license",
    "notes",
}

def fail(message):
    print(message, file=sys.stderr)
    raise SystemExit(1)

def require_obj(value, name):
    if not isinstance(value, dict):
        fail(f"{name} must be an object")
    return value

def reject_unknown_keys(obj, allowed, ctx):
    unknown = sorted(set(obj) - allowed)
    if unknown:
        fail(f"{ctx} has unsupported field(s): {', '.join(unknown)}")

def require_str(obj, key, ctx):
    value = obj.get(key)
    if not isinstance(value, str) or not value:
        fail(f"{ctx}.{key} must be a non-empty string")
    return value

def optional_str(obj, key, ctx):
    if key in obj and not isinstance(obj.get(key), str):
        fail(f"{ctx}.{key} must be a string")

def require_int(obj, key, ctx):
    value = obj.get(key)
    if type(value) is not int or value < 0:
        fail(f"{ctx}.{key} must be a non-negative integer")
    return value

def resolve_manifest_path(raw, tag):
    manifest_path = Path(raw)
    if manifest_path.is_absolute():
        fail(f"manifest_path for {tag} must be relative to the source index: {manifest_path}")
    manifest_path = base_dir / manifest_path
    manifest_path = manifest_path.resolve()
    if not manifest_path.is_file():
        fail(f"manifest_path does not exist for {tag}: {manifest_path}")
    if manifest_path.name != "Scarb.toml":
        fail(f"manifest_path for {tag} must point to Scarb.toml: {manifest_path}")
    return manifest_path

try:
    doc = require_obj(json.loads(source_index_path.read_text()), "source_index")
except json.JSONDecodeError as exc:
    fail(f"source_index is not valid JSON: {exc}")

reject_unknown_keys(doc, TOP_KEYS, "source_index")
if doc.get("schema_version") != 1:
    fail("source_index.schema_version must be 1")
corpus_id = require_str(doc, "corpus_id", "source_index")
chain = require_str(doc, "chain", "source_index")
license_policy = require_str(doc, "license_policy", "source_index")

selection = require_obj(doc.get("selection"), "source_index.selection")
reject_unknown_keys(selection, SELECTION_KEYS, "source_index.selection")
for key in ["source", "snapshot_id"]:
    require_str(selection, key, "source_index.selection")
from_block = require_int(selection, "from_block", "source_index.selection")
to_block = require_int(selection, "to_block", "source_index.selection")
if to_block < from_block:
    fail("source_index.selection.to_block must be greater than or equal to from_block")
coverage = require_str(selection, "coverage", "source_index.selection")
if coverage not in {"sample", "complete_deployed_contracts"}:
    fail("source_index.selection.coverage must be sample or complete_deployed_contracts")
optional_str(selection, "notes", "source_index.selection")

dedup = require_obj(doc.get("deduplication"), "source_index.deduplication")
reject_unknown_keys(dedup, DEDUP_KEYS, "source_index.deduplication")
dedupe_key = require_str(dedup, "key", "source_index.deduplication")
if dedupe_key not in {"class_hash", "source_package", "none"}:
    fail("source_index.deduplication.key must be class_hash, source_package, or none")
input_count = require_int(dedup, "input_count", "source_index.deduplication")
deduped_count = require_int(dedup, "deduped_count", "source_index.deduplication")
optional_str(dedup, "rules", "source_index.deduplication")

source_availability = require_obj(doc.get("source_availability"), "source_index.source_availability")
reject_unknown_keys(source_availability, SOURCE_AVAILABILITY_KEYS, "source_index.source_availability")
source_policy = require_str(source_availability, "policy", "source_index.source_availability")
if source_policy not in {"local_manifest_paths", "verified_source_refs"}:
    fail("source_index.source_availability.policy must be local_manifest_paths or verified_source_refs")
optional_str(source_availability, "notes", "source_index.source_availability")

items = doc.get("items")
if not isinstance(items, list) or not items:
    fail("source_index.items must be a non-empty array")
if deduped_count != len(items):
    fail(f"source_index.deduplication.deduped_count ({deduped_count}) must equal items length ({len(items)})")
if input_count < deduped_count:
    fail("source_index.deduplication.input_count must be >= deduped_count")

tag_re = re.compile(r"^[A-Za-z0-9._-]+$")
seen_tags = set()
seen_class_hashes = set()
normalized_items = []
for index, raw_item in enumerate(items):
    item = require_obj(raw_item, f"source_index.items[{index}]")
    reject_unknown_keys(item, ITEM_KEYS, f"source_index.items[{index}]")
    tag = require_str(item, "tag", f"source_index.items[{index}]")
    if not tag_re.match(tag):
        fail(f"source_index.items[{index}].tag has invalid characters: {tag}")
    if tag in seen_tags:
        fail(f"duplicate source index item tag: {tag}")
    seen_tags.add(tag)

    class_hash = require_str(item, "class_hash", f"source_index.items[{index}]")
    if dedupe_key == "class_hash":
        if class_hash in seen_class_hashes:
            fail(f"duplicate class_hash in class_hash-deduped source index: {class_hash}")
        seen_class_hashes.add(class_hash)

    manifest_raw = require_str(item, "manifest_path", f"source_index.items[{index}]")
    manifest_path = resolve_manifest_path(manifest_raw, tag)

    normalized = {key: item[key] for key in CORPUS_ITEM_KEYS if key in item}
    normalized["manifest_path"] = str(manifest_path)
    normalized["contract_address"] = require_str(item, "contract_address", f"source_index.items[{index}]")
    normalized["source_ref"] = require_str(item, "source_ref", f"source_index.items[{index}]")
    normalized["cairo_version"] = require_str(item, "cairo_version", f"source_index.items[{index}]")
    for optional_key in ["scarb_version", "license", "notes"]:
        optional_str(item, optional_key, f"source_index.items[{index}]")
    normalized_items.append(normalized)

corpus = {
    "schema_version": 1,
    "corpus_id": corpus_id,
    "chain": chain,
    "selection": dict(selection),
    "deduplication": dict(dedup),
    "license_policy": license_policy,
    "items": normalized_items,
}
if out_path.exists():
    try:
        if os.path.samefile(source_index_path, out_path):
            fail(f"Refusing to overwrite source index with generated corpus: {source_index_path.resolve()}")
    except OSError as exc:
        fail(f"unable to validate --out path: {exc}")
out_path.write_text(json.dumps(corpus, indent=2, sort_keys=True) + "\n")
PY

echo "Corpus JSON: $OUT_ABS"

#!/usr/bin/env bash
set -euo pipefail

INVENTORY_PATH=""
OUT_PATH=""

usage() {
  cat <<'USAGE'
Usage:
  build_deployed_contract_source_index.sh --inventory <inventory.json> --out <source-index.json>

Validates a reviewed deployed-contract source inventory, deduplicates it by the
inventory policy, and writes a source-index JSON file for
generate_deployed_contract_corpus.sh. The inventory is the durable raw evidence
input; the source index is deterministic generated evidence.
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
    --inventory)
      require_option_value "$1" "${2-}"
      INVENTORY_PATH="$2"
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

if [[ -z "$INVENTORY_PATH" ]]; then
  echo "build_deployed_contract_source_index.sh requires --inventory" >&2
  usage >&2
  exit 2
fi
if [[ -z "$OUT_PATH" ]]; then
  echo "build_deployed_contract_source_index.sh requires --out" >&2
  usage >&2
  exit 2
fi
if [[ ! -f "$INVENTORY_PATH" ]]; then
  echo "Inventory file not found: $INVENTORY_PATH" >&2
  exit 1
fi
if ! command -v python3 >/dev/null 2>&1; then
  echo "python3 is required for deployed contract source-index building" >&2
  exit 1
fi

INVENTORY_DIR_ABS="$(cd "$(dirname "$INVENTORY_PATH")" && pwd -P)"
INVENTORY_ABS="$INVENTORY_DIR_ABS/$(basename "$INVENTORY_PATH")"
OUT_DIR_RAW="$(dirname "$OUT_PATH")"
if [[ ! -d "$OUT_DIR_RAW" ]]; then
  echo "--out parent directory must exist and match the inventory directory: $OUT_DIR_RAW" >&2
  exit 2
fi
OUT_DIR_ABS="$(cd "$OUT_DIR_RAW" && pwd -P)"
if [[ "$OUT_DIR_ABS" != "$INVENTORY_DIR_ABS" ]]; then
  echo "--out must be written next to --inventory so generated manifest paths stay confined: $OUT_PATH" >&2
  exit 2
fi
OUT_ABS="$OUT_DIR_ABS/$(basename "$OUT_PATH")"
if [[ "$INVENTORY_ABS" == "$OUT_ABS" ]] || [[ -e "$OUT_ABS" && "$INVENTORY_ABS" -ef "$OUT_ABS" ]]; then
  echo "Refusing to overwrite source inventory with generated source index: $INVENTORY_ABS" >&2
  exit 2
fi
if [[ -d "$OUT_ABS" ]]; then
  echo "--out must be a file path, got directory: $OUT_ABS" >&2
  exit 2
fi

python3 - "$INVENTORY_ABS" "$OUT_ABS" <<'PY'
import json
import os
import re
import sys
import tempfile
from pathlib import Path

inventory_path = Path(sys.argv[1])
out_path = Path(sys.argv[2])
inventory_dir = inventory_path.parent
out_dir = out_path.parent

TOP_KEYS = {
    "schema_version",
    "corpus_id",
    "chain",
    "selection",
    "deduplication",
    "license_policy",
    "source_availability",
    "records",
}
SELECTION_KEYS = {"source", "snapshot_id", "from_block", "to_block", "coverage", "notes"}
DEDUP_KEYS = {"key", "rules"}
SOURCE_AVAILABILITY_KEYS = {"policy", "notes"}
RECORD_KEYS = {
    "tag",
    "source_kind",
    "contract_address",
    "class_hash",
    "source_ref",
    "manifest_path",
    "cairo_version",
    "source_package_id",
    "scarb_version",
    "license",
    "notes",
}
SOURCE_INDEX_ITEM_KEYS = {
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
}


def fail(message):
    print(message, file=sys.stderr)
    raise SystemExit(1)


def reject_output_inventory_alias():
    if out_path.exists():
        if out_path.is_dir():
            fail(f"--out must be a file path, got directory: {out_path}")
        try:
            if os.path.samefile(inventory_path, out_path):
                fail(f"Refusing to overwrite source inventory with generated source index: {inventory_path.resolve()}")
        except OSError as exc:
            fail(f"unable to validate --out path: {exc}")


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


def resolve_manifest_path(raw, tag):
    manifest_path = Path(raw)
    if manifest_path.is_absolute():
        fail(f"manifest_path for {tag} must be relative to the inventory: {manifest_path}")
    manifest_path = (inventory_dir / manifest_path).resolve()
    try:
        manifest_path.relative_to(inventory_dir)
    except ValueError:
        fail(f"manifest_path for {tag} must stay under the inventory directory: {manifest_path}")
    if not manifest_path.is_file():
        fail(f"manifest_path does not exist for {tag}: {manifest_path}")
    if manifest_path.name != "Scarb.toml":
        fail(f"manifest_path for {tag} must point to Scarb.toml: {manifest_path}")
    return manifest_path


try:
    doc = require_obj(json.loads(inventory_path.read_text()), "inventory")
except json.JSONDecodeError as exc:
    fail(f"inventory is not valid JSON: {exc}")

reject_unknown_keys(doc, TOP_KEYS, "inventory")
schema_version = doc.get("schema_version")
if type(schema_version) is not int or schema_version != 1:
    fail("inventory.schema_version must be 1")
corpus_id = require_str(doc, "corpus_id", "inventory")
chain = require_str(doc, "chain", "inventory")
license_policy = require_str(doc, "license_policy", "inventory")

selection = require_obj(doc.get("selection"), "inventory.selection")
reject_unknown_keys(selection, SELECTION_KEYS, "inventory.selection")
for key in ["source", "snapshot_id"]:
    require_str(selection, key, "inventory.selection")
from_block = require_int(selection, "from_block", "inventory.selection")
to_block = require_int(selection, "to_block", "inventory.selection")
if to_block < from_block:
    fail("inventory.selection.to_block must be greater than or equal to from_block")
coverage = require_str(selection, "coverage", "inventory.selection")
if coverage not in {"sample", "complete_deployed_contracts"}:
    fail("inventory.selection.coverage must be sample or complete_deployed_contracts")
optional_str(selection, "notes", "inventory.selection")

dedup = require_obj(doc.get("deduplication"), "inventory.deduplication")
reject_unknown_keys(dedup, DEDUP_KEYS, "inventory.deduplication")
dedupe_key = require_str(dedup, "key", "inventory.deduplication")
if dedupe_key not in {"class_hash", "source_package", "none"}:
    fail("inventory.deduplication.key must be class_hash, source_package, or none")
optional_str(dedup, "rules", "inventory.deduplication")

source_availability = require_obj(doc.get("source_availability"), "inventory.source_availability")
reject_unknown_keys(source_availability, SOURCE_AVAILABILITY_KEYS, "inventory.source_availability")
source_policy = require_str(source_availability, "policy", "inventory.source_availability")
if source_policy != "local_manifest_paths":
    fail("inventory.source_availability.policy must be local_manifest_paths")
optional_str(source_availability, "notes", "inventory.source_availability")

records = doc.get("records")
if not isinstance(records, list) or not records:
    fail("inventory.records must be a non-empty array")

tag_re = re.compile(r"^[A-Za-z0-9._-]+$")
seen_tags = set()
selected_by_key = {}
selected_records = []

for index, raw_record in enumerate(records):
    record = require_obj(raw_record, f"inventory.records[{index}]")
    reject_unknown_keys(record, RECORD_KEYS, f"inventory.records[{index}]")
    tag = require_str(record, "tag", f"inventory.records[{index}]")
    if not tag_re.match(tag):
        fail(f"inventory.records[{index}].tag has invalid characters: {tag}")
    if tag in seen_tags:
        fail(f"duplicate inventory record tag: {tag}")
    seen_tags.add(tag)

    class_hash = require_str(record, "class_hash", f"inventory.records[{index}]")
    source_kind_present = "source_kind" in record
    source_kind = record.get("source_kind", "deployed_contract")
    if not isinstance(source_kind, str) or not source_kind:
        fail(f"inventory.records[{index}].source_kind must be deployed_contract or declared_class")
    if source_kind not in {"deployed_contract", "declared_class"}:
        fail(f"inventory.records[{index}].source_kind must be deployed_contract or declared_class")
    if coverage == "complete_deployed_contracts" and (
        not source_kind_present or source_kind != "deployed_contract"
    ):
        fail(
            f"inventory.records[{index}].source_kind must be explicitly deployed_contract "
            "when inventory.selection.coverage is complete_deployed_contracts"
        )
    if source_kind == "deployed_contract":
        require_str(record, "contract_address", f"inventory.records[{index}]")
    else:
        optional_non_empty_str(record, "contract_address", f"inventory.records[{index}]")
    manifest_raw = require_str(record, "manifest_path", f"inventory.records[{index}]")
    manifest_path = resolve_manifest_path(manifest_raw, tag)
    source_package_id = record.get("source_package_id")
    if dedupe_key == "source_package" and (not isinstance(source_package_id, str) or not source_package_id):
        fail(f"inventory.records[{index}].source_package_id must be a non-empty string when deduplication.key is source_package")
    if "source_package_id" in record:
        require_str(record, "source_package_id", f"inventory.records[{index}]")

    normalized = {key: record[key] for key in SOURCE_INDEX_ITEM_KEYS if key in record}
    normalized["manifest_path"] = os.path.relpath(manifest_path, out_dir)
    normalized["source_kind"] = source_kind
    normalized["source_ref"] = require_str(record, "source_ref", f"inventory.records[{index}]")
    normalized["cairo_version"] = require_str(record, "cairo_version", f"inventory.records[{index}]")
    for optional_key in ["scarb_version", "license", "notes"]:
        optional_str(record, optional_key, f"inventory.records[{index}]")

    if dedupe_key == "none":
        selected_records.append(normalized)
        continue
    if dedupe_key == "class_hash":
        dedupe_value = class_hash
    else:
        dedupe_value = source_package_id
    if dedupe_value not in selected_by_key:
        selected_by_key[dedupe_value] = normalized
        selected_records.append(normalized)

source_index = {
    "schema_version": 1,
    "corpus_id": corpus_id,
    "chain": chain,
    "selection": dict(selection),
    "deduplication": {
        "key": dedupe_key,
        "input_count": len(records),
        "deduped_count": len(selected_records),
    },
    "license_policy": license_policy,
    "source_availability": dict(source_availability),
    "items": selected_records,
}
if "rules" in dedup:
    source_index["deduplication"]["rules"] = dedup["rules"]

payload = json.dumps(source_index, indent=2, sort_keys=True) + "\n"
reject_output_inventory_alias()
tmp_fd, tmp_name = tempfile.mkstemp(
    prefix=".tmp.deployed-contract-source-index.",
    suffix=".json",
    dir=str(out_dir),
)
tmp_path = Path(tmp_name)
try:
    with os.fdopen(tmp_fd, "w", encoding="utf-8") as handle:
        handle.write(payload)
        handle.flush()
        os.fsync(handle.fileno())
    reject_output_inventory_alias()
    os.replace(tmp_path, out_path)
finally:
    if tmp_path.exists():
        tmp_path.unlink()
PY

echo "Source index JSON: $OUT_ABS"

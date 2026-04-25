#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
LANE=""
OUTPUT=""
STAGING_DIR=""
TARGET_DIR=""
PREPARE_ONLY=0
CHECK_ONLY=0
KEEP_STAGING=0
STAGING_CREATED=0
PATCH_DIR_REL=""
PATCHED_CRATES=()

usage() {
  cat <<'USAGE'
Usage:
  build_native_toolchain_helper.sh --lane <major.minor> [--output /abs/path/to/uc]
    [--staging-dir /abs/path] [--target-dir /abs/path] [--prepare-only] [--check-only] [--keep-staging]

Examples:
  ./scripts/build_native_toolchain_helper.sh --lane 2.14
  ./scripts/build_native_toolchain_helper.sh --lane 2.14 --output "$HOME/.uc/toolchain-helpers/uc-cairo214-helper/bin/uc"
  ./scripts/build_native_toolchain_helper.sh --lane 2.14 --check-only
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
    --lane)
      require_option_value "$1" "${2-}"
      LANE="$2"
      shift 2
      ;;
    --output)
      require_option_value "$1" "${2-}"
      OUTPUT="$2"
      shift 2
      ;;
    --staging-dir)
      require_option_value "$1" "${2-}"
      STAGING_DIR="$2"
      shift 2
      ;;
    --target-dir)
      require_option_value "$1" "${2-}"
      TARGET_DIR="$2"
      shift 2
      ;;
    --prepare-only)
      PREPARE_ONLY=1
      shift
      ;;
    --check-only)
      CHECK_ONLY=1
      shift
      ;;
    --keep-staging)
      KEEP_STAGING=1
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

if [[ -z "$LANE" ]]; then
  echo "--lane is required" >&2
  usage >&2
  exit 2
fi
if (( PREPARE_ONLY == 1 && CHECK_ONLY == 1 )); then
  echo "--prepare-only and --check-only cannot be used together" >&2
  usage >&2
  exit 2
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo is required" >&2
  exit 1
fi
if ! command -v python3 >/dev/null 2>&1; then
  echo "python3 is required" >&2
  exit 1
fi
if ! python3 - <<'PY' >/dev/null 2>&1
import sys, tomllib
if sys.version_info < (3, 11):
    raise SystemExit(1)
PY
then
  echo "python3 >= 3.11 with tomllib is required to rewrite helper manifests" >&2
  exit 1
fi

read_metadata_field() {
  local field="$1"
  python3 - "$ROOT/Cargo.toml" "$LANE" "$field" <<'PY'
import sys, tomllib
from pathlib import Path
cargo_path = Path(sys.argv[1])
lane = sys.argv[2]
field = sys.argv[3]
doc = tomllib.loads(cargo_path.read_text())
helpers = doc.get("workspace", {}).get("metadata", {}).get("uc-native-toolchain-helpers", {})
if not isinstance(helpers, dict):
    print("helper lane metadata is missing from workspace Cargo.toml", file=sys.stderr)
    raise SystemExit(1)
try:
    entry = helpers[lane]
except KeyError:
    available = ", ".join(sorted(str(key) for key in helpers)) or "<none>"
    print(f"unsupported helper lane: {lane}. Available lanes: {available}", file=sys.stderr)
    raise SystemExit(1)
try:
    print(entry[field])
except KeyError:
    print(f"helper lane {lane} is missing metadata field: {field}", file=sys.stderr)
    raise SystemExit(1)
PY
}

read_metadata_field_optional() {
  local field="$1"
  python3 - "$ROOT/Cargo.toml" "$LANE" "$field" <<'PY'
import sys, tomllib
from pathlib import Path
cargo_path = Path(sys.argv[1])
lane = sys.argv[2]
field = sys.argv[3]
doc = tomllib.loads(cargo_path.read_text())
helpers = doc.get("workspace", {}).get("metadata", {}).get("uc-native-toolchain-helpers", {})
if not isinstance(helpers, dict):
    raise SystemExit(0)
entry = helpers.get(lane, {})
if isinstance(entry, dict) and field in entry:
    print(entry[field])
PY
}

CAIRO_VERSION="$(read_metadata_field cairo-version)"
SALSA_VERSION="$(read_metadata_field salsa-version)"
LOCKFILE_REL="$(read_metadata_field lockfile)"
PATCH_DIR_REL="$(read_metadata_field_optional patch-dir)"
LOCKFILE_PATH="$ROOT/$LOCKFILE_REL"
if [[ ! -f "$LOCKFILE_PATH" ]]; then
  echo "Missing helper lockfile: $LOCKFILE_PATH" >&2
  exit 1
fi

lane_digits="${LANE//./}"
HELPER_FEATURE="helper-cairo-${lane_digits}"
if [[ -z "$OUTPUT" ]]; then
  OUTPUT="$HOME/.uc/toolchain-helpers/uc-cairo${lane_digits}-helper/bin/uc"
fi
if [[ -z "$STAGING_DIR" ]]; then
  STAGING_DIR="$ROOT/.uc/toolchain-helper-builds/cairo-${LANE}-$(date +%Y%m%d-%H%M%S)-$$"
fi
if [[ -z "$TARGET_DIR" ]]; then
  TARGET_DIR="$ROOT/.uc/toolchain-helper-targets/cairo-${LANE}"
fi

cleanup() {
  if (( KEEP_STAGING == 0 )) && (( PREPARE_ONLY == 0 )) && (( STAGING_CREATED == 1 )) && [[ -d "$STAGING_DIR" ]]; then
    rm -rf "$STAGING_DIR"
  fi
}
trap cleanup EXIT

if [[ -e "$STAGING_DIR" ]]; then
  echo "staging dir already exists: $STAGING_DIR" >&2
  exit 1
fi
mkdir -p "$STAGING_DIR"
STAGING_CREATED=1

prepare_staging_tree() {
  local root_real staging_real stage_rel
  local -a tar_args
  root_real="$(cd "$ROOT" && pwd -P)"
  staging_real="$(cd "$STAGING_DIR" && pwd -P)"
  tar_args=(
    -C "$ROOT"
    --exclude='./.git'
    --exclude='./target'
    --exclude='./.uc'
    --exclude='./benchmarks/results'
  )
  if [[ "$staging_real" == "$root_real/"* ]]; then
    stage_rel="${staging_real#"$root_real"/}"
    tar_args+=(--exclude="./$stage_rel")
  fi
  tar "${tar_args[@]}" -cf - . | tar -C "$STAGING_DIR" -xf -
}

rewrite_workspace_manifest() {
  python3 - "$STAGING_DIR/Cargo.toml" "$CAIRO_VERSION" "$SALSA_VERSION" <<'PY'
import re, sys
from pathlib import Path
path = Path(sys.argv[1])
cairo_version = sys.argv[2]
salsa_version = sys.argv[3]
text = path.read_text()
for dep in [
    "cairo-lang-compiler",
    "cairo-lang-defs",
    "cairo-lang-filesystem",
    "cairo-lang-lowering",
    "cairo-lang-semantic",
    "cairo-lang-starknet",
    "cairo-lang-starknet-classes",
]:
    pattern = rf'^{re.escape(dep)}\s*=\s*".*"$'
    replacement = f'{dep} = "={cairo_version}"'
    text, count = re.subn(pattern, replacement, text, flags=re.MULTILINE)
    if count != 1:
        raise SystemExit(f"failed to rewrite {dep} in {path}")
text, count = re.subn(r'^salsa\s*=\s*".*"$', f'salsa = "{salsa_version}"', text, flags=re.MULTILINE)
if count != 1:
    raise SystemExit(f"failed to rewrite salsa in {path}")
text, count = re.subn(r'\n\[patch\.crates-io\]\n(?:.*\n)*?(?=\n\[|\Z)', '\n', text, flags=re.MULTILINE)
if count > 1:
    raise SystemExit(f"found multiple [patch.crates-io] sections in {path}")
path.write_text(text)
PY
}

helper_cargo_registry_src_root() {
  if [[ -n "${UC_HELPER_CARGO_REGISTRY_SRC:-}" ]]; then
    printf '%s\n' "$UC_HELPER_CARGO_REGISTRY_SRC"
  elif [[ -n "${CARGO_HOME:-}" ]]; then
    printf '%s\n' "$CARGO_HOME/registry/src"
  else
    printf '%s\n' "$HOME/.cargo/registry/src"
  fi
}

helper_locked_crate_version() {
  local crate_name="$1"
  python3 - "$LOCKFILE_PATH" "$crate_name" <<'PY'
import sys, tomllib
from pathlib import Path
lock_path = Path(sys.argv[1])
crate_name = sys.argv[2]
lock = tomllib.loads(lock_path.read_text())
versions = sorted(
    {
        package.get("version")
        for package in lock.get("package", [])
        if package.get("name") == crate_name and package.get("version")
    }
)
if len(versions) != 1:
    raise SystemExit(
        f"expected exactly one locked version for {crate_name} in {lock_path}, found {len(versions)}"
    )
print(versions[0])
PY
}

find_helper_crate_source() {
  local crate_name="$1"
  local crate_version="$2"
  local registry_src="$3"
  local -a matches=()
  while IFS= read -r candidate; do
    matches+=("$candidate")
  done < <(find "$registry_src" -mindepth 2 -maxdepth 2 -type d -name "${crate_name}-${crate_version}" | sort)
  if [[ "${#matches[@]}" -ne 1 ]]; then
    echo "expected exactly one registry source for ${crate_name}-${crate_version} under $registry_src, found ${#matches[@]}" >&2
    return 1
  fi
  printf '%s\n' "${matches[0]}"
}

append_helper_patch_section() {
  if [[ "${#PATCHED_CRATES[@]}" -eq 0 ]]; then
    return 0
  fi
  {
    printf '\n[patch.crates-io]\n'
    local crate_name
    for crate_name in "${PATCHED_CRATES[@]}"; do
      printf '%s = { path = ".uc/helper-lane-patches/cairo-%s/%s" }\n' \
        "$crate_name" "$LANE" "$crate_name"
    done
  } >> "$STAGING_DIR/Cargo.toml"
}

apply_helper_lane_patches() {
  if [[ -z "$PATCH_DIR_REL" ]]; then
    return 0
  fi
  local patch_dir="$ROOT/$PATCH_DIR_REL"
  if [[ ! -d "$patch_dir" ]]; then
    echo "helper lane $LANE patch-dir does not exist: $patch_dir" >&2
    return 1
  fi
  local -a patch_files=()
  while IFS= read -r patch_file; do
    patch_files+=("$patch_file")
  done < <(find "$patch_dir" -maxdepth 1 -type f -name '*.patch' | sort)
  if [[ "${#patch_files[@]}" -eq 0 ]]; then
    return 0
  fi
  if ! command -v patch >/dev/null 2>&1; then
    echo "patch is required to apply helper lane patches" >&2
    return 1
  fi
  local registry_src
  registry_src="$(helper_cargo_registry_src_root)"
  if [[ ! -d "$registry_src" ]]; then
    echo "cargo registry source root not found for helper lane patches: $registry_src" >&2
    return 1
  fi
  local patch_file crate_name crate_version crate_src crate_dst
  for patch_file in "${patch_files[@]}"; do
    crate_name="$(basename "$patch_file" .patch)"
    case "$crate_name" in
      cairo-lang-compiler|cairo-lang-defs|cairo-lang-filesystem|cairo-lang-lowering|cairo-lang-semantic|cairo-lang-starknet|cairo-lang-starknet-classes|cairo-lang-sierra-generator)
        ;;
      *)
        echo "unsupported helper lane patch crate: $crate_name" >&2
        return 1
        ;;
    esac
    crate_version="$(helper_locked_crate_version "$crate_name")"
    crate_src="$(find_helper_crate_source "$crate_name" "$crate_version" "$registry_src")"
    crate_dst="$STAGING_DIR/.uc/helper-lane-patches/cairo-${LANE}/${crate_name}"
    if [[ -e "$crate_dst" ]]; then
      echo "helper lane patch destination already exists: $crate_dst" >&2
      return 1
    fi
    mkdir -p "$crate_dst"
    tar -C "$crate_src" -cf - . | tar -C "$crate_dst" -xf -
    if ! (cd "$crate_dst" && patch -p1 --batch < "$patch_file"); then
      echo "failed to apply helper lane patch $patch_file to $crate_name" >&2
      return 1
    fi
    PATCHED_CRATES+=("$crate_name")
    printf 'Applied helper lane patch: %s -> %s\n' "$patch_file" "$crate_dst"
  done
  append_helper_patch_section
}

refresh_helper_lockfile_if_patched() {
  if [[ "${#PATCHED_CRATES[@]}" -eq 0 ]]; then
    return 0
  fi
  (
    cd "$STAGING_DIR"
    cargo metadata --format-version 1 >/dev/null
  )
  printf 'Refreshed helper staging Cargo.lock for patched crates\n'
}

prepare_staging_tree
rewrite_workspace_manifest
apply_helper_lane_patches
cp "$LOCKFILE_PATH" "$STAGING_DIR/Cargo.lock"
refresh_helper_lockfile_if_patched

if (( PREPARE_ONLY == 1 )); then
  printf 'Prepared helper staging tree: %s\n' "$STAGING_DIR"
  printf 'Lane: %s\n' "$LANE"
  printf 'Output target: %s\n' "$OUTPUT"
  printf 'Cargo target dir: %s\n' "$TARGET_DIR"
  exit 0
fi

mkdir -p "$TARGET_DIR"
if (( CHECK_ONLY == 1 )); then
  (
    cd "$STAGING_DIR"
    CARGO_TARGET_DIR="$TARGET_DIR" cargo test --locked --features "$HELPER_FEATURE" -p uc-cli \
      native_helper_cairo214_skip_unused_import_diagnostics_is_not_session_keyed -- --nocapture
    CARGO_TARGET_DIR="$TARGET_DIR" cargo test --locked --features "$HELPER_FEATURE" -p uc-cli \
      native_helper_cairo214_removed_unmodified_tracked_file_invalidates_cached_content -- --nocapture
    CARGO_TARGET_DIR="$TARGET_DIR" cargo test --locked --features "$HELPER_FEATURE" -p uc-cli \
      native_crate_cache_restore_preserves_existing_config_fields -- --nocapture
    CARGO_TARGET_DIR="$TARGET_DIR" cargo test --locked --features "$HELPER_FEATURE" -p uc-cli \
      native_apply_file_keyed_session_updates_skips_untracked_removed_file_slots -- --nocapture
  )
  printf 'Validated helper lane %s with targeted cargo tests\n' "$LANE"
  printf 'Cargo target dir: %s\n' "$TARGET_DIR"
  exit 0
fi

mkdir -p "$(dirname "$OUTPUT")"
(
  cd "$STAGING_DIR"
  CARGO_TARGET_DIR="$TARGET_DIR" cargo build --locked --release --features "$HELPER_FEATURE" --bin uc
)
cp "$TARGET_DIR/release/uc" "$OUTPUT"
chmod +x "$OUTPUT"

printf 'Built helper lane %s -> %s\n' "$LANE" "$OUTPUT"
printf 'Export with: export UC_NATIVE_TOOLCHAIN_%s_BIN=%q\n' "${LANE//./_}" "$OUTPUT"

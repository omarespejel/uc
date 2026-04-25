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

  grep -qF "Prepared helper staging tree:" "$stdout_path"
  grep -qF 'cairo-lang-compiler = "=2.14.0"' "$stage_dir/Cargo.toml"
  grep -qF 'salsa = "0.24.0"' "$stage_dir/Cargo.toml"
  grep -qF '[patch.crates-io]' "$stage_dir/Cargo.toml"
  grep -qF 'cairo-lang-lowering = { path = ".uc/helper-lane-patches/cairo-2.14/cairo-lang-lowering" }' \
    "$stage_dir/Cargo.toml"
  grep -qF 'cairo-lang-sierra-generator = { path = ".uc/helper-lane-patches/cairo-2.14/cairo-lang-sierra-generator" }' \
    "$stage_dir/Cargo.toml"
  grep -qF 'UC_CAIRO214_SIZE_TRACE_CONFIG' \
    "$stage_dir/.uc/helper-lane-patches/cairo-2.14/cairo-lang-lowering/src/db.rs"
  grep -qF 'UC_CAIRO214_SIZE_TRACE_MAX_KEYS' \
    "$stage_dir/.uc/helper-lane-patches/cairo-2.14/cairo-lang-lowering/src/db.rs"
  grep -qF 'UC_CAIRO214_SIZE_TRACE_WRITE_LOCK' \
    "$stage_dir/.uc/helper-lane-patches/cairo-2.14/cairo-lang-lowering/src/db.rs"
  grep -qF 'format!("{:p}:{:?}", db as *const dyn Database as *const (), function_id.get_internal_id())' \
    "$stage_dir/.uc/helper-lane-patches/cairo-2.14/cairo-lang-lowering/src/db.rs"
  grep -qF 'UC_CAIRO214_SIZE_TRACE_CONFIG' \
    "$stage_dir/.uc/helper-lane-patches/cairo-2.14/cairo-lang-sierra-generator/src/program_generator.rs"
  grep -qF 'UC_CAIRO214_SIZE_TRACE_MAX_KEYS' \
    "$stage_dir/.uc/helper-lane-patches/cairo-2.14/cairo-lang-sierra-generator/src/program_generator.rs"
  grep -qF 'UC_CAIRO214_SIZE_TRACE_WRITE_LOCK' \
    "$stage_dir/.uc/helper-lane-patches/cairo-2.14/cairo-lang-sierra-generator/src/program_generator.rs"
  grep -qF 'format!("{:p}:{:?}", db as *const dyn Database as *const (), function_id.get_internal_id())' \
    "$stage_dir/.uc/helper-lane-patches/cairo-2.14/cairo-lang-sierra-generator/src/program_generator.rs"
}

write_minimal_helper_repo_without_patch_section() {
  local repo_root="$1"
  mkdir -p "$repo_root/scripts" "$repo_root/toolchains/cairo-2.14"
  cp "$HELPER_SCRIPT" "$repo_root/scripts/build_native_toolchain_helper.sh"
  cp "$ROOT/toolchains/cairo-2.14/Cargo.lock" "$repo_root/toolchains/cairo-2.14/Cargo.lock"
  cat > "$repo_root/Cargo.toml" <<'TOML'
[workspace]
members = []
resolver = "2"

[workspace.dependencies]
cairo-lang-compiler = "2.16.0"
cairo-lang-defs = "2.16.0"
cairo-lang-filesystem = "2.16.0"
cairo-lang-lowering = "2.16.0"
cairo-lang-semantic = "2.16.0"
cairo-lang-starknet = "2.16.0"
cairo-lang-starknet-classes = "2.16.0"
salsa = "0.26.0"

[workspace.metadata.uc-native-toolchain-helpers."2.14"]
cairo-version = "2.14.0"
salsa-version = "0.24.0"
lockfile = "toolchains/cairo-2.14/Cargo.lock"
TOML
}

test_prepare_only_accepts_workspace_manifest_without_patch_section() {
  local fake_root="$TMP_DIR/no-patch-root"
  local stage_dir="$TMP_DIR/no-patch-stage"
  local stdout_path="$TMP_DIR/no-patch.out"
  write_minimal_helper_repo_without_patch_section "$fake_root"

  "$fake_root/scripts/build_native_toolchain_helper.sh" \
    --lane 2.14 \
    --staging-dir "$stage_dir" \
    --prepare-only >"$stdout_path"

  grep -qF "Prepared helper staging tree:" "$stdout_path"
  grep -qF 'cairo-lang-compiler = "=2.14.0"' "$stage_dir/Cargo.toml"
  grep -qF 'salsa = "0.24.0"' "$stage_dir/Cargo.toml"
  if grep -qF '[patch.crates-io]' "$stage_dir/Cargo.toml"; then
    echo "unexpected [patch.crates-io] section in no-patch helper staging Cargo.toml" >&2
    return 1
  fi
}

test_prepare_only_excludes_in_repo_staging_dir_from_archive() {
  local stage_dir="$ROOT/.tmp-helper-inrepo-stage-$$"
  local stdout_path="$TMP_DIR/inrepo-stage.out"
  rm -rf "$stage_dir"

  if ! "$HELPER_SCRIPT" --lane 2.14 --staging-dir "$stage_dir" --prepare-only >"$stdout_path"; then
    rm -rf "$stage_dir"
    return 1
  fi

  if ! grep -qF "Prepared helper staging tree:" "$stdout_path"; then
    rm -rf "$stage_dir"
    return 1
  fi
  if [[ -e "$stage_dir/$(basename "$stage_dir")" ]]; then
    echo "in-repo staging dir was copied into itself" >&2
    rm -rf "$stage_dir"
    return 1
  fi
  rm -rf "$stage_dir"
}

write_helper_repo_with_lane_patch() {
  local repo_root="$1"
  local registry_src="$2"
  write_minimal_helper_repo_without_patch_section "$repo_root"
  python3 - "$repo_root/Cargo.toml" <<'PY'
import sys
from pathlib import Path
path = Path(sys.argv[1])
text = path.read_text()
text = text.replace("members = []", 'members = ["probe"]')
path.write_text(text)
PY
  cat >> "$repo_root/Cargo.toml" <<'TOML'
patch-dir = "toolchains/cairo-2.14/patches"
TOML
  mkdir -p "$repo_root/probe/src"
  cat > "$repo_root/probe/Cargo.toml" <<'TOML'
[package]
name = "probe"
version = "0.0.0"
edition = "2021"

[dependencies]
cairo-lang-compiler = { workspace = true }
TOML
  printf 'pub fn probe() {}\n' > "$repo_root/probe/src/lib.rs"
  mkdir -p "$repo_root/toolchains/cairo-2.14/patches"
  cat > "$repo_root/toolchains/cairo-2.14/patches/cairo-lang-compiler.patch" <<'PATCH'
diff --git a/README.md b/README.md
--- a/README.md
+++ b/README.md
@@ -1 +1 @@
-original helper crate source
+patched helper crate source
PATCH
  mkdir -p "$registry_src/fake-index/cairo-lang-compiler-2.14.0/src"
  cat > "$registry_src/fake-index/cairo-lang-compiler-2.14.0/Cargo.toml" <<'TOML'
[package]
name = "cairo-lang-compiler"
version = "2.14.0"
edition = "2021"
TOML
  printf 'pub fn fake_compiler() {}\n' > "$registry_src/fake-index/cairo-lang-compiler-2.14.0/src/lib.rs"
  printf 'original helper crate source\n' > "$registry_src/fake-index/cairo-lang-compiler-2.14.0/README.md"
}

test_prepare_only_applies_helper_lane_patches_from_registry_source() {
  local fake_root="$TMP_DIR/patch-root"
  local fake_registry_src="$TMP_DIR/fake-registry/src"
  local stage_dir="$TMP_DIR/patch-stage"
  local stdout_path="$TMP_DIR/patch.out"
  write_helper_repo_with_lane_patch "$fake_root" "$fake_registry_src"

  UC_HELPER_CARGO_REGISTRY_SRC="$fake_registry_src" \
    "$fake_root/scripts/build_native_toolchain_helper.sh" \
      --lane 2.14 \
      --staging-dir "$stage_dir" \
      --prepare-only >"$stdout_path"

  grep -qF "Applied helper lane patch:" "$stdout_path"
  grep -qF "Refreshed helper staging Cargo.lock for patched crates" "$stdout_path"
  grep -qF 'patched helper crate source' \
    "$stage_dir/.uc/helper-lane-patches/cairo-2.14/cairo-lang-compiler/README.md"
  grep -qF '[patch.crates-io]' "$stage_dir/Cargo.toml"
  grep -qF 'cairo-lang-compiler = { path = ".uc/helper-lane-patches/cairo-2.14/cairo-lang-compiler" }' \
    "$stage_dir/Cargo.toml"
  (cd "$stage_dir" && cargo metadata --locked --format-version 1 >/dev/null)
}

test_prepare_only_and_check_only_are_mutually_exclusive() {
  local stdout_path="$TMP_DIR/mutually-exclusive.out"
  if "$HELPER_SCRIPT" --lane 2.14 --prepare-only --check-only >"$stdout_path" 2>&1; then
    echo "expected mutually exclusive helper modes to fail" >&2
    return 1
  fi
  grep -qF -- '--prepare-only and --check-only cannot be used together' "$stdout_path"
}

test_unsupported_lane_reports_actionable_error() {
  local stdout_path="$TMP_DIR/unsupported-lane.out"
  if "$HELPER_SCRIPT" --lane 9.99 --prepare-only >"$stdout_path" 2>&1; then
    echo "expected unsupported helper lane to fail" >&2
    return 1
  fi
  grep -qF 'unsupported helper lane: 9.99' "$stdout_path"
  grep -qF 'Available lanes: 2.14' "$stdout_path"
  if grep -qF 'Traceback' "$stdout_path"; then
    echo "unsupported helper lane should not emit a Python traceback" >&2
    cat "$stdout_path" >&2
    return 1
  fi
}

test_existing_staging_dir_is_not_removed_on_failure() {
  local stage_dir="$TMP_DIR/preexisting-stage"
  local stdout_path="$TMP_DIR/preexisting-stage.out"
  mkdir -p "$stage_dir"
  printf 'do not delete\n' > "$stage_dir/sentinel.txt"

  if "$HELPER_SCRIPT" --lane 2.14 --staging-dir "$stage_dir" --check-only >"$stdout_path" 2>&1; then
    echo "expected pre-existing staging dir to fail" >&2
    return 1
  fi
  grep -qF 'staging dir already exists:' "$stdout_path"
  if [[ ! -f "$stage_dir/sentinel.txt" ]]; then
    echo "pre-existing staging dir was removed by cleanup trap" >&2
    return 1
  fi
}

run_test "prepare_only_rewrites_workspace_manifest_for_cairo214" \
  test_prepare_only_rewrites_workspace_manifest_for_cairo214
run_test "prepare_only_accepts_workspace_manifest_without_patch_section" \
  test_prepare_only_accepts_workspace_manifest_without_patch_section
run_test "prepare_only_excludes_in_repo_staging_dir_from_archive" \
  test_prepare_only_excludes_in_repo_staging_dir_from_archive
run_test "prepare_only_applies_helper_lane_patches_from_registry_source" \
  test_prepare_only_applies_helper_lane_patches_from_registry_source
run_test "prepare_only_and_check_only_are_mutually_exclusive" \
  test_prepare_only_and_check_only_are_mutually_exclusive
run_test "unsupported_lane_reports_actionable_error" \
  test_unsupported_lane_reports_actionable_error
run_test "existing_staging_dir_is_not_removed_on_failure" \
  test_existing_staging_dir_is_not_removed_on_failure

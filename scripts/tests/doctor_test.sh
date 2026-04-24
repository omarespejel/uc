#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
ROOT="$(cd "$SCRIPT_DIR/../.." && pwd -P)"
DOCTOR_SCRIPT="$ROOT/scripts/doctor.sh"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

run_test() {
  echo "[test] $1"
  shift
  "$@"
}

write_mock_uc_bin() {
  local path="$1"
  cat > "$path" <<'MOCK'
#!/usr/bin/env bash
set -euo pipefail
manifest=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --manifest-path)
      manifest="$2"
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done
if [[ "$manifest" == *"helper-missing"* ]]; then
  printf '{"manifest_path":"%s","status":"unsupported","supported":false,"reason":"native compile requires a Cairo 2.14.0 helper lane, but `UC_NATIVE_TOOLCHAIN_2_14_BIN` is unset","issue_kind":"missing_toolchain_helper","diagnostics":[{"code":"UCN1004","category":"toolchain_lane_unavailable","severity":"error","title":"Required native toolchain helper is missing","what_happened":"The project requires native Cairo 2.14.0, but `UC_NATIVE_TOOLCHAIN_2_14_BIN` is not configured.","why":"native compile requires a Cairo 2.14.0 helper lane, but `UC_NATIVE_TOOLCHAIN_2_14_BIN` is unset","how_to_fix":["Set `UC_NATIVE_TOOLCHAIN_2_14_BIN` to the path of a uc binary built with the required cairo-lang lane."],"retryable":false,"fallback_used":false,"toolchain_expected":"2.14.0","toolchain_found":null}]}
' "$manifest"
else
  printf '{"manifest_path":"%s","status":"supported","supported":true,"compiler_version":"2.16.0","diagnostics":[]}
' "$manifest"
fi
MOCK
  chmod +x "$path"
}

write_manifest() {
  local root="$1"
  local name="$2"
  mkdir -p "$root/$name"
  cat > "$root/$name/Scarb.toml" <<MANIFEST
[package]
name = "$name"
version = "0.1.0"
edition = "2024_07"
MANIFEST
}

test_doctor_manifest_probe_fails_for_missing_helper_lane() {
  local cases_root="$TMP_DIR/cases"
  local mock_bin_dir="$TMP_DIR/mock-bin"
  local mock_uc="$mock_bin_dir/uc"
  mkdir -p "$mock_bin_dir"
  write_mock_uc_bin "$mock_uc"
  write_manifest "$cases_root" supported
  write_manifest "$cases_root" helper-missing

  local stdout_path="$TMP_DIR/doctor.out"
  if "$DOCTOR_SCRIPT" --uc-bin "$mock_uc" \
    --manifest-path "$cases_root/supported/Scarb.toml" \
    --manifest-path "$cases_root/helper-missing/Scarb.toml" >"$stdout_path" 2>&1; then
    echo "expected doctor to fail when helper lane is missing" >&2
    return 1
  fi
  grep -q '\[ok\] native support .*supported/Scarb.toml' "$stdout_path"
  grep -q '\[missing\] native support .*helper-missing/Scarb.toml.*UCN1004' "$stdout_path"
  grep -q 'UC_NATIVE_TOOLCHAIN_2_14_BIN' "$stdout_path"
}

test_doctor_requires_python_tomllib() {
  local fake_bin_dir="$TMP_DIR/fake-python-bin"
  local stdout_path="$TMP_DIR/python-tomllib.out"
  mkdir -p "$fake_bin_dir"
  cat > "$fake_bin_dir/python3" <<'PYTHON'
#!/usr/bin/env bash
exit 1
PYTHON
  chmod +x "$fake_bin_dir/python3"

  if PATH="$fake_bin_dir:$PATH" "$DOCTOR_SCRIPT" >"$stdout_path" 2>&1; then
    echo "expected doctor to fail when python3 lacks tomllib support" >&2
    return 1
  fi
  grep -q 'python3 >= 3.11 with tomllib is required for native helper builds' "$stdout_path"
}

run_test "doctor_manifest_probe_fails_for_missing_helper_lane" \
  test_doctor_manifest_probe_fails_for_missing_helper_lane
run_test "doctor_requires_python_tomllib" \
  test_doctor_requires_python_tomllib

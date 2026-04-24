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
elif [[ "$manifest" == *"/malformed-json/"* ]]; then
  printf '{"manifest_path":'
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

link_required_host_tools() {
  local fake_bin_dir="$1"
  for cmd in bash env head sort python3; do
    local resolved
    resolved="$(command -v "$cmd" || true)"
    if [[ -z "$resolved" ]]; then
      echo "required host command not found for test sandbox: $cmd" >&2
      return 1
    fi
    ln -s "$resolved" "$fake_bin_dir/$cmd"
  done
}

write_version_stub() {
  local path="$1"
  local body="$2"
  cat > "$path" <<STUB
#!/usr/bin/env bash
printf '%s\n' "$body"
STUB
  chmod +x "$path"
}

write_git_hooks_stub() {
  local path="$1"
  cat > "$path" <<'GITMOCK'
#!/usr/bin/env bash
if [[ "$1" == "config" && "$2" == "--get" && "$3" == "core.hooksPath" ]]; then
  printf '.githooks\n'
fi
GITMOCK
  chmod +x "$path"
}

write_required_tool_stubs() {
  local fake_bin_dir="$1"
  write_version_stub "$fake_bin_dir/cargo" "cargo 1.88.0"
  write_version_stub "$fake_bin_dir/rustc" "rustc 1.88.0"
  write_version_stub "$fake_bin_dir/scarb" "scarb 2.16.0"
  write_version_stub "$fake_bin_dir/rg" "ripgrep 14.1.1"
}

test_doctor_manifest_probe_reports_missing_jq_without_aborting() {
  local cases_root="$TMP_DIR/jq-cases"
  local fake_bin_dir="$TMP_DIR/no-jq-bin"
  local mock_uc="$fake_bin_dir/uc"
  local stdout_path="$TMP_DIR/no-jq.out"
  mkdir -p "$fake_bin_dir"
  write_manifest "$cases_root" supported
  write_mock_uc_bin "$mock_uc"

  link_required_host_tools "$fake_bin_dir"
  write_required_tool_stubs "$fake_bin_dir"
  write_git_hooks_stub "$fake_bin_dir/git"

  if PATH="$fake_bin_dir" "$DOCTOR_SCRIPT" \
    --uc-bin "$mock_uc" \
    --manifest-path "$cases_root/supported/Scarb.toml" >"$stdout_path" 2>&1; then
    echo "expected doctor to fail when jq is unavailable for manifest probe" >&2
    return 1
  fi
  grep -q '\[missing\] jq' "$stdout_path"
  grep -q '\[missing\] jq is required for manifest probe:' "$stdout_path"
  grep -q 'doctor failed:' "$stdout_path"
}

test_doctor_skips_tomllib_probe_when_python3_missing() {
  local fake_bin_dir="$TMP_DIR/no-python-bin"
  local stdout_path="$TMP_DIR/no-python.out"
  mkdir -p "$fake_bin_dir"
  for cmd in bash env head sort; do
    local resolved
    resolved="$(command -v "$cmd" || true)"
    if [[ -z "$resolved" ]]; then
      echo "required host command not found for test sandbox: $cmd" >&2
      return 1
    fi
    ln -s "$resolved" "$fake_bin_dir/$cmd"
  done
  write_required_tool_stubs "$fake_bin_dir"
  write_version_stub "$fake_bin_dir/jq" "jq-1.7"
  write_git_hooks_stub "$fake_bin_dir/git"

  if PATH="$fake_bin_dir" "$DOCTOR_SCRIPT" >"$stdout_path" 2>&1; then
    echo "expected doctor to fail when python3 is unavailable" >&2
    return 1
  fi
  grep -q '\[missing\] python3' "$stdout_path"
  grep -q '\[skip\] python3 tomllib support check skipped because python3 is unavailable' "$stdout_path"
  if grep -q 'python3 >= 3.11 with tomllib is required' "$stdout_path"; then
    echo "tomllib version check should not run when python3 is unavailable" >&2
    return 1
  fi
}

test_doctor_manifest_probe_reports_invalid_json_without_aborting() {
  local cases_root="$TMP_DIR/malformed-json-cases"
  local mock_bin_dir="$TMP_DIR/malformed-json-bin"
  local mock_uc="$mock_bin_dir/uc"
  local stdout_path="$TMP_DIR/malformed-json.out"
  mkdir -p "$mock_bin_dir"
  write_manifest "$cases_root" supported
  write_manifest "$cases_root" malformed-json
  write_mock_uc_bin "$mock_uc"

  if "$DOCTOR_SCRIPT" --uc-bin "$mock_uc" \
    --manifest-path "$cases_root/supported/Scarb.toml" \
    --manifest-path "$cases_root/malformed-json/Scarb.toml" >"$stdout_path" 2>&1; then
    echo "expected doctor to fail when uc support native returns invalid JSON" >&2
    return 1
  fi
  grep -q '\[ok\] native support .*supported/Scarb.toml' "$stdout_path"
  grep -q '\[missing\] native support probe returned invalid JSON for .*malformed-json/Scarb.toml' "$stdout_path"
  grep -q 'doctor failed:' "$stdout_path"
}

run_test "doctor_manifest_probe_fails_for_missing_helper_lane" \
  test_doctor_manifest_probe_fails_for_missing_helper_lane
run_test "doctor_requires_python_tomllib" \
  test_doctor_requires_python_tomllib
run_test "doctor_manifest_probe_reports_missing_jq_without_aborting" \
  test_doctor_manifest_probe_reports_missing_jq_without_aborting
run_test "doctor_skips_tomllib_probe_when_python3_missing" \
  test_doctor_skips_tomllib_probe_when_python3_missing
run_test "doctor_manifest_probe_reports_invalid_json_without_aborting" \
  test_doctor_manifest_probe_reports_invalid_json_without_aborting

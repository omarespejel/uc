#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
BENCH_SCRIPT="$SCRIPT_DIR/../run_real_repo_benchmarks.sh"

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
if [[ "$1" == "support" && "$2" == "native" ]]; then
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
  printf 'build %s disallow=%s corelib=%s report=%s\n' "$manifest" "${UC_NATIVE_DISALLOW_SCARB_FALLBACK:-}" "${UC_NATIVE_CORELIB_SRC:-}" "$report_path" >> "$args_log"
  if [[ "$manifest" == *"unstable"* && "${UC_NATIVE_DISALLOW_SCARB_FALLBACK:-}" == "1" ]]; then
    state_dir="${MOCK_UC_STATE_DIR:-}"
    if [[ -n "$state_dir" ]]; then
      mkdir -p "$state_dir"
      case_tag="$(basename "$(dirname "$manifest")")-$(basename "$manifest")-$(printf '%s' "$manifest" | cksum)"
      case_tag="${case_tag//[^A-Za-z0-9_.-]/_}"
      count_file="$state_dir/${case_tag}.count"
      count=0
      if [[ -f "$count_file" ]]; then
        count="$(cat "$count_file")"
      fi
      count=$((count + 1))
      printf '%s' "$count" > "$count_file"
      if [[ "$count" -eq 4 ]]; then
        sleep 1.5
      fi
    fi
  fi
  if [[ -n "$report_path" ]]; then
    mkdir -p "$(dirname "$report_path")"
    compile_backend="uc_native"
    fallback_used="false"
    diagnostics='[]'
    if [[ "$manifest" == *"fallback-used"* ]]; then
      compile_backend="uc_native_external_helper"
      fallback_used="true"
      diagnostics='[{"code":"UCN2002","category":"native_fallback_local_native_error","severity":"warn","title":"Native local build downgraded to Scarb","what_happened":"native failed","why":"native failed","how_to_fix":["fix native"],"retryable":true,"fallback_used":true,"toolchain_expected":"2.16.0","toolchain_found":"2.16.0"}]'
    fi
    if [[ "$manifest" == *"fallback-backend-only"* ]]; then
      compile_backend="scarb_fallback"
    fi
  cat > "$report_path" <<REPORT
{
  "generated_at_epoch_ms": 1,
  "engine": "uc",
  "daemon_used": false,
  "manifest_path": "$manifest",
  "workspace_root": "$(dirname "$manifest")",
  "profile": "dev",
  "session_key": "session-$manifest",
  "command": ["uc","build"],
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
  if [[ -z "$report_path" && "${UC_NATIVE_DISALLOW_SCARB_FALLBACK:-}" != "1" ]]; then
    echo "expected strict uc benchmark build to set UC_NATIVE_DISALLOW_SCARB_FALLBACK=1" >&2
    exit 22
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
if [[ "$subcommand" == "fetch" && "$manifest" == *"fails-fetch"* ]]; then
  echo "forced scarb fetch failure for $manifest" >&2
  exit 18
fi
if [[ "$subcommand" == "build" && "$manifest" == *"fails-build"* ]]; then
  echo "forced scarb build failure for $manifest" >&2
  exit 17
fi
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

test_real_repo_benchmark_rejects_missing_case_values() {
  local stderr_path="$TEST_TMP_DIR/missing-case.err"
  if "$BENCH_SCRIPT" --case >"$TEST_TMP_DIR/missing-case.out" 2>"$stderr_path"; then
    echo "expected real repo benchmark script to reject missing --case values" >&2
    return 1
  fi
  if ! grep -q "Usage:" "$stderr_path"; then
    echo "expected usage output for missing --case values" >&2
    cat "$stderr_path" >&2
    return 1
  fi
}

test_real_repo_benchmark_rejects_zero_runs_from_environment() {
  local cases_root="$TEST_TMP_DIR/env-cases"
  local mock_bin_dir="$TEST_TMP_DIR/env-mock-bin"
  local mock_uc="$mock_bin_dir/uc"
  local mock_scarb="$mock_bin_dir/scarb"
  local stderr_path="$TEST_TMP_DIR/env-zero-runs.err"
  mkdir -p "$mock_bin_dir"
  write_mock_uc_bin "$mock_uc"
  write_mock_scarb_bin "$mock_scarb"
  write_manifest_case "$cases_root" "supported"

  if RUNS=0 PATH="$mock_bin_dir:$PATH" "$BENCH_SCRIPT" \
    --uc-bin "$mock_uc" \
    --results-dir "$TEST_TMP_DIR/env-results" \
    --case "$cases_root/supported/Scarb.toml" supported \
    >"$TEST_TMP_DIR/env-zero-runs.out" 2>"$stderr_path"; then
    echo "expected real repo benchmark script to reject RUNS=0 from environment" >&2
    return 1
  fi
  if ! grep -q "RUNS must be a positive integer" "$stderr_path"; then
    echo "expected explicit RUNS validation failure" >&2
    cat "$stderr_path" >&2
    return 1
  fi
}

test_real_repo_benchmark_rejects_no_cases_with_updated_usage() {
  local stderr_path="$TEST_TMP_DIR/no-cases.err"
  if "$BENCH_SCRIPT" >"$TEST_TMP_DIR/no-cases.out" 2>"$stderr_path"; then
    echo "expected real repo benchmark script to reject missing cases" >&2
    return 1
  fi
  if ! grep -q "requires at least one case via --case or --cases-file" "$stderr_path"; then
    echo "expected updated no-cases validation message" >&2
    cat "$stderr_path" >&2
    return 1
  fi
  if ! grep -q "Provide at least one case via --case or --cases-file" "$stderr_path"; then
    echo "expected updated no-cases usage guidance" >&2
    cat "$stderr_path" >&2
    return 1
  fi
}

test_real_repo_benchmark_rejects_directory_uc_bin() {
  local stderr_path="$TEST_TMP_DIR/directory-uc-bin.err"
  if "$BENCH_SCRIPT" \
    --uc-bin "$TEST_TMP_DIR" \
    --case /tmp/Scarb.toml supported \
    >"$TEST_TMP_DIR/directory-uc-bin.out" 2>"$stderr_path"; then
    echo "expected real repo benchmark script to reject directory --uc-bin" >&2
    return 1
  fi
  if ! grep -q "UC binary is missing or not a regular file" "$stderr_path"; then
    echo "expected regular-file validation for --uc-bin" >&2
    cat "$stderr_path" >&2
    return 1
  fi
}

test_real_repo_benchmark_accepts_cases_file() {
  local cases_root="$TEST_TMP_DIR/cases-file-cases"
  local mock_bin_dir="$TEST_TMP_DIR/cases-file-mock-bin"
  local mock_uc="$mock_bin_dir/uc"
  local mock_scarb="$mock_bin_dir/scarb"
  local results_dir="$TEST_TMP_DIR/cases-file-results"
  local cases_file="$TEST_TMP_DIR/cases-file.tsv"
  mkdir -p "$mock_bin_dir" "$results_dir"
  write_mock_uc_bin "$mock_uc"
  write_mock_scarb_bin "$mock_scarb"
  write_manifest_case "$cases_root" "supported"
  write_manifest_case "$cases_root" "unsupported"
  printf '%s\t%s\n' "$cases_root/supported/Scarb.toml" supported > "$cases_file"
  printf '%s\t%s\n' "$cases_root/unsupported/Scarb.toml" unsupported >> "$cases_file"

  local stdout_text
  stdout_text="$(
    PATH="$mock_bin_dir:$PATH" \
    MOCK_UC_ARGS_LOG="$TEST_TMP_DIR/cases-file-uc.args" \
    MOCK_SCARB_ARGS_LOG="$TEST_TMP_DIR/cases-file-scarb.args" \
    "$BENCH_SCRIPT" \
      --uc-bin "$mock_uc" \
      --results-dir "$results_dir" \
      --runs 1 \
      --cold-runs 1 \
      --warm-settle-seconds 0 \
      --cases-file "$cases_file"
  )"

  local json_path
  json_path="$(awk -F': ' '/Benchmark JSON:/ {print $2}' <<<"$stdout_text")"
  [[ -f "$json_path" ]] || { echo "missing json report: $json_path" >&2; return 1; }

  local supported_count unsupported_count
  supported_count="$(jq -r '.summary.support_matrix.native_supported' "$json_path")"
  unsupported_count="$(jq -r '.summary.support_matrix.native_unsupported' "$json_path")"
  if [[ "$supported_count" != "1" || "$unsupported_count" != "1" ]]; then
    echo "expected cases file to populate support matrix" >&2
    cat "$json_path" >&2
    return 1
  fi
}

test_real_repo_benchmark_canonicalizes_relative_paths() {
  local work_dir="$TEST_TMP_DIR/relative-paths-work"
  local cases_root="$work_dir/cases"
  local mock_bin_dir="$work_dir/bin"
  local results_dir="$work_dir/results"
  local corelib_dir="$work_dir/corelib/src"
  mkdir -p "$mock_bin_dir" "$results_dir" "$corelib_dir"
  write_mock_uc_bin "$mock_bin_dir/uc"
  write_mock_scarb_bin "$mock_bin_dir/scarb"
  write_manifest_case "$cases_root" "supported"

  local stdout_text
  stdout_text="$(
    cd "$work_dir"
    PATH="$mock_bin_dir:$PATH" \
    MOCK_UC_ARGS_LOG="$TEST_TMP_DIR/relative-paths-uc.args" \
    MOCK_SCARB_ARGS_LOG="$TEST_TMP_DIR/relative-paths-scarb.args" \
    UC_NATIVE_CORELIB_SRC=corelib/src \
    "$BENCH_SCRIPT" \
      --uc-bin bin/uc \
      --results-dir results \
      --runs 1 \
      --cold-runs 1 \
      --warm-settle-seconds 0 \
      --case cases/supported/Scarb.toml supported
  )"

  local json_path
  json_path="$(awk -F': ' '/Benchmark JSON:/ {print $2}' <<<"$stdout_text")"
  [[ -f "$json_path" ]] || { echo "missing json report: $json_path" >&2; return 1; }

  local classification
  classification="$(jq -r '.cases[] | select(.tag=="supported") | .support_matrix.classification' "$json_path")"
  if [[ "$classification" != "native_supported" ]]; then
    echo "expected relative paths to preserve native-supported classification" >&2
    cat "$json_path" >&2
    return 1
  fi

  if grep -q 'report=results/' "$TEST_TMP_DIR/relative-paths-uc.args"; then
    echo "expected uc auto-build report path to be canonicalized before cwd changes" >&2
    cat "$TEST_TMP_DIR/relative-paths-uc.args" >&2
    return 1
  fi
  local canonical_results_dir
  canonical_results_dir="$(cd "$results_dir" && pwd -P)"
  if ! grep -q "report=$canonical_results_dir/real-repo-supported-uc-auto-build-report.json" "$TEST_TMP_DIR/relative-paths-uc.args"; then
    echo "expected absolute report path under the requested results directory" >&2
    cat "$TEST_TMP_DIR/relative-paths-uc.args" >&2
    return 1
  fi
  local canonical_corelib_dir
  canonical_corelib_dir="$(cd "$corelib_dir" && pwd -P)"
  if ! grep -q "corelib=$canonical_corelib_dir" "$TEST_TMP_DIR/relative-paths-uc.args"; then
    echo "expected relative UC_NATIVE_CORELIB_SRC to be canonicalized before cwd changes" >&2
    cat "$TEST_TMP_DIR/relative-paths-uc.args" >&2
    return 1
  fi
}

test_real_repo_benchmark_rejects_malformed_cases_file_rows() {
  local cases_file="$TEST_TMP_DIR/malformed-cases-file.tsv"
  local stderr_path="$TEST_TMP_DIR/malformed-cases-file.err"
  local -a rows=(
    $'/tmp/Scarb.toml\ttag\t'
    $'/tmp/Scarb.toml\ttag\textra'
    $'/tmp/Scarb.toml'
    $'\ttag'
    $'/tmp/Scarb.toml\t'
  )

  local index
  for index in "${!rows[@]}"; do
    printf '%s\n' "${rows[$index]}" > "$cases_file"
    if "$BENCH_SCRIPT" --cases-file "$cases_file" >"$TEST_TMP_DIR/malformed-cases-file-$index.out" 2>"$stderr_path"; then
      echo "expected malformed cases-file row to be rejected: ${rows[$index]}" >&2
      return 1
    fi
    if ! grep -q "Invalid cases file row 1" "$stderr_path"; then
      echo "expected malformed cases-file row validation error for: ${rows[$index]}" >&2
      cat "$stderr_path" >&2
      return 1
    fi
  done
}

test_real_repo_benchmark_records_support_matrix_categories() {
  local cases_root="$TEST_TMP_DIR/cases"
  local mock_bin_dir="$TEST_TMP_DIR/mock-bin"
  local mock_uc="$mock_bin_dir/uc"
  local mock_scarb="$mock_bin_dir/scarb"
  local results_dir="$TEST_TMP_DIR/results"
  local corelib_dir="$TEST_TMP_DIR/fake-corelib/src"
  mkdir -p "$mock_bin_dir" "$results_dir" "$corelib_dir"
  write_mock_uc_bin "$mock_uc"
  write_mock_scarb_bin "$mock_scarb"
  write_manifest_case "$cases_root" "supported"
  write_manifest_case "$cases_root" "fallback-used"
  write_manifest_case "$cases_root" "fallback-backend-only"
  write_manifest_case "$cases_root" "unsupported"

  local stdout_text
  stdout_text="$(
    PATH="$mock_bin_dir:$PATH" \
    UC_NATIVE_CORELIB_SRC="$corelib_dir" \
    MOCK_UC_ARGS_LOG="$TEST_TMP_DIR/uc.args" \
    MOCK_SCARB_ARGS_LOG="$TEST_TMP_DIR/scarb.args" \
    "$BENCH_SCRIPT" \
      --uc-bin "$mock_uc" \
      --results-dir "$results_dir" \
      --runs 1 \
      --cold-runs 1 \
      --warm-settle-seconds 0 \
      --case "$cases_root/supported/Scarb.toml" supported \
      --case "$cases_root/fallback-used/Scarb.toml" fallback-used \
      --case "$cases_root/fallback-backend-only/Scarb.toml" fallback-backend-only \
      --case "$cases_root/unsupported/Scarb.toml" unsupported
  )"
  assert_contains "$stdout_text" "Benchmark JSON:"
  assert_contains "$stdout_text" "Benchmark Markdown:"

  local json_path
  json_path="$(awk -F': ' '/Benchmark JSON:/ {print $2}' <<<"$stdout_text")"
  local md_path
  md_path="$(awk -F': ' '/Benchmark Markdown:/ {print $2}' <<<"$stdout_text")"
  [[ -f "$json_path" ]] || { echo "missing json report: $json_path" >&2; return 1; }
  [[ -f "$md_path" ]] || { echo "missing markdown report: $md_path" >&2; return 1; }

  local supported_status
  supported_status="$(jq -r '.cases[] | select(.tag=="supported") | .native_support.status' "$json_path")"
  local unsupported_status
  unsupported_status="$(jq -r '.cases[] | select(.tag=="unsupported") | .native_support.status' "$json_path")"
  local fallback_classification
  fallback_classification="$(jq -r '.cases[] | select(.tag=="fallback-used") | .support_matrix.classification' "$json_path")"
  local fallback_used_flag
  fallback_used_flag="$(jq -r '.cases[] | select(.tag=="fallback-used") | .support_matrix.fallback_used' "$json_path")"
  local fallback_backend_only_classification
  fallback_backend_only_classification="$(jq -r '.cases[] | select(.tag=="fallback-backend-only") | .support_matrix.classification' "$json_path")"
  local fallback_backend_only_flag
  fallback_backend_only_flag="$(jq -r '.cases[] | select(.tag=="fallback-backend-only") | .support_matrix.fallback_used' "$json_path")"
  local supported_benchmark_status
  supported_benchmark_status="$(jq -r '.cases[] | select(.tag=="supported") | .benchmark_status' "$json_path")"
  local unsupported_classification
  unsupported_classification="$(jq -r '.cases[] | select(.tag=="unsupported") | .support_matrix.classification' "$json_path")"
  local supported_classification
  supported_classification="$(jq -r '.cases[] | select(.tag=="supported") | .support_matrix.classification' "$json_path")"
  if [[ "$supported_status" != "supported" || "$unsupported_status" != "unsupported" || "$supported_benchmark_status" != "ok" || "$supported_classification" != "native_supported" || "$fallback_classification" != "fallback_used" || "$fallback_used_flag" != "true" || "$fallback_backend_only_classification" != "fallback_used" || "$fallback_backend_only_flag" != "true" || "$unsupported_classification" != "native_unsupported" ]]; then
    echo "unexpected support classification in json report" >&2
    cat "$json_path" >&2
    return 1
  fi

  local markdown_text
  markdown_text="$(cat "$md_path")"
  assert_contains "$markdown_text" "## Support Matrix"
  assert_contains "$markdown_text" "## Native-Supported Benchmark Cases"
  assert_contains "$markdown_text" "## Native-Supported Benchmark Cases With Build Failures"
  assert_contains "$markdown_text" "## Fallback-Used Cases"
  assert_contains "$markdown_text" "## Native-Unsupported Cases"
  assert_contains "$markdown_text" "| supported |"
  assert_contains "$markdown_text" "| fallback-used | fallback_used | uc_native_external_helper |"
  assert_contains "$markdown_text" "| fallback-backend-only | fallback_used | scarb_fallback |"
  assert_contains "$markdown_text" "| unsupported | native_unsupported | <none> | 2.14.0 |"

  local canonical_corelib_dir
  canonical_corelib_dir="$(cd "$corelib_dir" && pwd -P)"
  if ! grep -q "build .*supported.* disallow=1 corelib=$canonical_corelib_dir report=" "$TEST_TMP_DIR/uc.args"; then
    echo "expected supported case to run strict uc benchmark build" >&2
    cat "$TEST_TMP_DIR/uc.args" >&2
    return 1
  fi
  if ! grep -q "build .*supported.* report=.*/real-repo-supported-uc-auto-build-report.json" "$TEST_TMP_DIR/uc.args"; then
    echo "expected supported case to run uc auto-build classification" >&2
    cat "$TEST_TMP_DIR/uc.args" >&2
    return 1
  fi
  if ! grep -q "build .*fallback-used.* report=.*/real-repo-fallback-used-uc-auto-build-report.json" "$TEST_TMP_DIR/uc.args"; then
    echo "expected fallback-used case to run uc auto-build classification" >&2
    cat "$TEST_TMP_DIR/uc.args" >&2
    return 1
  fi
  if grep -q "build .*fallback-used.* disallow=1" "$TEST_TMP_DIR/uc.args"; then
    echo "fallback-used case should not run strict native benchmark builds" >&2
    cat "$TEST_TMP_DIR/uc.args" >&2
    return 1
  fi
  if grep -q "build .*fallback-backend-only.* disallow=1" "$TEST_TMP_DIR/uc.args"; then
    echo "fallback-backend-only case should not run strict native benchmark builds" >&2
    cat "$TEST_TMP_DIR/uc.args" >&2
    return 1
  fi
  if grep -q "build .*unsupported" "$TEST_TMP_DIR/uc.args"; then
    echo "unsupported case should not run uc build" >&2
    cat "$TEST_TMP_DIR/uc.args" >&2
    return 1
  fi

  if ! grep -Eq 'subcommand=build manifest=.*/Scarb.toml' "$TEST_TMP_DIR/scarb.args"; then
    echo "expected supported case to run scarb build with rewritten manifest path" >&2
    cat "$TEST_TMP_DIR/scarb.args" >&2
    return 1
  fi
}

test_real_repo_benchmark_records_supported_build_failures() {
  local cases_root="$TEST_TMP_DIR/failure-cases"
  local mock_bin_dir="$TEST_TMP_DIR/failure-mock-bin"
  local mock_uc="$mock_bin_dir/uc"
  local mock_scarb="$mock_bin_dir/scarb"
  local results_dir="$TEST_TMP_DIR/failure-results"
  mkdir -p "$mock_bin_dir" "$results_dir"
  write_mock_uc_bin "$mock_uc"
  write_mock_scarb_bin "$mock_scarb"
  write_manifest_case "$cases_root" "fails-build"

  local stdout_text
  stdout_text="$(
    PATH="$mock_bin_dir:$PATH" \
    MOCK_UC_ARGS_LOG="$TEST_TMP_DIR/failure-uc.args" \
    MOCK_SCARB_ARGS_LOG="$TEST_TMP_DIR/failure-scarb.args" \
    "$BENCH_SCRIPT" \
      --uc-bin "$mock_uc" \
      --results-dir "$results_dir" \
      --runs 1 \
      --cold-runs 1 \
      --warm-settle-seconds 0 \
      --case "$cases_root/fails-build/Scarb.toml" fails-build
  )"
  local json_path
  json_path="$(awk -F': ' '/Benchmark JSON:/ {print $2}' <<<"$stdout_text")"
  local md_path
  md_path="$(awk -F': ' '/Benchmark Markdown:/ {print $2}' <<<"$stdout_text")"

  local benchmark_status
  benchmark_status="$(jq -r '.cases[] | select(.tag=="fails-build") | .benchmark_status' "$json_path")"
  local cold_status
  cold_status="$(jq -r '.cases[] | select(.tag=="fails-build") | .benchmarks.scarb.build.cold.status' "$json_path")"
  local cold_exit_code
  cold_exit_code="$(jq -r '.cases[] | select(.tag=="fails-build") | .benchmarks.scarb.build.cold.exit_code' "$json_path")"
  if [[ "$benchmark_status" != "failed" || "$cold_status" != "failed" || "$cold_exit_code" != "17" ]]; then
    echo "expected supported build failure to be recorded in json report" >&2
    cat "$json_path" >&2
    return 1
  fi

  local markdown_text
  markdown_text="$(cat "$md_path")"
  assert_contains "$markdown_text" "## Native-Supported Benchmark Cases With Build Failures"
  assert_contains "$markdown_text" "| fails-build | scarb | build.cold | 17 |"
}

test_real_repo_benchmark_reports_prefetch_failure_context() {
  local cases_root="$TEST_TMP_DIR/prefetch-cases"
  local mock_bin_dir="$TEST_TMP_DIR/prefetch-mock-bin"
  local mock_uc="$mock_bin_dir/uc"
  local mock_scarb="$mock_bin_dir/scarb"
  local results_dir="$TEST_TMP_DIR/prefetch-results"
  local stderr_path="$TEST_TMP_DIR/prefetch.err"
  mkdir -p "$mock_bin_dir" "$results_dir"
  write_mock_uc_bin "$mock_uc"
  write_mock_scarb_bin "$mock_scarb"
  write_manifest_case "$cases_root" "fails-fetch"

  local stdout_text
  stdout_text="$(PATH="$mock_bin_dir:$PATH" \
    MOCK_UC_ARGS_LOG="$TEST_TMP_DIR/prefetch-uc.args" \
    MOCK_SCARB_ARGS_LOG="$TEST_TMP_DIR/prefetch-scarb.args" \
    "$BENCH_SCRIPT" \
      --uc-bin "$mock_uc" \
      --results-dir "$results_dir" \
      --runs 1 \
      --cold-runs 1 \
      --warm-settle-seconds 0 \
      --case "$cases_root/fails-fetch/Scarb.toml" fails-fetch \
      2>"$stderr_path")"

  local json_path
  json_path="$(awk -F': ' '/Benchmark JSON:/ {print $2}' <<<"$stdout_text")"
  local md_path
  md_path="$(awk -F': ' '/Benchmark Markdown:/ {print $2}' <<<"$stdout_text")"
  local expected_manifest
  expected_manifest="$(cd "$cases_root/fails-fetch" && pwd -P)/Scarb.toml"
  local classification
  classification="$(jq -r '.cases[] | select(.tag=="fails-fetch") | .support_matrix.classification' "$json_path")"
  local benchmark_status
  benchmark_status="$(jq -r '.cases[] | select(.tag=="fails-fetch") | .benchmark_status' "$json_path")"
  local exit_code
  exit_code="$(jq -r '.cases[] | select(.tag=="fails-fetch") | .support_matrix.exit_code' "$json_path")"
  local log_path
  log_path="$(jq -r '.cases[] | select(.tag=="fails-fetch") | .support_matrix.log_path' "$json_path")"
  local reason
  reason="$(jq -r '.cases[] | select(.tag=="fails-fetch") | .support_matrix.reason' "$json_path")"
  if [[ "$classification" != "build_failed" || "$benchmark_status" != "skipped" || "$exit_code" != "18" || "$reason" != "scarb offline fetch failed before uc auto build classification" ]]; then
    echo "expected prefetch failure to be recorded as a per-case build_failed support classification" >&2
    cat "$json_path" >&2
    return 1
  fi
  if [[ ! -f "$log_path" ]] || ! grep -q "forced scarb fetch failure" "$log_path"; then
    echo "expected underlying scarb fetch failure in per-case log" >&2
    cat "${log_path:-/dev/null}" >&2 || true
    return 1
  fi
  if ! grep -q "scarb fetch failed for manifest_path=$expected_manifest" "$stderr_path"; then
    echo "expected manifest-scoped scarb fetch failure context on stderr" >&2
    cat "$stderr_path" >&2
    return 1
  fi
  if ! grep -q "exit_code=18" "$stderr_path"; then
    echo "expected real scarb fetch exit code on stderr" >&2
    cat "$stderr_path" >&2
    return 1
  fi
  local markdown_text
  markdown_text="$(cat "$md_path")"
  if [[ "$markdown_text" != *"| fails-fetch | 18 | $log_path | scarb offline fetch failed before uc auto build classification |"* ]]; then
    echo "expected markdown auto-build classification row for prefetch failure" >&2
    cat "$md_path" >&2
    return 1
  fi
}

test_real_repo_benchmark_surfaces_stability_warnings() {
  local cases_root="$TEST_TMP_DIR/stability-cases"
  local mock_bin_dir="$TEST_TMP_DIR/stability-mock-bin"
  local mock_uc="$mock_bin_dir/uc"
  local mock_scarb="$mock_bin_dir/scarb"
  local results_dir="$TEST_TMP_DIR/stability-results"
  mkdir -p "$mock_bin_dir" "$results_dir"
  write_mock_uc_bin "$mock_uc"
  write_mock_scarb_bin "$mock_scarb"
  write_manifest_case "$cases_root" "unstable-supported"

  local stdout_text
  stdout_text="$(
    PATH="$mock_bin_dir:$PATH" \
    MOCK_UC_ARGS_LOG="$TEST_TMP_DIR/stability-uc.args" \
    MOCK_UC_STATE_DIR="$TEST_TMP_DIR/stability-uc.state" \
    MOCK_SCARB_ARGS_LOG="$TEST_TMP_DIR/stability-scarb.args" \
    "$BENCH_SCRIPT" \
      --uc-bin "$mock_uc" \
      --results-dir "$results_dir" \
      --runs 4 \
      --cold-runs 5 \
      --warm-settle-seconds 0 \
      --case "$cases_root/unstable-supported/Scarb.toml" unstable-supported
  )"
  local json_path
  json_path="$(awk -F': ' '/Benchmark JSON:/ {print $2}' <<<"$stdout_text")"
  local md_path
  md_path="$(awk -F': ' '/Benchmark Markdown:/ {print $2}' <<<"$stdout_text")"

  local unstable_count
  unstable_count="$(jq -r '.summary.unstable_lane_count' "$json_path")"
  local has_expected_unstable_tag
  has_expected_unstable_tag="$(jq -r '.summary.unstable_lanes | any(.tag=="unstable-supported")' "$json_path")"
  if [[ "$unstable_count" -lt 1 || "$has_expected_unstable_tag" != "true" ]]; then
    echo "expected unstable lane warning to be recorded" >&2
    cat "$json_path" >&2
    return 1
  fi

  local markdown_text
  markdown_text="$(cat "$md_path")"
  assert_contains "$markdown_text" "## Stability Warnings"
  assert_contains "$markdown_text" "| unstable-supported |"
}

test_real_repo_benchmark_keeps_unstable_lanes_on_partial_failures() {
  local cases_root="$TEST_TMP_DIR/partial-stability-cases"
  local mock_bin_dir="$TEST_TMP_DIR/partial-stability-mock-bin"
  local mock_uc="$mock_bin_dir/uc"
  local mock_scarb="$mock_bin_dir/scarb"
  local results_dir="$TEST_TMP_DIR/partial-stability-results"
  mkdir -p "$mock_bin_dir" "$results_dir"
  write_mock_uc_bin "$mock_uc"
  write_mock_scarb_bin "$mock_scarb"
  write_manifest_case "$cases_root" "unstable-fails-build"

  local stdout_text
  stdout_text="$(
    PATH="$mock_bin_dir:$PATH" \
    MOCK_UC_ARGS_LOG="$TEST_TMP_DIR/partial-stability-uc.args" \
    MOCK_UC_STATE_DIR="$TEST_TMP_DIR/partial-stability-uc.state" \
    MOCK_SCARB_ARGS_LOG="$TEST_TMP_DIR/partial-stability-scarb.args" \
    "$BENCH_SCRIPT" \
      --uc-bin "$mock_uc" \
      --results-dir "$results_dir" \
      --runs 4 \
      --cold-runs 5 \
      --warm-settle-seconds 0 \
      --case "$cases_root/unstable-fails-build/Scarb.toml" unstable-fails-build
  )"
  local json_path
  json_path="$(awk -F': ' '/Benchmark JSON:/ {print $2}' <<<"$stdout_text")"

  local benchmark_status
  benchmark_status="$(jq -r '.cases[] | select(.tag=="unstable-fails-build") | .benchmark_status' "$json_path")"
  if [[ "$benchmark_status" != "failed" ]]; then
    echo "fixture should create a partially failed benchmark case" >&2
    cat "$json_path" >&2
    return 1
  fi

  local unstable_count
  unstable_count="$(jq -r '.summary.unstable_lane_count' "$json_path")"
  local has_expected_unstable_tag
  has_expected_unstable_tag="$(jq -r '.summary.unstable_lanes | any(.tag=="unstable-fails-build")' "$json_path")"
  if [[ "$unstable_count" -lt 1 || "$has_expected_unstable_tag" != "true" ]]; then
    echo "expected unstable ok lanes to remain visible despite a failed sibling lane" >&2
    cat "$json_path" >&2
    return 1
  fi
}

test_real_repo_benchmark_instability_state_is_manifest_specific() {
  local cases_root_a="$TEST_TMP_DIR/stability-collision-a"
  local cases_root_b="$TEST_TMP_DIR/stability-collision-b"
  local mock_bin_dir="$TEST_TMP_DIR/stability-collision-mock-bin"
  local mock_uc="$mock_bin_dir/uc"
  local mock_scarb="$mock_bin_dir/scarb"
  local results_dir="$TEST_TMP_DIR/stability-collision-results"
  local state_dir="$TEST_TMP_DIR/stability-collision-state"
  mkdir -p "$mock_bin_dir" "$results_dir" "$state_dir"
  write_mock_uc_bin "$mock_uc"
  write_mock_scarb_bin "$mock_scarb"
  write_manifest_case "$cases_root_a" "unstable-same-name"
  write_manifest_case "$cases_root_b" "unstable-same-name"

  PATH="$mock_bin_dir:$PATH" \
  MOCK_UC_ARGS_LOG="$TEST_TMP_DIR/stability-collision-uc.args" \
  MOCK_UC_STATE_DIR="$state_dir" \
  MOCK_SCARB_ARGS_LOG="$TEST_TMP_DIR/stability-collision-scarb.args" \
  "$BENCH_SCRIPT" \
    --uc-bin "$mock_uc" \
    --results-dir "$results_dir" \
    --runs 1 \
    --cold-runs 1 \
    --warm-settle-seconds 0 \
    --case "$cases_root_a/unstable-same-name/Scarb.toml" unstable-a \
    --case "$cases_root_b/unstable-same-name/Scarb.toml" unstable-b >/dev/null

  local counter_count
  counter_count="$(find "$state_dir" -name '*.count' -type f | wc -l | tr -d ' ')"
  if [[ "$counter_count" != "4" ]]; then
    echo "expected one instability counter per manifest, got $counter_count" >&2
    find "$state_dir" -name '*.count' -type f -print >&2
    return 1
  fi
}

run_test "real_repo_benchmark_rejects_missing_case_values" \
  test_real_repo_benchmark_rejects_missing_case_values
run_test "real_repo_benchmark_rejects_zero_runs_from_environment" \
  test_real_repo_benchmark_rejects_zero_runs_from_environment
run_test "real_repo_benchmark_rejects_no_cases_with_updated_usage" \
  test_real_repo_benchmark_rejects_no_cases_with_updated_usage
run_test "real_repo_benchmark_rejects_directory_uc_bin" \
  test_real_repo_benchmark_rejects_directory_uc_bin
run_test "real_repo_benchmark_accepts_cases_file" \
  test_real_repo_benchmark_accepts_cases_file
run_test "real_repo_benchmark_canonicalizes_relative_paths" \
  test_real_repo_benchmark_canonicalizes_relative_paths
run_test "real_repo_benchmark_rejects_malformed_cases_file_rows" \
  test_real_repo_benchmark_rejects_malformed_cases_file_rows
run_test "real_repo_benchmark_records_support_matrix_categories" \
  test_real_repo_benchmark_records_support_matrix_categories
run_test "real_repo_benchmark_records_supported_build_failures" \
  test_real_repo_benchmark_records_supported_build_failures
run_test "real_repo_benchmark_reports_prefetch_failure_context" \
  test_real_repo_benchmark_reports_prefetch_failure_context
run_test "real_repo_benchmark_surfaces_stability_warnings" \
  test_real_repo_benchmark_surfaces_stability_warnings
run_test "real_repo_benchmark_keeps_unstable_lanes_on_partial_failures" \
  test_real_repo_benchmark_keeps_unstable_lanes_on_partial_failures
run_test "real_repo_benchmark_instability_state_is_manifest_specific" \
  test_real_repo_benchmark_instability_state_is_manifest_specific

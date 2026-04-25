#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
SUMMARY_SCRIPT="$SCRIPT_DIR/../summarize_corpus_opportunities.py"
TEST_TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TEST_TMP_DIR"' EXIT

run_test() {
  local name="$1"
  shift
  echo "[test] $name"
  "$@"
}

assert_json_value() {
  local name="$1"
  local actual="$2"
  local expected="$3"
  local json_path="$4"
  if [[ "$actual" != "$expected" ]]; then
    echo "unexpected $name: got '$actual', expected '$expected'" >&2
    cat "$json_path" >&2
    exit 1
  fi
}

assert_markdown_contains() {
  local needle="$1"
  local markdown_path="$2"
  if ! grep -Fq "$needle" "$markdown_path"; then
    echo "markdown summary is missing expected substring: $needle" >&2
    cat "$markdown_path" >&2
    exit 1
  fi
}

write_real_repo_fixture() {
  local path="$1"
  cat > "$path" <<'JSON'
{
  "generated_at": "2026-04-25T00:00:00Z",
  "summary": {
    "support_matrix": {
      "native_supported": 1,
      "fallback_used": 1,
      "native_unsupported": 1,
      "build_failed": 0
    },
    "unstable_lanes": [
      {
        "tag": "supported-hotspot",
        "tool": "uc",
        "stage": "build.cold",
        "p50_ms": 1000,
        "p95_ms": 1800,
        "max_ms": 2000,
        "p95_over_p50": 1.8,
        "max_over_p50": 2.0
      }
    ],
    "unstable_lane_count": 1
  },
  "cases": [
    {
      "tag": "supported-hotspot",
      "manifest_path": "/tmp/supported/Scarb.toml",
      "native_support": {
        "supported": true,
        "package_cairo_version": "2.16.0",
        "toolchain": {
          "requested_version": "2.16.0",
          "requested_major_minor": "2.16",
          "request_source": "package_cairo_version",
          "source": "builtin",
          "compiler_version": "2.16.0"
        },
        "diagnostics": []
      },
      "support_matrix": {
        "classification": "native_supported",
        "compile_backend": "uc_native",
        "fallback_used": false,
        "build_report": {
          "native_toolchain": {
            "requested_version": "2.16.0",
            "requested_major_minor": "2.16",
            "request_source": "package_cairo_version",
            "source": "builtin",
            "compiler_version": "2.16.0"
          },
          "phase_telemetry": {
            "compile_ms": 4000,
            "native_frontend_compile_ms": 3200,
            "native_casm_ms": 500,
            "fingerprint_ms": 100
          },
          "diagnostics": []
        }
      },
      "benchmark_status": "ok",
      "benchmarks": {
        "scarb": {"build": {"cold": {"stats": {"p95_ms": 1000}}, "warm_noop": {"stats": {"p95_ms": 500}}}},
        "uc": {"build": {"cold": {"stats": {"p95_ms": 900}}, "warm_noop": {"stats": {"p95_ms": 20}}}}
      }
    },
    {
      "tag": "fallback-case",
      "manifest_path": "/tmp/fallback/Scarb.toml",
      "native_support": {
        "supported": true,
        "package_cairo_version": "2.16.0",
        "diagnostics": []
      },
      "support_matrix": {
        "classification": "fallback_used",
        "compile_backend": "scarb_fallback",
        "fallback_used": true,
        "reason": "native failed and fallback was enabled",
        "build_report": {
          "diagnostics": [
            {
              "code": "UCN2002",
              "category": "native_fallback_local_native_error",
              "why": "native failed",
              "fallback_used": true
            }
          ]
        }
      },
      "benchmark_status": "skipped",
      "benchmarks": null
    },
    {
      "tag": "unsupported-case",
      "manifest_path": "/tmp/unsupported/Scarb.toml",
      "native_support": {
        "supported": false,
        "package_cairo_version": "2.13.0",
        "reason": "no native lane for Cairo 2.13.0"
      },
      "support_matrix": {
        "classification": "native_unsupported",
        "fallback_used": false,
        "reason": "no native lane for Cairo 2.13.0"
      },
      "benchmark_status": "skipped",
      "benchmarks": null
    }
  ]
}
JSON
}

test_real_repo_summary_records_opportunities() {
  local fixture="$TEST_TMP_DIR/real-repo.json"
  local out_json="$TEST_TMP_DIR/opportunities.json"
  local out_md="$TEST_TMP_DIR/opportunities.md"
  write_real_repo_fixture "$fixture"

  "$SUMMARY_SCRIPT" --benchmark-json "$fixture" --out-json "$out_json" --out-md "$out_md"

  local source_kind blocker_count fallback_gap unsupported_gap hotspot unstable weak_speedup diag_gap warm_opportunity
  source_kind="$(jq -r '.source_kind' "$out_json")"
  blocker_count="$(jq -r '.summary.blocker_opportunity_count' "$out_json")"
  fallback_gap="$(jq -r '.cases[] | select(.tag=="fallback-case") | .opportunity_codes | index("UCO1002") != null' "$out_json")"
  unsupported_gap="$(jq -r '.cases[] | select(.tag=="unsupported-case") | .opportunity_codes | index("UCO1001") != null' "$out_json")"
  hotspot="$(jq -r '.cases[] | select(.tag=="supported-hotspot") | .opportunity_codes | index("UCO3001") != null' "$out_json")"
  unstable="$(jq -r '.cases[] | select(.tag=="supported-hotspot") | .opportunity_codes | index("UCO2002") != null' "$out_json")"
  weak_speedup="$(jq -r '.cases[] | select(.tag=="supported-hotspot") | .opportunity_codes | index("UCO3006") != null' "$out_json")"
  diag_gap="$(jq -r '.cases[] | select(.tag=="fallback-case") | .opportunity_codes | index("UCO5001") != null' "$out_json")"
  warm_opportunity="$(jq -r '.cases[] | select(.tag=="supported-hotspot") | .opportunity_codes | index("UCO4002") != null' "$out_json")"

  assert_json_value "source_kind" "$source_kind" "real_repo_benchmark" "$out_json"
  assert_json_value "blocker_count" "$blocker_count" "2" "$out_json"
  assert_json_value "fallback_gap" "$fallback_gap" "true" "$out_json"
  assert_json_value "unsupported_gap" "$unsupported_gap" "true" "$out_json"
  assert_json_value "hotspot" "$hotspot" "true" "$out_json"
  assert_json_value "unstable" "$unstable" "true" "$out_json"
  assert_json_value "weak_speedup" "$weak_speedup" "true" "$out_json"
  assert_json_value "diag_gap" "$diag_gap" "true" "$out_json"
  assert_json_value "warm_opportunity" "$warm_opportunity" "true" "$out_json"

  assert_markdown_contains "Corpus Opportunity Summary" "$out_md"
  assert_markdown_contains "UCO3001" "$out_md"
  assert_markdown_contains "fallback-case" "$out_md"
}

test_deployed_wrapper_preserves_corpus_metadata() {
  local real_fixture="$TEST_TMP_DIR/real-repo.json"
  local deployed_fixture="$TEST_TMP_DIR/deployed.json"
  local out_json="$TEST_TMP_DIR/deployed-opportunities.json"
  write_real_repo_fixture "$real_fixture"
  jq '{
    schema_version: 1,
    corpus: {
      corpus_id: "sample-corpus",
      chain: "starknet-sepolia",
      selection: {coverage: "sample"},
      summary: {item_count: 1},
      items: [
        {
          tag: "supported-hotspot",
          source_kind: "deployed_contract",
          contract_address: "0x123",
          class_hash: "0xabc",
          cairo_version: "2.16.0"
        }
      ]
    },
    benchmark: .
  }' "$real_fixture" > "$deployed_fixture"

  "$SUMMARY_SCRIPT" --benchmark-json "$deployed_fixture" --out-json "$out_json"

  local source_kind corpus_id case_source_kind address
  source_kind="$(jq -r '.source_kind' "$out_json")"
  corpus_id="$(jq -r '.corpus.corpus_id' "$out_json")"
  case_source_kind="$(jq -r '.cases[] | select(.tag=="supported-hotspot") | .source_kind' "$out_json")"
  address="$(jq -r '.cases[] | select(.tag=="supported-hotspot") | .contract_address' "$out_json")"
  if [[ "$source_kind" != "deployed_contract_corpus" || "$corpus_id" != "sample-corpus" || "$case_source_kind" != "deployed_contract" || "$address" != "0x123" ]]; then
    echo "deployed wrapper metadata was not preserved" >&2
    cat "$out_json" >&2
    exit 1
  fi
}

test_complete_but_generic_diagnostic_is_not_agent_grade() {
  local fixture="$TEST_TMP_DIR/generic-diagnostic.json"
  local out_json="$TEST_TMP_DIR/generic-diagnostic-opportunities.json"
  cat > "$fixture" <<'JSON'
{
  "generated_at": "2026-04-25T00:00:00Z",
  "summary": {
    "support_matrix": {
      "native_supported": 0,
      "fallback_used": 0,
      "native_unsupported": 0,
      "build_failed": 1
    },
    "unstable_lanes": [],
    "unstable_lane_count": 0
  },
  "cases": [
    {
      "tag": "generic-native-failure",
      "manifest_path": "/tmp/generic/Scarb.toml",
      "native_support": {
        "supported": true,
        "package_cairo_version": "2.14.0",
        "diagnostics": []
      },
      "support_matrix": {
        "classification": "build_failed",
        "compile_backend": "uc_native_external_helper",
        "fallback_used": false,
        "reason": "Compilation failed.",
        "build_report": {
          "diagnostics": [
            {
              "code": "UCN2002",
              "category": "native_fallback_local_native_error",
              "what_happened": "Compilation failed.",
              "why": "Compilation failed.",
              "how_to_fix": [
                "Review the selected native toolchain lane."
              ],
              "retryable": true,
              "fallback_used": true,
              "toolchain_expected": "2.14.0",
              "toolchain_found": "2.14.0"
            }
          ]
        }
      },
      "benchmark_status": "skipped",
      "benchmarks": null
    }
  ]
}
JSON

  "$SUMMARY_SCRIPT" --benchmark-json "$fixture" --out-json "$out_json"

  local generic_gap weak_reason build_blocker
  generic_gap="$(jq -r '.cases[] | select(.tag=="generic-native-failure") | .opportunity_codes | index("UCO5001") != null' "$out_json")"
  weak_reason="$(jq -r '.cases[] | select(.tag=="generic-native-failure") | .opportunities[] | select(.code=="UCO5001") | .why' "$out_json")"
  build_blocker="$(jq -r '.cases[] | select(.tag=="generic-native-failure") | .opportunity_codes | index("UCO1003") != null' "$out_json")"
  assert_json_value "generic_gap" "$generic_gap" "true" "$out_json"
  assert_json_value "build_blocker" "$build_blocker" "true" "$out_json"
  if [[ "$weak_reason" != *"what_happened is generic"* || "$weak_reason" != *"why is generic"* ]]; then
    echo "expected generic diagnostic fields to be named in UCO5001 reason" >&2
    cat "$out_json" >&2
    exit 1
  fi
}

run_test "real_repo_summary_records_opportunities" test_real_repo_summary_records_opportunities
run_test "deployed_wrapper_preserves_corpus_metadata" test_deployed_wrapper_preserves_corpus_metadata
run_test "complete_but_generic_diagnostic_is_not_agent_grade" test_complete_but_generic_diagnostic_is_not_agent_grade

echo "All corpus opportunity summary tests passed."

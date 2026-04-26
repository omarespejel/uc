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
              "schema_version": 1,
              "code": "UCN2002",
              "category": "native_fallback_local_native_error",
              "severity": "warn",
              "title": "Native local build downgraded to Scarb",
              "docs_url": "https://github.com/omarespejel/uc/blob/main/docs/agent/AGENT_DIAGNOSTICS.md#ucn2002",
              "what_happened": "   ",
              "why": "Compilation failed.",
              "how_to_fix": [
                "Review the selected native toolchain lane."
              ],
              "next_commands": [
                "uc support native --manifest-path <Scarb.toml> --format json"
              ],
              "safe_automated_action": "inspect_native_support_then_retry",
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
    },
    {
      "tag": "fallback-used-only-generic",
      "manifest_path": "/tmp/fallback-used-only-generic/Scarb.toml",
      "native_support": {
        "supported": true,
        "package_cairo_version": "2.14.0",
        "diagnostics": []
      },
      "support_matrix": {
        "classification": "build_failed",
        "compile_backend": "uc_native_external_helper",
        "fallback_used": true,
        "reason": "fallback flag persisted from helper report",
        "build_report": {
          "diagnostics": []
        }
      },
      "benchmark_status": "skipped",
      "benchmarks": null
    },
    {
      "tag": "scarb-fallback-only-generic",
      "manifest_path": "/tmp/scarb-fallback-only-generic/Scarb.toml",
      "native_support": {
        "supported": true,
        "package_cairo_version": "2.14.0",
        "diagnostics": []
      },
      "support_matrix": {
        "classification": "build_failed",
        "compile_backend": "scarb_fallback",
        "fallback_used": false,
        "reason": "fallback backend label persisted from helper report",
        "build_report": {
          "diagnostics": []
        }
      },
      "benchmark_status": "skipped",
      "benchmarks": null
    }
  ]
}
JSON

  "$SUMMARY_SCRIPT" --benchmark-json "$fixture" --out-json "$out_json"

  local generic_gap weak_reason build_blocker fallback_activation
  local flag_only_backend flag_only_matrix flag_only_output
  local backend_only_backend backend_only_matrix backend_only_output
  generic_gap="$(jq -r '.cases[] | select(.tag=="generic-native-failure") | .opportunity_codes | index("UCO5001") != null' "$out_json")"
  weak_reason="$(jq -r '.cases[] | select(.tag=="generic-native-failure") | .opportunities[] | select(.code=="UCO5001") | .why' "$out_json")"
  build_blocker="$(jq -r '.cases[] | select(.tag=="generic-native-failure") | .opportunity_codes | index("UCO1003") != null' "$out_json")"
  fallback_activation="$(jq -r '.cases[] | select(.tag=="generic-native-failure") | .opportunity_codes | index("UCO1002") != null' "$out_json")"
  flag_only_backend="$(jq -r '.cases[] | select(.tag=="fallback-used-only-generic") | .compile_backend' "$out_json")"
  flag_only_matrix="$(jq -r '.cases[] | select(.tag=="fallback-used-only-generic") | .fallback_used' "$out_json")"
  flag_only_output="$(jq -r '.cases[] | select(.tag=="fallback-used-only-generic") | .opportunity_codes | index("UCO1002") != null' "$out_json")"
  backend_only_backend="$(jq -r '.cases[] | select(.tag=="scarb-fallback-only-generic") | .compile_backend' "$out_json")"
  backend_only_matrix="$(jq -r '.cases[] | select(.tag=="scarb-fallback-only-generic") | .fallback_used' "$out_json")"
  backend_only_output="$(jq -r '.cases[] | select(.tag=="scarb-fallback-only-generic") | .opportunity_codes | index("UCO1002") != null' "$out_json")"
  assert_json_value "generic_gap" "$generic_gap" "true" "$out_json"
  assert_json_value "build_blocker" "$build_blocker" "true" "$out_json"
  assert_json_value "fallback_activation" "$fallback_activation" "true" "$out_json"
  assert_json_value "fallback-used-only generic backend" "$flag_only_backend" "uc_native_external_helper" "$out_json"
  assert_json_value "fallback-used-only generic fallback_used" "$flag_only_matrix" "true" "$out_json"
  assert_json_value "fallback-used-only generic UCO1002" "$flag_only_output" "true" "$out_json"
  assert_json_value "scarb-fallback-only generic backend" "$backend_only_backend" "scarb_fallback" "$out_json"
  assert_json_value "scarb-fallback-only generic fallback_used" "$backend_only_matrix" "true" "$out_json"
  assert_json_value "scarb-fallback-only generic UCO1002" "$backend_only_output" "true" "$out_json"
  if [[ "$weak_reason" != *"what_happened is generic"* || "$weak_reason" != *"why is generic"* ]]; then
    echo "expected generic diagnostic fields to be named in UCO5001 reason" >&2
    cat "$out_json" >&2
    exit 1
  fi
}

test_stock_and_malformed_diagnostic_is_not_agent_grade() {
  local fixture="$TEST_TMP_DIR/stock-malformed-diagnostic.json"
  local out_json="$TEST_TMP_DIR/stock-malformed-diagnostic-opportunities.json"
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
      "tag": "stock-malformed-native-failure",
      "manifest_path": "/tmp/stock-malformed/Scarb.toml",
      "native_support": {
        "supported": true,
        "package_cairo_version": "2.14.0",
        "diagnostics": []
      },
      "support_matrix": {
        "classification": "build_failed",
        "compile_backend": "uc_native_external_helper",
        "fallback_used": false,
        "reason": "uc auto build failed before backend classification completed",
        "build_report": {
          "diagnostics": [
            {
              "schema_version": 1,
              "code": "UCN2002",
              "category": "native_fallback_local_native_error",
              "severity": "warn",
              "title": "Native local build downgraded to Scarb",
              "docs_url": "https://github.com/omarespejel/uc/blob/main/docs/agent/AGENT_DIAGNOSTICS.md#ucn2002",
              "what_happened": "uc auto build failed before backend classification completed",
              "why": {"message": "native build failed"},
              "how_to_fix": [
                "Review the selected native toolchain lane."
              ],
              "next_commands": [
                "uc support native --manifest-path <Scarb.toml> --format json"
              ],
              "safe_automated_action": "inspect_native_support_then_retry",
              "retryable": true,
              "fallback_used": true
            }
          ]
        }
      },
      "benchmark_status": "skipped",
      "benchmarks": null
    },
    {
      "tag": "fallback-used-only-stock",
      "manifest_path": "/tmp/fallback-used-only-stock/Scarb.toml",
      "native_support": {
        "supported": true,
        "package_cairo_version": "2.14.0",
        "diagnostics": []
      },
      "support_matrix": {
        "classification": "build_failed",
        "compile_backend": "uc_native_external_helper",
        "fallback_used": true,
        "reason": "fallback flag persisted from helper report",
        "build_report": {
          "diagnostics": []
        }
      },
      "benchmark_status": "skipped",
      "benchmarks": null
    },
    {
      "tag": "scarb-fallback-only-stock",
      "manifest_path": "/tmp/scarb-fallback-only-stock/Scarb.toml",
      "native_support": {
        "supported": true,
        "package_cairo_version": "2.14.0",
        "diagnostics": []
      },
      "support_matrix": {
        "classification": "build_failed",
        "compile_backend": "scarb_fallback",
        "fallback_used": false,
        "reason": "fallback backend label persisted from helper report",
        "build_report": {
          "diagnostics": []
        }
      },
      "benchmark_status": "skipped",
      "benchmarks": null
    }
  ]
}
JSON

  "$SUMMARY_SCRIPT" --benchmark-json "$fixture" --out-json "$out_json"

  local generic_gap weak_reason fallback_activation
  local flag_only_backend flag_only_matrix flag_only_output
  local backend_only_backend backend_only_matrix backend_only_output
  generic_gap="$(jq -r '.cases[] | select(.tag=="stock-malformed-native-failure") | .opportunity_codes | index("UCO5001") != null' "$out_json")"
  weak_reason="$(jq -r '.cases[] | select(.tag=="stock-malformed-native-failure") | .opportunities[] | select(.code=="UCO5001") | .why' "$out_json")"
  fallback_activation="$(jq -r '.cases[] | select(.tag=="stock-malformed-native-failure") | .opportunity_codes | index("UCO1002") != null' "$out_json")"
  flag_only_backend="$(jq -r '.cases[] | select(.tag=="fallback-used-only-stock") | .compile_backend' "$out_json")"
  flag_only_matrix="$(jq -r '.cases[] | select(.tag=="fallback-used-only-stock") | .fallback_used' "$out_json")"
  flag_only_output="$(jq -r '.cases[] | select(.tag=="fallback-used-only-stock") | .opportunity_codes | index("UCO1002") != null' "$out_json")"
  backend_only_backend="$(jq -r '.cases[] | select(.tag=="scarb-fallback-only-stock") | .compile_backend' "$out_json")"
  backend_only_matrix="$(jq -r '.cases[] | select(.tag=="scarb-fallback-only-stock") | .fallback_used' "$out_json")"
  backend_only_output="$(jq -r '.cases[] | select(.tag=="scarb-fallback-only-stock") | .opportunity_codes | index("UCO1002") != null' "$out_json")"
  assert_json_value "generic_gap" "$generic_gap" "true" "$out_json"
  assert_json_value "fallback_activation" "$fallback_activation" "true" "$out_json"
  assert_json_value "fallback-used-only stock backend" "$flag_only_backend" "uc_native_external_helper" "$out_json"
  assert_json_value "fallback-used-only stock fallback_used" "$flag_only_matrix" "true" "$out_json"
  assert_json_value "fallback-used-only stock UCO1002" "$flag_only_output" "true" "$out_json"
  assert_json_value "scarb-fallback-only stock backend" "$backend_only_backend" "scarb_fallback" "$out_json"
  assert_json_value "scarb-fallback-only stock fallback_used" "$backend_only_matrix" "true" "$out_json"
  assert_json_value "scarb-fallback-only stock UCO1002" "$backend_only_output" "true" "$out_json"
  if [[ "$weak_reason" != *"what_happened is generic"* || "$weak_reason" != *"why is not a string"* ]]; then
    echo "expected stock and malformed diagnostic fields to be named in UCO5001 reason" >&2
    cat "$out_json" >&2
    exit 1
  fi
}

test_fallback_signal_branches_are_detected() {
  local fixture="$TEST_TMP_DIR/fallback-signal-branches.json"
  local out_json="$TEST_TMP_DIR/fallback-signal-branches-opportunities.json"
  cat > "$fixture" <<'JSON'
{
  "generated_at": "2026-04-25T00:00:00Z",
  "summary": {
    "support_matrix": {
      "native_supported": 0,
      "fallback_used": 0,
      "native_unsupported": 0,
      "build_failed": 2
    },
    "unstable_lanes": [],
    "unstable_lane_count": 0
  },
  "cases": [
    {
      "tag": "fallback-used-only",
      "manifest_path": "/tmp/fallback-used-only/Scarb.toml",
      "native_support": {
        "supported": true,
        "package_cairo_version": "2.14.0",
        "diagnostics": []
      },
      "support_matrix": {
        "classification": "build_failed",
        "compile_backend": "uc_native_external_helper",
        "fallback_used": true,
        "reason": "fallback flag persisted from helper report",
        "build_report": {
          "diagnostics": []
        }
      },
      "benchmark_status": "skipped",
      "benchmarks": null
    },
    {
      "tag": "scarb-fallback-only",
      "manifest_path": "/tmp/scarb-fallback-only/Scarb.toml",
      "native_support": {
        "supported": true,
        "package_cairo_version": "2.14.0",
        "diagnostics": []
      },
      "support_matrix": {
        "classification": "build_failed",
        "compile_backend": "scarb_fallback",
        "fallback_used": false,
        "reason": "fallback backend label persisted from helper report",
        "build_report": {
          "diagnostics": []
        }
      },
      "benchmark_status": "skipped",
      "benchmarks": null
    }
  ]
}
JSON

  "$SUMMARY_SCRIPT" --benchmark-json "$fixture" --out-json "$out_json"

  local flag_only_backend flag_only_matrix flag_only_output flag_only_uco
  flag_only_backend="$(jq -r '.cases[] | select(.tag=="fallback-used-only") | .compile_backend' "$out_json")"
  flag_only_matrix="$(jq -r '.cases[] | select(.tag=="fallback-used-only") | .fallback_used' "$out_json")"
  flag_only_output="$(jq -r '.cases[] | select(.tag=="fallback-used-only") | .opportunity_codes | index("UCO1002") != null' "$out_json")"
  flag_only_uco="$(jq -r '.cases[] | select(.tag=="fallback-used-only") | .opportunity_codes | index("UCO1003") != null' "$out_json")"

  local backend_only_backend backend_only_output backend_only_uco
  backend_only_backend="$(jq -r '.cases[] | select(.tag=="scarb-fallback-only") | .compile_backend' "$out_json")"
  backend_only_output="$(jq -r '.cases[] | select(.tag=="scarb-fallback-only") | .fallback_used' "$out_json")"
  backend_only_uco="$(jq -r '.cases[] | select(.tag=="scarb-fallback-only") | .opportunity_codes | index("UCO1002") != null' "$out_json")"

  assert_json_value "fallback-used-only backend" "$flag_only_backend" "uc_native_external_helper" "$out_json"
  assert_json_value "fallback-used-only fallback_used" "$flag_only_matrix" "true" "$out_json"
  assert_json_value "fallback-used-only UCO1002" "$flag_only_output" "true" "$out_json"
  assert_json_value "fallback-used-only UCO1003" "$flag_only_uco" "true" "$out_json"
  assert_json_value "scarb-fallback-only backend" "$backend_only_backend" "scarb_fallback" "$out_json"
  assert_json_value "scarb-fallback-only fallback_used" "$backend_only_output" "true" "$out_json"
  assert_json_value "scarb-fallback-only UCO1002" "$backend_only_uco" "true" "$out_json"
}

test_remediation_fields_are_validated_without_overmatching() {
  local fixture="$TEST_TMP_DIR/remediation-field-quality.json"
  local out_json="$TEST_TMP_DIR/remediation-field-quality-opportunities.json"
  cat > "$fixture" <<'JSON'
{
  "generated_at": "2026-04-25T00:00:00Z",
  "summary": {
    "support_matrix": {
      "native_supported": 3,
      "fallback_used": 0,
      "native_unsupported": 0,
      "build_failed": 0
    },
    "unstable_lanes": [],
    "unstable_lane_count": 0
  },
  "cases": [
    {
      "tag": "detailed-prefixed-native-failure",
      "manifest_path": "/tmp/detailed/Scarb.toml",
      "native_support": {
        "supported": true,
        "package_cairo_version": "2.14.0",
        "diagnostics": [
          {
            "schema_version": 1,
            "code": "UCN2002",
            "category": "native_fallback_local_native_error",
            "severity": "warn",
            "title": "Native build failed with Cairo diagnostic E0002",
            "docs_url": "https://github.com/omarespejel/uc/blob/main/docs/agent/AGENT_DIAGNOSTICS.md#ucn2002",
            "what_happened": "uc auto build failed: error[E0002] Method span could not be called.",
            "why": "The selected Cairo helper emitted E0002 while compiling a dependency.",
            "how_to_fix": [
              "Open the recorded native build log and inspect E0002."
            ],
            "next_commands": [
              "uc replay /tmp/uc-failure.json"
            ],
            "safe_automated_action": "record_failure_bundle",
            "retryable": true,
            "fallback_used": false
          }
        ]
      },
      "support_matrix": {
        "classification": "native_supported",
        "compile_backend": "uc_native_external_helper",
        "fallback_used": false
      },
      "benchmark_status": "skipped",
      "benchmarks": null
    },
    {
      "tag": "empty-remediation-fields",
      "manifest_path": "/tmp/empty-remediation/Scarb.toml",
      "native_support": {
        "supported": true,
        "package_cairo_version": "2.14.0",
        "diagnostics": [
          {
            "schema_version": 1,
            "code": "UCN2002",
            "category": "native_fallback_local_native_error",
            "severity": "warn",
            "title": "Native build failed with Cairo diagnostic E0002",
            "docs_url": " ",
            "what_happened": "The Cairo helper emitted error E0002 while compiling a dependency.",
            "why": "The dependency calls a method that is unavailable for the selected core type.",
            "how_to_fix": [],
            "next_commands": [],
            "safe_automated_action": "",
            "retryable": true,
            "fallback_used": false
          }
        ]
      },
      "support_matrix": {
        "classification": "native_supported",
        "compile_backend": "uc_native_external_helper",
        "fallback_used": false
      },
      "benchmark_status": "skipped",
      "benchmarks": null
    },
    {
      "tag": "wrong-required-types",
      "manifest_path": "/tmp/wrong-types/Scarb.toml",
      "native_support": {
        "supported": true,
        "package_cairo_version": "2.14.0",
        "diagnostics": [
          {
            "schema_version": "1",
            "code": [],
            "category": {},
            "severity": "warn",
            "title": "Native build failed with Cairo diagnostic E0002",
            "docs_url": "https://github.com/omarespejel/uc/blob/main/docs/agent/AGENT_DIAGNOSTICS.md#ucn2002",
            "what_happened": "The Cairo helper emitted error E0002 while compiling a dependency.",
            "why": "The dependency calls a method that is unavailable for the selected core type.",
            "how_to_fix": [
              "Open the recorded native build log and inspect E0002."
            ],
            "next_commands": [
              "uc replay /tmp/uc-failure.json"
            ],
            "safe_automated_action": "record_failure_bundle",
            "retryable": "true",
            "fallback_used": 1
          }
        ]
      },
      "support_matrix": {
        "classification": "native_supported",
        "compile_backend": "uc_native_external_helper",
        "fallback_used": false
      },
      "benchmark_status": "skipped",
      "benchmarks": null
    }
  ]
}
JSON

  "$SUMMARY_SCRIPT" --benchmark-json "$fixture" --out-json "$out_json"

  local detailed_gap remediation_gap remediation_reason type_gap type_reason
  detailed_gap="$(jq -r '.cases[] | select(.tag=="detailed-prefixed-native-failure") | .opportunity_codes | index("UCO5001") != null' "$out_json")"
  remediation_gap="$(jq -r '.cases[] | select(.tag=="empty-remediation-fields") | .opportunity_codes | index("UCO5001") != null' "$out_json")"
  remediation_reason="$(jq -r '.cases[] | select(.tag=="empty-remediation-fields") | .opportunities[] | select(.code=="UCO5001") | .why' "$out_json")"
  type_gap="$(jq -r '.cases[] | select(.tag=="wrong-required-types") | .opportunity_codes | index("UCO5001") != null' "$out_json")"
  type_reason="$(jq -r '.cases[] | select(.tag=="wrong-required-types") | .opportunities[] | select(.code=="UCO5001") | .why' "$out_json")"
  assert_json_value "detailed_gap" "$detailed_gap" "false" "$out_json"
  assert_json_value "remediation_gap" "$remediation_gap" "true" "$out_json"
  assert_json_value "type_gap" "$type_gap" "true" "$out_json"
  if [[ "$remediation_reason" != *"docs_url is empty"* || "$remediation_reason" != *"how_to_fix is empty"* || "$remediation_reason" != *"next_commands is empty"* || "$remediation_reason" != *"safe_automated_action is empty"* ]]; then
    echo "expected empty remediation fields to be named in UCO5001 reason" >&2
    cat "$out_json" >&2
    exit 1
  fi
  if [[ "$type_reason" != *"schema_version is not an integer"* || "$type_reason" != *"code is empty"* || "$type_reason" != *"category is empty"* || "$type_reason" != *"retryable is not a boolean"* || "$type_reason" != *"fallback_used is not a boolean"* ]]; then
    echo "expected wrong required field types to be named in UCO5001 reason" >&2
    cat "$out_json" >&2
    exit 1
  fi
}

run_test "real_repo_summary_records_opportunities" test_real_repo_summary_records_opportunities
run_test "deployed_wrapper_preserves_corpus_metadata" test_deployed_wrapper_preserves_corpus_metadata
run_test "complete_but_generic_diagnostic_is_not_agent_grade" test_complete_but_generic_diagnostic_is_not_agent_grade
run_test "stock_and_malformed_diagnostic_is_not_agent_grade" test_stock_and_malformed_diagnostic_is_not_agent_grade
run_test "fallback_signal_branches_are_detected" test_fallback_signal_branches_are_detected
run_test "remediation_fields_are_validated_without_overmatching" test_remediation_fields_are_validated_without_overmatching

echo "All corpus opportunity summary tests passed."

#!/usr/bin/env zsh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${(%):-%N}")" && pwd -P)"
ROOT_DIR="$(git -C "$SCRIPT_DIR/../.." rev-parse --show-toplevel 2>/dev/null || (cd "$SCRIPT_DIR/../.." && pwd -P))"
WORKSPACE_ROOT="${WORKSPACE_ROOT:-}"
STAMP="$(date +%Y%m%d-%H%M%S)"
OUT_DIR="$ROOT_DIR/benchmarks/results"
SUMMARY_MD="$OUT_DIR/compare-summary-$STAMP.md"

if [[ -z "$WORKSPACE_ROOT" ]]; then
  echo "WORKSPACE_ROOT is required for dual-run comparator." >&2
  echo "Set WORKSPACE_ROOT to a path where scarb/examples exists." >&2
  exit 1
fi
WORKSPACE_ROOT="$(cd "$WORKSPACE_ROOT" && pwd -P)"

mkdir -p "$OUT_DIR"

cd "$ROOT_DIR"
cargo build -p uc-cli >/dev/null
UC_BIN="$ROOT_DIR/target/debug/uc"

run_case() {
  local name="$1"
  local manifest="$2"
  local output_rel="benchmarks/results/compare-${name}-$STAMP.json"
  local output="$ROOT_DIR/$output_rel"

  "$UC_BIN" compare-build \
    --manifest-path "$manifest" \
    --clean-before-each true \
    --output-path "$output" >/dev/null

  echo "$output"
}

HELLO_MANIFEST="$WORKSPACE_ROOT/scarb/examples/hello_world/Scarb.toml"
WORKSPACES_MANIFEST="$WORKSPACE_ROOT/scarb/examples/workspaces/Scarb.toml"

if [[ ! -f "$HELLO_MANIFEST" || ! -f "$WORKSPACES_MANIFEST" ]]; then
  echo "Expected benchmark manifests not found under $WORKSPACE_ROOT" >&2
  echo "Hint: set WORKSPACE_ROOT to a path where scarb/examples exists." >&2
  exit 1
fi

HELLO_JSON="$(run_case "hello_world" "$HELLO_MANIFEST")"
WS_JSON="$(run_case "workspaces" "$WORKSPACES_MANIFEST")"

{
  echo "# Dual-Run Comparator Summary ($STAMP)"
  echo
  echo "| Workload | Passed | Artifact Mismatches | Diagnostics Similarity (%) | Baseline ms | Candidate ms |"
  echo "|---|---|---:|---:|---:|---:|"
  for report in "$HELLO_JSON" "$WS_JSON"; do
    jq -r '
      . as $r
      | $r.manifest_path
      | split("/")
      | .[-2] as $workload
      | "| \($workload) | \($r.passed) | \($r.artifact_mismatch_count) | \(($r.diagnostics.similarity_percent * 100 | round) / 100) | \(($r.baseline.elapsed_ms * 100 | round) / 100) | \(($r.candidate.elapsed_ms * 100 | round) / 100) |"
    ' "$report"
  done
  echo
  echo "## Reports"
  echo "- benchmarks/results/$(basename "$HELLO_JSON")"
  echo "- benchmarks/results/$(basename "$WS_JSON")"
} > "$SUMMARY_MD"

echo "Comparator summary: $SUMMARY_MD"

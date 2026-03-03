#!/usr/bin/env zsh
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
OUT_DIR="$ROOT_DIR/benchmarks/results"
STAMP="$(date +%Y%m%d-%H%M%S)"
OUT_FILE="$OUT_DIR/local-benchmark-$STAMP.md"

mkdir -p "$OUT_DIR"

if ! command -v hyperfine >/dev/null 2>&1; then
  echo "hyperfine not found. Install it first." >&2
  exit 1
fi

cat > "$OUT_FILE" <<EOF
# Local Benchmark Report ($STAMP)

## Environment
- Host: $(hostname)
- Date: $(date)

## Notes
- Replace commands below with your actual tool binaries and workspace paths.

EOF

echo "Running placeholder benchmark suite..."

hyperfine --warmup 1 --runs 5 \
  --export-markdown "$OUT_DIR/hyperfine-noop-$STAMP.md" \
  'echo "warm run A" >/dev/null' \
  'echo "warm run B" >/dev/null'

cat >> "$OUT_FILE" <<EOF
## Outputs
- Hyperfine markdown: benchmarks/results/hyperfine-noop-$STAMP.md

EOF

echo "Benchmark report written to: $OUT_FILE"


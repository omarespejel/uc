#!/usr/bin/env bash

uc_native_ci_install_no_scarb_stub() {
  local stub_path="$1"
  mkdir -p "$(dirname "$stub_path")"
  cat > "$stub_path" <<'STUB'
#!/usr/bin/env bash
echo "native-only gate: scarb is intentionally unavailable" >&2
exit 127
STUB
  chmod +x "$stub_path"
}

uc_native_ci_log_indicates_unsupported() {
  local log_path="$1"
  if [[ -z "$log_path" || ! -f "$log_path" ]]; then
    return 1
  fi

  grep -Eq \
    'native compile does not support .* yet|native compile manifest includes non-starknet dependencies|Plugin diagnostic: Unsupported attribute\.|#\[executable\]|\[dependencies\]\.cairo_execute' \
    "$log_path"
}

uc_native_ci_verify_report() {
  local report_path="$1"
  local tag="$2"
  local allowed_backends_csv="$3"

  if [[ ! -f "$report_path" ]]; then
    echo "native CI gate failed: report missing for '$tag'" >&2
    return 1
  fi

  python3 - "$report_path" "$tag" "$allowed_backends_csv" <<'PY'
import json
import sys
from pathlib import Path

report_path = Path(sys.argv[1])
tag = sys.argv[2]
allowed_backends = {item for item in sys.argv[3].split(",") if item}
report = json.loads(report_path.read_text(encoding="utf-8"))
exit_code = report.get("exit_code")
if exit_code != 0:
    raise SystemExit(
        f"native CI gate failed: non-zero report exit code for {tag}: {exit_code}"
    )
command = report.get("command") or []
backend = command[0] if command else ""
if backend not in allowed_backends:
    raise SystemExit(
        f"native CI gate failed: unexpected backend for {tag}: {backend or command!r}"
    )
print(f"{tag}: backend={backend}")
PY
}

#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

chmod +x .githooks/pre-push scripts/local_ci_gate.sh
git config core.hooksPath .githooks

printf 'Installed repo-managed git hooks.\n'
printf 'core.hooksPath=%s\n' "$(git config --get core.hooksPath)"

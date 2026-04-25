#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

log() {
  printf '[local-ci] %s\n' "$*"
}

run_make() {
  local target="$1"
  if [[ -n "${UC_LOCAL_CI_CAPTURE_PATH:-}" ]]; then
    printf 'make %s\n' "$target" >>"$UC_LOCAL_CI_CAPTURE_PATH"
    return 0
  fi
  log "running make $target"
  make "$target"
}

collect_changed_files() {
  if [[ -n "${UC_LOCAL_CI_CHANGED_FILES_FILE:-}" ]]; then
    cat "$UC_LOCAL_CI_CHANGED_FILES_FILE"
    return 0
  fi

  if ! git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    return 0
  fi

  local base=""
  if git rev-parse --verify origin/main >/dev/null 2>&1; then
    base="$(git merge-base HEAD origin/main)"
  elif git rev-parse --verify HEAD^ >/dev/null 2>&1; then
    base="HEAD^"
  fi

  {
    if [[ -n "$base" ]]; then
      git diff --name-only "$base..HEAD"
    fi
    git diff --name-only --cached
    git diff --name-only
    git ls-files --others --exclude-standard
  } | awk 'NF { print }' | sort -u
}

main() {
  local changed_files
  changed_files="$(collect_changed_files)"

  local docs_surface_changed=0
  local benchmark_changed=0
  local script_changed=0
  local rust_changed=0
  local native_changed=0

  if [[ -n "$changed_files" ]]; then
    while IFS= read -r path; do
      [[ -n "$path" ]] || continue
      case "$path" in
        AGENTS.md|.codex/START_HERE.md|docs/agent/*|docs/AGENT_FIRST_LAUNCH_MINIMUM_2026-04-24.md|docs/PROJECT_MODEL_STRATEGY.md|.coderabbit.yaml|.pr_agent.toml|best_practices.md|pr_compliance_checklist.yaml|.github/workflows/*|Makefile|scripts/doctor.sh|scripts/refresh_repo_map.sh|scripts/validate_agent_surface.sh|scripts/install_git_hooks.sh|scripts/local_ci_gate.sh|scripts/tests/local_ci_gate_test.sh|.githooks/pre-push)
          docs_surface_changed=1
          ;;
      esac
      case "$path" in
        scripts/*.sh|scripts/tests/*|docs/NATIVE_TOOLCHAIN_HELPERS.md)
          script_changed=1
          ;;
      esac
      case "$path" in
        benchmarks/*)
          benchmark_changed=1
          ;;
      esac
      case "$path" in
        *.rs|Cargo.toml|Cargo.lock)
          rust_changed=1
          ;;
      esac
      case "$path" in
        crates/uc-cli/src/main.rs|crates/uc-cli/src/main_tests.rs|crates/uc-cli/src/fingerprint.rs|crates/uc-cli/src/commands/build.rs|crates/uc-cli/tests/*|third_party/cairo-lang-filesystem/*|third_party/cairo-lang-filesystem/**/*)
          native_changed=1
          rust_changed=1
          ;;
      esac
    done <<<"$changed_files"
  fi

  run_make doctor

  if [[ -z "$changed_files" ]]; then
    log "no changed files detected; running minimal agent surface validation"
    run_make agent-validate
    return 0
  fi

  log "changed files:"
  while IFS= read -r path; do
    [[ -n "$path" ]] || continue
    printf '  - %s\n' "$path"
  done <<<"$changed_files"

  if (( native_changed )); then
    log "selected local gate: validate-native"
    run_make validate-native
    return 0
  fi

  if (( rust_changed )); then
    log "selected local gate: validate-fast"
    run_make validate-fast
    return 0
  fi

  if (( script_changed )); then
    log "selected local gate: validate-scripts"
    run_make validate-scripts
  fi

  if (( benchmark_changed )); then
    log "selected local gate: validate-bench-scripts"
    run_make validate-bench-scripts
  fi

  if (( docs_surface_changed || script_changed || !benchmark_changed )); then
    log "selected local gate: agent-validate"
    run_make agent-validate
  fi
}

main "$@"

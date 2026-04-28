#!/usr/bin/env zsh
set -euo pipefail

OWNER="${OWNER:-omarespejel}"
REPO="${REPO:-uc}"
PROJECT_TITLE="${PROJECT_TITLE:-uc Delivery Program}"

if ! command -v gh >/dev/null 2>&1; then
  echo "gh CLI not found" >&2
  exit 1
fi

echo "Bootstrapping labels for $OWNER/$REPO"

create_label() {
  local name="$1"
  local color="$2"
  local desc="$3"
  gh label create "$name" --repo "$OWNER/$REPO" --color "$color" --description "$desc" 2>/dev/null || \
  gh label edit "$name" --repo "$OWNER/$REPO" --color "$color" --description "$desc" >/dev/null
}

create_label "type:epic" "5319E7" "Epic-level work item"
create_label "type:feature" "1D76DB" "Feature work item"
create_label "type:task" "0E8A16" "Execution task"
create_label "type:bug" "D73A4A" "Defect"
create_label "area:benchmark" "FBCA04" "Benchmarking and perf analysis"
create_label "area:compiler" "0052CC" "Compiler and build engine"
create_label "area:resolver" "A371F7" "Project model, resolver, fetch, and source store"
create_label "area:agent" "1B7F83" "Agent-facing command surface and schemas"
create_label "area:ci" "C2E0C6" "CI and automation"
create_label "priority:p0" "B60205" "Highest priority"
create_label "priority:p1" "D93F0B" "High priority"
create_label "priority:p2" "FBCA04" "Medium priority"

echo "Bootstrapping milestones"

create_milestone() {
  local title="$1"
  local desc="$2"
  if gh api "repos/$OWNER/$REPO/milestones" --paginate | jq -e ".[] | select(.title == \"$title\")" >/dev/null; then
    echo "Milestone exists: $title"
  else
    gh api "repos/$OWNER/$REPO/milestones" --method POST -f title="$title" -f description="$desc" >/dev/null
    echo "Created milestone: $title"
  fi
}

create_milestone "M0 Foundations and Agent Contract" "Benchmark harness, KPI stack, ADRs, and agent-facing contract conventions."
create_milestone "M1 Build Proof MVP" "Sessionized build MVP and dual-run comparator."
create_milestone "M2 Agent-First Control Plane" "Project inspect, support detection, stable schemas, and build planning."
create_milestone "M3 Resolver, Fetch, and Toolchain Ownership" "Lockfile-first resolve/fetch, source store, and toolchain ensure path."
create_milestone "M4 Command Surface Expansion and CI/Proving" "Core command expansion, remote cache, and execute/prove integration."
create_milestone "M5 Cutover" "Org-wide rollout, migration completion, and compatibility-lane retirement planning."

echo "Seeding core issues"

milestone_exists() {
  local title="$1"
  gh api "repos/$OWNER/$REPO/milestones" --paginate | jq -e ".[] | select(.title == \"$title\")" >/dev/null
}

M0="M0 Foundations and Agent Contract"
M1="M1 Build Proof MVP"
M2="M2 Agent-First Control Plane"
M3="M3 Resolver, Fetch, and Toolchain Ownership"
M4="M4 Command Surface Expansion and CI/Proving"
M5="M5 Cutover"

for title in "$M0" "$M1" "$M2" "$M3" "$M4" "$M5"; do
  if ! milestone_exists "$title"; then
    echo "Required milestone missing: $title" >&2
    exit 1
  fi
done

create_issue_if_missing() {
  local title="$1"
  local body="$2"
  local labels="$3"
  local milestone="$4"
  if gh issue list --repo "$OWNER/$REPO" --state all --search "\"$title\" in:title" --json title | jq -e ".[] | select(.title == \"$title\")" >/dev/null; then
    echo "Issue exists: $title"
    return
  fi
  gh issue create \
    --repo "$OWNER/$REPO" \
    --title "$title" \
    --body "$body" \
    --label "$labels" \
    --milestone "$milestone" >/dev/null
  echo "Created issue: $title"
}

create_issue_if_missing \
  "epic: Foundation and agent contract" \
  "Lock benchmark harness, KPI stack, ADRs, repo-local agent instructions, and the initial machine-readable contract." \
  "type:epic,priority:p0,area:agent" \
  "$M0"

create_issue_if_missing \
  "feat: Establish benchmark harness and report/schema conventions" \
  "Add repeatable warm/cold scenario benchmarks and versioned report conventions for agent-facing outputs." \
  "type:feature,priority:p0,area:benchmark" \
  "$M0"

create_issue_if_missing \
  "feat: Build comparator for artifact and diagnostics parity" \
  "Create dual-run comparator to detect correctness drift between Scarb and uc native paths." \
  "type:feature,priority:p0,area:compiler" \
  "$M1"

create_issue_if_missing \
  "epic: Build proof MVP" \
  "Sessionized compile daemon, stable execution reports, local CAS, and correctness-gated build proof." \
  "type:epic,priority:p0,area:compiler" \
  "$M1"

create_issue_if_missing \
  "epic: Agent-first control plane" \
  "Deliver project inspect, support detection, stable schemas, and build planning as first-class uc surfaces." \
  "type:epic,priority:p0,area:agent" \
  "$M2"

create_issue_if_missing \
  "feat: First-party project inspect and support-native reports" \
  "Emit versioned project and support reports with explicit toolchain, fallback, and offline-readiness state." \
  "type:feature,priority:p0,area:agent" \
  "$M2"

create_issue_if_missing \
  "epic: Resolver, fetch, and toolchain ownership" \
  "Own lockfile-first resolve/fetch, source-store lifecycle, and toolchain/helper-lane acquisition in uc." \
  "type:epic,priority:p0,area:resolver" \
  "$M3"

create_issue_if_missing \
  "feat: Implement lockfile-first resolve/fetch and source-store lifecycle" \
  "Build uc resolve/fetch plus store status and prune primitives with bounded concurrency and offline-readiness reporting." \
  "type:feature,priority:p0,area:resolver" \
  "$M3"

create_issue_if_missing \
  "feat: Implement toolchain ensure and helper-lane diagnostics" \
  "Build explicit Cairo/helper toolchain ensure path with expected/found reporting and remediation-grade failures." \
  "type:feature,priority:p0,area:resolver" \
  "$M3"

create_issue_if_missing \
  "epic: Command surface expansion and CI/proving integration" \
  "Expand core command coverage and integrate remote cache plus execute/prove acceleration behind explicit agent-visible contracts." \
  "type:epic,priority:p1,area:ci" \
  "$M4"

create_issue_if_missing \
  "epic: Platform cutover execution" \
  "Drive uc default switch in CI and manage compatibility-lane retirement planning." \
  "type:epic,priority:p1,area:ci" \
  "$M5"

echo "Attempting Project setup"

if gh project create --owner "$OWNER" --title "$PROJECT_TITLE" >/tmp/uc-project-create.out 2>/tmp/uc-project-create.err; then
  PROJECT_ID="$(cat /tmp/uc-project-create.out | tr -d '\n')"
  echo "Created project: $PROJECT_TITLE (id: $PROJECT_ID)"
else
  echo "Project creation skipped. You may need to run:"
  echo "  gh auth refresh -s project"
  echo "  gh project create --owner $OWNER --title \"$PROJECT_TITLE\""
fi

echo "Bootstrap complete."

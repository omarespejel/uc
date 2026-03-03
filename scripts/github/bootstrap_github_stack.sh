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
create_label "area:migration" "B60205" "Migration tooling and docs"
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

create_milestone "M0 Foundations" "Program setup, benchmark harness, KPI stack."
create_milestone "M1 Core Engine MVP" "Daemonized build MVP and dual-run comparator."
create_milestone "M2 Migration Tooling" "Project migration path and core command surface."
create_milestone "M3 CI and Proving" "Remote cache and proving acceleration."
create_milestone "M4 Cutover" "Org-wide cutover and legacy sunset."

echo "Seeding core issues"

milestone_exists() {
  local title="$1"
  gh api "repos/$OWNER/$REPO/milestones" --paginate | jq -e ".[] | select(.title == \"$title\")" >/dev/null
}

M0="M0 Foundations"
M1="M1 Core Engine MVP"

if ! milestone_exists "$M0"; then
  echo "Required milestone missing: $M0" >&2
  exit 1
fi
if ! milestone_exists "$M1"; then
  echo "Required milestone missing: $M1" >&2
  exit 1
fi

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
  "epic: Program foundation and KPI operating stack" \
  "Set up baseline benchmark harness, KPI scorecard, and milestone governance." \
  "type:epic,priority:p0,area:benchmark" \
  "$M0"

create_issue_if_missing \
  "feat: Implement benchmark harness and baseline report generation" \
  "Add repeatable warm/cold scenario benchmarks and publish artifact reports." \
  "type:feature,priority:p0,area:benchmark" \
  "$M0"

create_issue_if_missing \
  "feat: Build comparator for artifact and diagnostics parity" \
  "Create dual-run comparator to detect correctness drift." \
  "type:feature,priority:p0,area:compiler" \
  "$M1"

create_issue_if_missing \
  "epic: Core build engine MVP" \
  "Sessionized compile daemon, stable API, local CAS, fallback path." \
  "type:epic,priority:p0,area:compiler" \
  "$M1"

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

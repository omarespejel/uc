#!/usr/bin/env bash
# Collect review-bot feedback for a PR as a single JSON document.
#
# Coding agents use this instead of scraping the PR web UI: it merges CodeRabbit and
# Qodo review comments, general comments, and the pre-merge/commit check states into one
# machine-readable object, and reports which threads are still unresolved.
#
# Usage:
#   scripts/pr_feedback.sh [PR_NUMBER] [--unresolved-only]
#
# With no PR number, resolves the PR for the current branch.
# Requires: gh (authenticated), jq.

set -euo pipefail

PR_NUMBER=""
UNRESOLVED_ONLY=0

for arg in "$@"; do
  case "$arg" in
    --unresolved-only) UNRESOLVED_ONLY=1 ;;
    -h|--help)
      sed -n '2,14p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      if [[ "$arg" =~ ^[0-9]+$ ]]; then
        PR_NUMBER="$arg"
      else
        echo "unknown argument: $arg" >&2
        exit 2
      fi
      ;;
  esac
done

for bin in gh jq; do
  if ! command -v "$bin" >/dev/null 2>&1; then
    echo "required tool not found: $bin" >&2
    exit 3
  fi
done

if [[ -z "$PR_NUMBER" ]]; then
  PR_NUMBER="$(gh pr view --json number --jq .number 2>/dev/null || true)"
  if [[ -z "$PR_NUMBER" ]]; then
    echo "no PR number given and no PR found for the current branch" >&2
    exit 2
  fi
fi

REPO="$(gh repo view --json nameWithOwner --jq .nameWithOwner)"

# Review threads carry the resolution state, which the plain comments API does not expose.
# shellcheck disable=SC2016  # GraphQL variables are literal, not shell expansions.
THREADS="$(gh api graphql -f query='
  query($owner:String!, $name:String!, $pr:Int!) {
    repository(owner:$owner, name:$name) {
      pullRequest(number:$pr) {
        reviewThreads(first:100) {
          nodes {
            isResolved
            isOutdated
            path
            line
            comments(first:20) {
              nodes { author { login } body url createdAt }
            }
          }
        }
      }
    }
  }' \
  -F owner="${REPO%%/*}" -F name="${REPO##*/}" -F pr="$PR_NUMBER" \
  --jq '.data.repository.pullRequest.reviewThreads.nodes')"

BOTS='["coderabbitai","coderabbitai[bot]","qodo-code-review","qodo-code-review[bot]","qodo-merge-pro","greptile-apps","greptile-apps[bot]"]'

STATUS="$(gh pr view "$PR_NUMBER" --json statusCheckRollup,mergeable,reviewDecision,title,url \
  --jq '{title,url,mergeable,reviewDecision,checks:[.statusCheckRollup[]? | {name:(.name // .context), status:(.status // "COMPLETED"), conclusion:(.conclusion // .state)}]}')"

# `gh --jq` takes only an expression, so filter with a real jq invocation to pass --argjson.
GENERAL="$(gh pr view "$PR_NUMBER" --json comments \
  | jq --argjson bots "$BOTS" \
      '[.comments[] | select(.author.login as $a | $bots | index($a)) | {author:.author.login, createdAt, body}]')"

jq -n \
  --argjson threads "$THREADS" \
  --argjson status "$STATUS" \
  --argjson general "$GENERAL" \
  --argjson bots "$BOTS" \
  --arg pr "$PR_NUMBER" \
  --argjson unresolved_only "$UNRESOLVED_ONLY" '
  {
    pr: ($pr | tonumber),
    title: $status.title,
    url: $status.url,
    review_decision: $status.reviewDecision,
    mergeable: $status.mergeable,
    checks: $status.checks,
    bot_threads: [
      $threads[]
      | select(.comments.nodes[0].author.login as $a | $bots | index($a))
      | select($unresolved_only == 0 or .isResolved == false)
      | {
          resolved: .isResolved,
          outdated: .isOutdated,
          path: .path,
          line: .line,
          bot: .comments.nodes[0].author.login,
          url: .comments.nodes[0].url,
          body: .comments.nodes[0].body,
          replies: [.comments.nodes[1:][] | {author: .author.login, body: .body}]
        }
    ],
    bot_summaries: $general
  }
  | . + {
      counts: {
        unresolved_bot_threads: ([.bot_threads[] | select(.resolved == false)] | length),
        total_bot_threads: (.bot_threads | length),
        failing_checks: ([.checks[] | select(.conclusion == "FAILURE" or .conclusion == "ERROR")] | length)
      }
    }
  '

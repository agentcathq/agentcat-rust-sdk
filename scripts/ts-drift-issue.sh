#!/usr/bin/env bash
# Opens or updates the one open `ts-drift` issue with the nightly findings.
#   scripts/ts-drift-issue.sh <summary.md>
# Needs GH_TOKEN (issues: write) and GITHUB_REPOSITORY. Opens nothing when the summary file is empty.
set -euo pipefail

summary="${1:?usage: ts-drift-issue.sh <summary.md>}"
if [ ! -s "$summary" ]; then
  echo "no drift: no issue opened"
  exit 0
fi

date_utc="$(date -u +%F)"
gh label create ts-drift --force --repo "$GITHUB_REPOSITORY" \
  --description "The TS SDK moved away from the pinned oracle commit" --color B60205 >/dev/null

existing="$(gh issue list --repo "$GITHUB_REPOSITORY" --label ts-drift --state open \
  --json number --jq '.[0].number // empty')"
if [ -n "$existing" ]; then
  gh issue comment "$existing" --repo "$GITHUB_REPOSITORY" --body-file "$summary"
  echo "updated issue #$existing"
else
  gh issue create --repo "$GITHUB_REPOSITORY" --label ts-drift \
    --title "TS drift: $date_utc" --body-file "$summary"
fi

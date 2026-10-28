#!/usr/bin/env bash
# Prints the base commit of the mutation diff gate (C53): `git diff "$(scripts/mutants-base.sh)"...HEAD`.
#
# In CI (GITHUB_ACTIONS=true): the head SHA of the newest successful run of ci.yml on the same branch (the
# head branch of a pull request), not counting this run. The `gh api` call needs GH_TOKEN and `actions: read`.
# With no such run, or when the commit is not in the local clone, the parent commit (HEAD^) and a message on
# stderr. Locally: origin/rewrite, or HEAD^ with a message when that ref is missing.
set -euo pipefail
cd "$(dirname "$0")/.."

# The empty tree: the base of a root commit, which has no parent.
empty_tree=4b825dc642cb6eb9a060e54bf8d69288fbe4e3d4

has_commit() {
  git cat-file -e "$1^{commit}" 2>/dev/null
}

parent() {
  if has_commit 'HEAD^'; then
    git rev-parse 'HEAD^'
  else
    echo "mutants-base: HEAD has no parent; using the empty tree" >&2
    echo "$empty_tree"
  fi
}

if [ "${GITHUB_ACTIONS:-}" = true ]; then
  if [ "${GITHUB_EVENT_NAME:-}" = pull_request ]; then
    branch="${GITHUB_HEAD_REF:?GITHUB_HEAD_REF is not set}"
  else
    branch="${GITHUB_REF_NAME:?GITHUB_REF_NAME is not set}"
  fi
  # Newest first. The current run is not a success yet, but a re-run of an older attempt can be listed: skip it.
  # A gh failure (rate limit, 403, outage) stops the script: the parent-commit fallback is only for a branch
  # with no green run (an empty answer), never for an error.
  if ! sha="$(gh api "repos/${GITHUB_REPOSITORY:?GITHUB_REPOSITORY is not set}/actions/workflows/ci.yml/runs" \
    -X GET -f branch="$branch" -f status=success -f per_page=30 \
    --jq "first(.workflow_runs[] | select(.id != ${GITHUB_RUN_ID:-0}) | .head_sha) // empty")"; then
    echo "mutants-base: the gh api call failed; stopping, because the gate cannot tell whether the branch has a green run" >&2
    exit 1
  fi
  if [ -z "$sha" ]; then
    echo "mutants-base: no successful ci.yml run on branch $branch; using the parent commit" >&2
    parent
  elif has_commit "$sha"; then
    echo "$sha"
  else
    echo "mutants-base: commit $sha of the last successful run is not in the clone; using the parent commit" >&2
    parent
  fi
else
  if has_commit origin/rewrite; then
    git rev-parse origin/rewrite
  else
    echo "mutants-base: origin/rewrite is missing; using the parent commit" >&2
    parent
  fi
fi

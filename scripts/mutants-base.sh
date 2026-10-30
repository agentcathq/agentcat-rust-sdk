#!/usr/bin/env bash
# Prints the base commit of the mutation diff gate: `git diff "$(scripts/mutants-base.sh)"...HEAD`.
#
# In CI (GITHUB_ACTIONS=true): the head SHA of the newest successful run of ci.yml, on any branch and not
# counting this run, whose commit is in the local clone and is an ancestor of HEAD. A green run of another
# branch counts: after a merge commit, the green tip of the merged branch is an ancestor of the merge. A
# green commit that HEAD does not descend from (a run from before a history reset, a branch that was never
# merged) is never a base, because `git diff base...HEAD` then has no merge base or covers the wrong lines.
# The `gh api` call needs GH_TOKEN and `actions: read`. With no such run, the parent commit (HEAD^) and a
# message on stderr. A `gh` error stops the script (exit 1).
#
# Locally: the upstream of the current branch (`@{upstream}`, for example origin/my-branch), else origin/main,
# else HEAD^ with a message on stderr.
set -euo pipefail
cd "$(dirname "$0")/.."

# The empty tree: the base of a root commit, which has no parent.
empty_tree=4b825dc642cb6eb9a060e54bf8d69288fbe4e3d4
# GitHub lists at most 1000 runs for a filtered query: 10 pages of 100.
max_pages=10

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
  repo="${GITHUB_REPOSITORY:?GITHUB_REPOSITORY is not set}"
  page=1
  while [ "$page" -le "$max_pages" ]; do
    # Newest first, every branch. The current run is not a success yet, but a re-run of an older attempt can
    # be listed: skip it. A gh failure (rate limit, 403, outage) stops the script: the parent-commit fallback
    # is only for a history with no green ancestor (an empty answer), never for an error.
    if ! runs="$(gh api "repos/$repo/actions/workflows/ci.yml/runs" \
      -X GET -f status=success -f per_page=100 -f page="$page" \
      --jq '.workflow_runs[] | "\(.id) \(.head_sha)"')"; then
      echo "mutants-base: the gh api call failed; stopping, because the gate cannot tell which green commit HEAD descends from" >&2
      exit 1
    fi
    if [ -z "$runs" ]; then
      break
    fi
    while read -r id sha; do
      if [ "$id" = "${GITHUB_RUN_ID:-0}" ]; then
        continue
      fi
      if has_commit "$sha" && git merge-base --is-ancestor "$sha" HEAD; then
        echo "$sha"
        exit 0
      fi
    done <<<"$runs"
    page=$((page + 1))
  done
  echo "mutants-base: no successful ci.yml run is on a commit in the clone that HEAD descends from; using the parent commit" >&2
  parent
else
  if upstream="$(git rev-parse --abbrev-ref --symbolic-full-name '@{upstream}' 2>/dev/null)" && has_commit "$upstream"; then
    git rev-parse "$upstream"
  elif has_commit origin/main; then
    git rev-parse origin/main
  else
    echo "mutants-base: the branch has no upstream and origin/main is missing; using the parent commit" >&2
    parent
  fi
fi

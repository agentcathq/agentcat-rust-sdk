#!/usr/bin/env bash
# Writes the diff of the mutation diff gate to target/mutants.diff: the changes from the merge base of HEAD
# and the base that scripts/mutants-base.sh prints. The diff compares the working tree with that merge base,
# so locally uncommitted changes to tracked or staged files count; in CI the working tree is HEAD.
#
# A commit with no parent and no green ancestor (the first commit of a repository) has the empty tree as its
# base. Its diff is empty, with a message on stderr: the gate then finds no mutant and passes, and the nightly
# run mutates the whole crate. A failure of scripts/mutants-base.sh stops the script with its exit code.
set -euo pipefail
cd "$(dirname "$0")/.."

# The empty tree, which scripts/mutants-base.sh prints for a commit with no parent.
empty_tree=4b825dc642cb6eb9a060e54bf8d69288fbe4e3d4

base="$(scripts/mutants-base.sh)"
mkdir -p target
if [ "$base" = "$empty_tree" ]; then
  echo "mutants-diff: HEAD has no parent, so the diff is empty; the nightly mutation run covers the whole crate" >&2
  : > target/mutants.diff
  exit 0
fi
echo "mutants-diff: base commit $base" >&2
git diff "$(git merge-base "$base" HEAD)" > target/mutants.diff

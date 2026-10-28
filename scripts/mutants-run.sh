#!/usr/bin/env bash
# Runs `cargo mutants` with the given arguments and maps its exit code (C53 ruling: a timed-out mutant counts
# as detected). Exit codes of cargo-mutants: 0 all caught, 1 usage error, 2 a mutant was missed, 3 a mutant
# timed out, 4 the unmutated tree fails its tests. MUTANTS_OK_CODES (default "0 3") lists the codes that
# pass; every other code is returned as it is. The diff gate (ci.yml, scripts/check.sh) uses the default;
# the nightly shards also pass 2, because `mutants-score` judges the missed mutants of all shards together.
# MUTANTS_BIN replaces `cargo` (a test stub).
set -uo pipefail
code=0
"${MUTANTS_BIN:-cargo}" mutants "$@" || code=$?
for ok in ${MUTANTS_OK_CODES:-0 3}; do
  if [ "$code" -eq "$ok" ]; then
    if [ "$code" -ne 0 ]; then
      echo "mutants-run: cargo mutants exited $code, which passes here"
    fi
    exit 0
  fi
done
echo "mutants-run: cargo mutants exited $code" >&2
exit "$code"

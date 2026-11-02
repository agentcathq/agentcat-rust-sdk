#!/usr/bin/env bash
# Runs `cargo mutants` with the given arguments and maps its exit code (a timed-out mutant counts as
# detected). Exit codes of cargo-mutants: 0 all caught, 1 usage error, 2 a mutant was missed, 3 a mutant
# timed out, 4 the unmutated tree fails its tests. MUTANTS_OK_CODES (default "0 3") lists the codes that
# pass; every other code is returned as it is. The diff gate (ci.yml, scripts/check.sh) uses the default;
# the nightly shards also pass 2, because `mutants-score` judges the missed mutants of all shards together.
# A run that missed a mutant fails with exit 2 whatever the exit code, unless 2 is one of MUTANTS_OK_CODES:
# cargo-mutants reports a timeout (3) before a miss (2), so exit 3 can hide a missed mutant. The missed
# mutants are the lines of mutants.out/missed.txt in the current directory, where cargo-mutants writes them
# when it gets no output directory; a missing file lists none. So the script refuses -o/--output and
# CARGO_MUTANTS_OUTPUT (exit 1): with another output directory, a missed mutant would pass without a word.
# cargo-mutants moves an earlier mutants.out to mutants.out.old when it starts, but writes nothing when it has
# nothing to do (an empty diff), so the script makes that move first: an earlier run's missed.txt never
# counts as this run's.
# MUTANTS_BIN replaces `cargo` (a test stub).
set -uo pipefail
ok_codes="${MUTANTS_OK_CODES:-0 3}"
missed=mutants.out/missed.txt
refuse_output() {
  echo "mutants-run: refusing $1: the script reads the missed mutants from $missed in the current directory" >&2
  exit 1
}
if [ -n "${CARGO_MUTANTS_OUTPUT:-}" ]; then
  refuse_output CARGO_MUTANTS_OUTPUT
fi
for arg in "$@"; do
  case "$arg" in
    --) break ;;
    -o | -o* | --output | --output=*) refuse_output "$arg" ;;
  esac
done
if [ -d mutants.out ]; then
  rm -rf mutants.out.old
  mv mutants.out mutants.out.old
fi
code=0
"${MUTANTS_BIN:-cargo}" mutants "$@" || code=$?
passes=0
misses_pass=0
for ok in $ok_codes; do
  if [ "$code" -eq "$ok" ]; then
    passes=1
  fi
  if [ "$ok" -eq 2 ]; then
    misses_pass=1
  fi
done
if [ "$passes" -eq 0 ]; then
  echo "mutants-run: cargo mutants exited $code" >&2
  exit "$code"
fi
if [ "$misses_pass" -eq 0 ] && [ -s "$missed" ]; then
  echo "mutants-run: cargo mutants exited $code, but it missed $(grep -c '' "$missed") mutant(s):" >&2
  cat "$missed" >&2
  exit 2
fi
if [ "$code" -ne 0 ]; then
  echo "mutants-run: cargo mutants exited $code, which passes here"
fi
exit 0

#!/usr/bin/env bash
# Runs the pull-request gates locally.
# Usage: scripts/check.sh [--full] [--mutants] [--bless-public-api]
# --mutants mutates the lines changed since MUTANTS_BASE (default origin/main);
# set it to the pull request's base branch. CARGO_MUTANTS_JOBS sets parallelism.
set -euo pipefail
cd "$(dirname "$0")/.."
export AGENTCAT_DISABLE_DIAGNOSTICS=1 DISABLE_DIAGNOSTICS=1
# public-api.txt comes from this nightly (rustdoc JSON drifts between nightlies).
# The public-api job in .github/workflows/ci.yml uses the same one.
public_api_toolchain=nightly-2026-10-18
full=0
mutants=0
bless=0
for arg in "$@"; do
  case "$arg" in
    --full) full=1 ;;
    --mutants) mutants=1 ;;
    --bless-public-api) bless=1 ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done
step() { printf '\n== %s\n' "$*"; }

step fmt
cargo fmt --check
step clippy
cargo clippy --lib --all-features -- -D warnings
cargo clippy --lib -- -D warnings
cargo clippy --lib --no-default-features --features rustls -- -D warnings
cargo clippy --all-targets --all-features -- -D warnings
step test
cargo test --all-targets
cargo test --doc
step docs
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
step deny
cargo deny check
step public-api
if [ "$bless" = 1 ]; then
  # Next to the target, so that mv is an atomic rename.
  snapshot="$(mktemp public-api.txt.XXXXXX)"
  trap 'rm -f "$snapshot"' EXIT
  cargo "+$public_api_toolchain" public-api --simplified > "$snapshot"
  chmod 644 "$snapshot"
  mv "$snapshot" public-api.txt
fi
cargo "+$public_api_toolchain" public-api --simplified | diff -u public-api.txt -
if [ "$full" = 1 ]; then
  step msrv
  cargo +1.88 check --lib --all-features
  step minimal-versions
  cargo +nightly minimal-versions check --direct --lib --all-features
  step features
  cargo hack check --lib --feature-powerset --depth 2 --exclude-features default,exporters --at-least-one-of rustls,native-tls
  step coverage
  cargo llvm-cov --all-targets --summary-only --fail-under-lines "$(cat scripts/coverage-floor)"
fi
if [ "$mutants" = 1 ]; then
  step mutants
  mkdir -p target
  git diff "${MUTANTS_BASE:-origin/main}...HEAD" -- src > target/pr.diff
  if [ -s target/pr.diff ]; then
    code=0
    cargo mutants --in-diff target/pr.diff --output target/mutants || code=$?
    # 0 all caught, 2 some missed, 3 some timed out: the score decides.
    case $code in 0|2|3) ;; *) exit "$code" ;; esac
    python3 scripts/mutants-score.py --min-core 95 --min-adapter 95 target/mutants/mutants.out
  else
    echo "no changes under src/ since ${MUTANTS_BASE:-origin/main}"
  fi
fi

#!/usr/bin/env bash
# Runs the pull-request gates locally.
# Usage: scripts/check.sh [--full] [--bless-public-api]
set -euo pipefail
cd "$(dirname "$0")/.."
export AGENTCAT_DISABLE_DIAGNOSTICS=1 DISABLE_DIAGNOSTICS=1
# public-api.txt comes from this nightly (rustdoc JSON drifts between nightlies).
# The public-api job in .github/workflows/ci.yml uses the same one.
public_api_toolchain=nightly-2026-10-18
full=0
bless=0
for arg in "$@"; do
  case "$arg" in
    --full) full=1 ;;
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
  snapshot="$(mktemp)"
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
fi

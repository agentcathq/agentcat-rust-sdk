#!/usr/bin/env bash
# Local gate: exit 0 means green. It runs the commands of every job in .github/workflows/ci.yml.
set -euo pipefail
export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")/.."

# A gate that skips a check is not a gate: a missing tool or toolchain stops it here.
missing=0
need() { # need <install command> <probe command...>
  local install="$1"
  shift
  if ! "$@" >/dev/null 2>&1; then
    echo "install with: $install"
    missing=1
  fi
}
host="$(rustc -vV | sed -n 's/^host: //p')"
has_toolchain() { rustup toolchain list | grep -Eq "^$1-$host( |$)"; }
need "cargo install cargo-nextest --locked" cargo nextest --version
need "cargo install cargo-hack --locked" cargo hack --version
need "cargo install cargo-minimal-versions --locked" cargo minimal-versions --version
need "cargo install cargo-deny --locked" cargo deny --version
need "rustup toolchain install 1.88" has_toolchain '1\.88'
need "rustup toolchain install nightly" has_toolchain nightly
need "Node 22 (it includes corepack): the oracle tests need it" command -v node
need "Node 22 (it includes corepack): the oracle tests need it" command -v corepack
if [ "$missing" -ne 0 ]; then
  exit 1
fi

# fmt, clippy, hack
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo hack clippy --workspace --each-feature --no-dev-deps -- -D warnings

# oracle-harness (the parity check runs after the tests below)
corepack pnpm install --frozen-lockfile
corepack pnpm -r --workspace-concurrency=1 test
node oracle/src/vectors/run.mjs --check
node oracle/src/parity/list-tests.mjs --check
node harness/src/dump-contract.mjs --check

# test (R07 in a build of the SDK alone: another workspace member could turn the feature on)
cargo nextest run --workspace --all-features
cargo test -p agentcat-sdk --all-features --test json_key_order
cargo test --workspace --doc
cargo run -q -p xtask -- parity

# msrv
cargo +1.88 nextest run --workspace --all-features
cargo +1.88 test -p agentcat-sdk --all-features --test json_key_order

# docs
RUSTDOCFLAGS="--cfg docsrs -D warnings" cargo +nightly doc --workspace --all-features --no-deps

# deny
cargo deny check

# minimal-versions
cargo +nightly minimal-versions check --direct --workspace

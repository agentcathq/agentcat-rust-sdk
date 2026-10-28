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
node_hint='Node.js 22 or later. Node 22 to 24 include corepack; on Node 25 and later, run `npm install -g corepack`.'
need "$node_hint" command -v node
need "$node_hint" command -v corepack
if [ "$missing" -ne 0 ]; then
  exit 1
fi

# fmt, clippy, hack
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo hack clippy --workspace --each-feature --no-dev-deps -- -D warnings

# oracle-harness (the parity check runs after the tests below)
corepack pnpm install --frozen-lockfile
# the harness tests run the Rust reference server
cargo build -p harness-ref-server
corepack pnpm -r --workspace-concurrency=1 test
node oracle/src/vectors/run.mjs --check
node oracle/src/parity/list-tests.mjs --check
node harness/src/dump-contract.mjs --check
# scripts/ tests never contact the network (rmcp-versions.mjs fetches only when run as a script)
node --test "scripts/*.test.mjs"
# the customer examples (C54) run against the local mock API; stdout carries only JSON-RPC, the last event
# arrives, and the process exits 0 within 5 s of the client close
cargo build -p agentcat-sdk --examples --all-features
for example in stdio streamable_http hooks; do
  node harness/src/examples-check.mjs "$example"
done

# harness-l3 (the Rust reference server is built above): the MCP-wire delta and the published events
# against TS v2 on every scenario, transport and lane (every tracked Rust server must drain); an A/A run
# of the Rust server, which must show no delta or event difference at all, ruled or not; and three canary
# runs, whose perturbed second server must be caught in every combination: the context canary
# (HARNESS_CANARY=1) and the call canary (HARNESS_CANARY=call) in the delta and nowhere in the events, the
# event canary (HARNESS_CANARY=event) in the events and nowhere in the delta. harness/src/l3-expect.mjs
# checks harness/out/matrix.json after each of the last four.
node harness/src/matrix.mjs
node harness/src/matrix.mjs --servers rust,rust --scenarios basic
node harness/src/l3-expect.mjs none
canary=0
node harness/src/matrix.mjs --servers rust,rust --scenarios basic --env2 HARNESS_CANARY=1 || canary=$?
if [ "$canary" -ne 1 ]; then
  echo "L3 canary failed: the matrix exited $canary, expected 1"
  exit 1
fi
node harness/src/l3-expect.mjs caught
for canary in call event; do
  status=0
  node harness/src/matrix.mjs --servers rust,rust --scenarios basic --env2 "HARNESS_CANARY=$canary" || status=$?
  if [ "$status" -ne 1 ]; then
    echo "L3 $canary canary failed: the matrix exited $status, expected 1"
    exit 1
  fi
  node harness/src/l3-expect.mjs "caught-$canary"
done

# test (R07 in a build of the SDK alone: another workspace member could turn the feature on)
cargo nextest run --workspace --all-features
# the tests that compile only without a TLS feature (C44's warning for an https: URL)
cargo nextest run -p agentcat-sdk --no-default-features
cargo test -p agentcat-sdk --all-features --test json_key_order
cargo test --workspace --doc
cargo run -q -p xtask -- parity --final
cargo run -q -p xtask -- rmcp-canary

# msrv (warnings denied: an older rustc can warn where the current one does not)
RUSTFLAGS="-D warnings" cargo +1.88 nextest run --workspace --all-features
RUSTFLAGS="-D warnings" cargo +1.88 test -p agentcat-sdk --all-features --test json_key_order

# docs
RUSTDOCFLAGS="--cfg docsrs -D warnings" cargo +nightly doc --workspace --all-features --no-deps
# a second run checks the doc links of the crate-private items, which the first run skips
RUSTDOCFLAGS="--cfg docsrs -D warnings" cargo +nightly doc --workspace --all-features --no-deps --document-private-items

# deny
cargo deny --all-features check

# minimal-versions
cargo +nightly minimal-versions check --direct --workspace
# The serde_json floor (R22): the feature self-check finds no feature in a build of the SDK alone on
# serde_json 1.0.127, the workspace's floor. serde_with 3.15 and later need a newer serde_json, so
# serde_with goes down to 3.14.1 first. The pins go on Cargo.lock, which is restored after.
lock_copy="$(mktemp)"
cp Cargo.lock "$lock_copy"
trap 'cp "$lock_copy" Cargo.lock; rm -f "$lock_copy"' EXIT
cargo update -p serde_with --precise 3.14.1
cargo update -p serde_json --precise 1.0.127
cargo +stable nextest run -p agentcat-sdk --all-features --lib --no-tests=fail -E 'test(=delivery::tests::json_features_empty_in_this_build)'
cp "$lock_copy" Cargo.lock
rm -f "$lock_copy"
trap - EXIT

# rmcp-versions: the current rmcp is the one in Cargo.lock; CI repeats the tests for every 3.x minor
# (job `rmcp`, R25). Run `node scripts/rmcp-versions.mjs` (network) to see the list.
cargo build -p agentcat-sdk --examples --all-features
cargo test -p agentcat-sdk --doc --all-features
RMCP_EXPECTED_VERSION="$(sed -n '/^name = "rmcp"$/{n;s/^version = "\(.*\)"$/\1/p;q}' Cargo.lock)"
export RMCP_EXPECTED_VERSION
cargo nextest run -p agentcat-sdk --test rmcp_version
unset RMCP_EXPECTED_VERSION

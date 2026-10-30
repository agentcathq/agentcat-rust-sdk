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
has_component() { rustup component list --installed | grep -Eq "^$1-$host$"; }
need "cargo install cargo-nextest --locked" cargo nextest --version
need "cargo install cargo-hack --locked" cargo hack --version
need "cargo install cargo-minimal-versions --locked" cargo minimal-versions --version
need "cargo install cargo-deny --locked" cargo deny --version
need "cargo install cargo-public-api@0.52.0 --locked" cargo public-api --version
need "cargo install cargo-llvm-cov@0.9.1 --locked" cargo llvm-cov --version
need "cargo install cargo-mutants@27.1.0 --locked" cargo mutants --version
need "rustup component add llvm-tools-preview" has_component 'llvm-tools(-preview)?'
need "rustup toolchain install 1.88" has_toolchain '1\.88'
need "rustup toolchain install nightly" has_toolchain nightly
# cargo-public-api reads the rustdoc JSON of this dated nightly (TOOLCHAIN in xtask/src/public_api.rs)
need "rustup toolchain install nightly-2026-10-18" has_toolchain 'nightly-2026-10-18'
need "install jq with your package manager (for example apt-get install jq)" jq --version
if [ "$missing" -ne 0 ]; then
  exit 1
fi

# fmt, clippy, hack
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo hack clippy --workspace --each-feature --no-dev-deps -- -D warnings

# the customer examples, each against a local mock API: stdout carries only JSON-RPC, the last event
# arrives, and the process exits 0 within 5 s of the client close
cargo run -q -p xtask -- examples-check --all

# test; the key-order test runs again in a build of the SDK alone, where no other workspace member can
# turn serde_json's preserve_order on
cargo nextest run --workspace --all-features
# the tests that compile only without a TLS feature (the warning for an https: URL)
cargo nextest run -p agentcat-sdk --no-default-features
cargo test -p agentcat-sdk --all-features --test json_key_order
cargo test --workspace --doc
cargo run -q -p xtask -- rmcp-canary

# coverage: at least 90 % of the crate's lines and 95 % of the pure modules' lines, test files left out
cargo llvm-cov nextest -p agentcat-sdk --all-features --ignore-filename-regex '(/tests\.rs$|/test_support/|/tests/)' \
  --json --summary-only --output-path target/coverage.json
cargo run -q -p xtask -- coverage target/coverage.json

# mutants: no surviving mutant in the lines that differ from the upstream of the current branch (for
# example origin/my-branch), or from origin/main when the branch has none (scripts/mutants-base.sh). The diff
# compares the working tree with the merge base, so uncommitted changes to tracked files and to files staged
# with `git add` count (an untracked, unstaged file does not). A timed-out mutant counts as detected
# (scripts/mutants-run.sh); a missed mutant fails. An empty diff, or one with no mutable SDK source, finds no
# mutant and passes. CI runs the same gate against the newest green commit that HEAD descends from.
mkdir -p target
git diff "$(git merge-base "$(scripts/mutants-base.sh)" HEAD)" > target/mutants.diff
scripts/mutants-run.sh --in-diff target/mutants.diff

# msrv (warnings denied: an older rustc can warn where the current one does not)
RUSTFLAGS="-D warnings" cargo +1.88 nextest run --workspace --all-features
RUSTFLAGS="-D warnings" cargo +1.88 test -p agentcat-sdk --all-features --test json_key_order
# the README, the crate guide and the rustdoc examples that a person copies must compile on 1.88
RUSTFLAGS="-D warnings" cargo +1.88 test --workspace --doc --all-features

# docs
RUSTDOCFLAGS="--cfg docsrs -D warnings" cargo +nightly doc --workspace --all-features --no-deps
# a second run checks the doc links of the crate-private items, which the first run skips
RUSTDOCFLAGS="--cfg docsrs -D warnings" cargo +nightly doc --workspace --all-features --no-deps --document-private-items

# api: the public API against agentcat-sdk/public-api.txt and the crate allowlist, on the pinned
# nightly-2026-10-18; the package holds only the expected files; the publish dry run contacts crates.io but
# uploads nothing
cargo run -q -p xtask -- public-api
cargo run -q -p xtask -- package-list
cargo publish -p agentcat-sdk --dry-run --allow-dirty

# deny
cargo deny --all-features check

# minimal-versions
cargo +nightly minimal-versions check --direct --workspace
# The serde_json floor: the feature self-check finds no feature in a build of the SDK alone on
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
# (job `rmcp`). Run `cargo run -q -p xtask -- rmcp-versions` (network) to see the list.
cargo build -p agentcat-sdk --examples --all-features
cargo test -p agentcat-sdk --doc --all-features
RMCP_EXPECTED_VERSION="$(sed -n '/^name = "rmcp"$/{n;s/^version = "\(.*\)"$/\1/p;q}' Cargo.lock)"
export RMCP_EXPECTED_VERSION
cargo nextest run -p agentcat-sdk --test rmcp_version
unset RMCP_EXPECTED_VERSION

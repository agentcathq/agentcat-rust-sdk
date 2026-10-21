#!/usr/bin/env bash
# Local gate: exit 0 means green.
set -euo pipefail
export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")/.."

if ! cargo nextest --version >/dev/null 2>&1; then
  echo "install with: cargo install cargo-nextest --locked"
  exit 1
fi

cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
if ! command -v node >/dev/null 2>&1 || ! command -v corepack >/dev/null 2>&1; then
  echo "install Node 22 (it includes corepack): the oracle tests need it"
  exit 1
fi

corepack pnpm install --frozen-lockfile
corepack pnpm -r --workspace-concurrency=1 test
node oracle/src/vectors/run.mjs --check
node oracle/src/parity/list-tests.mjs --check
node harness/src/dump-contract.mjs --check
cargo nextest run --workspace --all-features
cargo run -q -p xtask -- parity
cargo test --workspace --doc
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps
cargo deny check
cargo +1.88 check --workspace --all-targets

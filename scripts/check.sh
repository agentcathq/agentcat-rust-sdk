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
cargo nextest run --workspace --all-features
cargo run -q -p xtask -- parity
cargo test --workspace --doc
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps
cargo deny check
cargo +1.88 check --workspace --all-targets

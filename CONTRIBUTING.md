# Contributing

This repository holds the AgentCat Rust SDK (`agentcat-sdk`) and the tools that compare it with the
TypeScript SDK. This file tells you how to set up a checkout and run the checks.

## Prerequisites

Install these tools:

- Rust stable and Rust 1.88, the minimum supported version: `rustup toolchain install stable 1.88`
- Rust nightly, for the docs build and the minimal-versions check: `rustup toolchain install nightly`
- The cargo tools of the gate:
  `cargo install cargo-nextest cargo-hack cargo-minimal-versions cargo-deny --locked`
- Node.js 22 or later. Node 22 to 24 include corepack; on Node 25 and later, run `npm install -g corepack`.
  The tests use Node to run the TypeScript SDK.

If a tool is missing, `scripts/check.sh` stops and prints the install command.

## First run

1. Install the JavaScript dependencies: `corepack pnpm install`
2. Clone and build the TypeScript SDK: `node oracle/src/fetch-ts.mjs`

Step 2 clones the TypeScript SDK at the commit in `oracle/oracle.lock` into `oracle/.cache/`. Then it
builds the clone with the pnpm version that the TypeScript repository pins. The first build takes
a few minutes. The tests do this step too when the build is missing.

Do not point the tools at another checkout of the TypeScript SDK. They use only their own clean
clone at the pinned commit.

## Run the checks

Run `scripts/check.sh`. Exit 0 means green. The script runs the commands of every job in
`.github/workflows/ci.yml`: formatting, clippy, the feature matrix, the JavaScript tests, the checks
of the generated files, the Rust tests on stable and on 1.88, the parity check, the docs build,
cargo-deny and the minimal-versions check.

For a shorter loop, run one part, for example `cargo test`, `cargo nextest run --workspace` or
`corepack pnpm --filter harness test`.

Some files are generated: `agentcat-sdk/tests/vectors/`, `parity/ts-tests.json` and
`harness/contract/`. Do not edit them by hand. Each generator has a `--check` mode, and the gate
runs it.

## Tests never contact production

No test, harness run or oracle run may contact AgentCat production (`api.agentcat.com`,
`otel.agentcat.com` or any other production host).

- The TypeScript reference servers send events only to a local mock API. They refuse to start
  when `AGENTCAT_API_URL` is not a local URL.
- The egress guard (`oracle/src/egress-guard.mjs`) refuses connections to hosts other than
  loopback in the TypeScript servers and in `vitest list`.
- If a new test needs a network service, start the service on `127.0.0.1` in the test.

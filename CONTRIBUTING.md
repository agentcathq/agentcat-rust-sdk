# Contributing

This repository holds the AgentCat Rust SDK (`agentcat-sdk`) and the tools that compare it with the
TypeScript SDK. This file describes the repository and how the SDK proves parity with TypeScript,
and tells you how to set up a checkout and run the checks. The [README](README.md) is the page of
the crate for its users.

Read these before a change:

- [`docs/DECISIONS.md`](docs/DECISIONS.md): the decisions of the owner and of the design.
- [The design](docs/superpowers/specs/2026-10-20-rust-sdk-design.md), with the deviation register
  in section 9.

## What the repository holds

| Path | What it holds |
|---|---|
| [`README.md`](README.md) | The README of the crate. GitHub and crates.io show it, and its Rust code blocks compile as doctests (`ReadmeDoctests` in `agentcat-sdk/src/lib.rs`). `agentcat-sdk/README.md` is a symlink to it, and `cargo package` puts a copy of the file in the package. |
| [`agentcat-sdk/`](agentcat-sdk) | The published crate: the SDK, its tests, its examples and the CHANGELOG. |
| [`xtask/`](xtask) | Repository tasks in Rust, run as `cargo run -p xtask -- <task>`: the parity check (`parity`), the check that the SDK forwards every method of `rmcp`'s `ServerHandler` (`rmcp-canary`), the public API check against `agentcat-sdk/public-api.txt` (`public-api`), the check of the files in the published package (`package-list`) and the line coverage floors (`coverage`). |
| [`oracle/`](oracle) | Node tools that build the TypeScript SDK at a pinned commit and run its modules: the test vectors, the property-test oracle and the list of TypeScript tests. |
| [`harness/`](harness) | The differential harness: TypeScript and Rust reference servers, one scripted MCP client, a mock AgentCat API, and the rules that compare two runs. |
| [`parity/`](parity) | The list of every TypeScript test (`ts-tests.json`, generated) and the map from each test to its Rust test, harness scenario or reason (`manifest.toml`). |
| [`scripts/`](scripts) | `check.sh`, the local gate, which runs the commands of every CI job. `mutants-base.sh` and `mutants-run.sh` pick the base commit of the mutation check and run it. `rmcp-versions.mjs` lists the rmcp versions that CI tests. `ts-drift-issue.sh` opens the issue of the nightly TypeScript drift check. |
| [`docs/`](docs) | The decision log, the research, the design, the plans, the report of the dashboard demo, and the images of the README (`docs/static/`, `docs/cats/`). |

## How the SDK proves parity with TypeScript

The TypeScript SDK at the commit that `oracle/oracle.lock` pins is the answer key. No expected
value in a test comes from the Rust output. Four layers compare the two SDKs.

### L1: test vectors

The oracle runs the real TypeScript modules on fixed inputs and writes the results to JSON files in
`agentcat-sdk/tests/vectors/`, with a SHA-256 manifest. The Rust tests run the pure functions of the
SDK on the same inputs and compare the results. The vectors cover constants and texts, IDs and
session handles, injected schemas with their key order, token estimates, `JSON.stringify` output,
UTF-16 cuts, tags, normalization, truncation, sanitization and redaction. Nobody edits a vector file
by hand, and the gate regenerates them to make sure that they are current.

### L2: property tests

The same functions run on random inputs. A long-running Node process runs the TypeScript module for
each input, and the Rust test compares the two answers. Each push runs 256 cases, and the nightly
run runs 10 000. The repository keeps the seeds of past failures.

### L3: differential harness

A TypeScript reference server with the real TypeScript SDK and a Rust reference server with
`agentcat-sdk` serve the same tools. One scripted MCP client drives both through the same scenarios.
It uses stdio and streamable HTTP (with sessions, without sessions, and with server-sent events),
and the protocol versions 2025-11-25 and 2026-07-28. A mock API records every event. The harness
compares what tracking changes on the MCP wire and the published events. Each allowed difference
has a ruling that names its entry in the deviation register. An A/A run and three canary runs make
sure that the comparison still finds differences.

### L4: contract

Every published body must match a pinned copy of the request schema of the AgentCat API. A local
replay also sends the bodies through the ingest function of the AgentCat backend.

### The parity manifest

The parity manifest maps every TypeScript test to a Rust test, a vector file, a harness scenario or
a reason why it does not apply. The gate fails on a test without a mapping. Every known
difference from TypeScript has a row in the deviation register of the design.

## Prerequisites

Install these tools:

- Rust stable and Rust 1.88, the minimum supported version: `rustup toolchain install stable 1.88`
- Rust nightly, for the docs build and the minimal-versions check: `rustup toolchain install nightly`
- The dated nightly of the public API check: `rustup toolchain install nightly-2026-10-18`
- The cargo tools of the gate:
  `cargo install cargo-nextest cargo-hack cargo-minimal-versions cargo-deny --locked`
- The cargo tools of the public API, coverage and mutation checks:
  `cargo install cargo-public-api@0.52.0 cargo-llvm-cov@0.9.1 cargo-mutants@27.1.0 --locked`.
  cargo-public-api reads the rustdoc JSON of nightly-2026-10-18 (`TOOLCHAIN` in
  `xtask/src/public_api.rs`). The toolchain is pinned, because a newer nightly can change the
  rustdoc JSON format. To bump the toolchain or cargo-public-api, change the toolchain in
  `xtask/src/public_api.rs`, in the `api` job of `.github/workflows/ci.yml` and in
  `scripts/check.sh` together, and the cargo-public-api version in `ci.yml`, `check.sh` and this
  file. Then run `cargo run -p xtask -- public-api`. A test of the xtask fails when the toolchain
  names differ. cargo-llvm-cov also needs `rustup component add llvm-tools-preview`.
  After an intended change to the public API, run `cargo run -p xtask -- public-api --bless`.
  An rmcp bump changes the `Tracked` `ServerHandler` lines of the snapshot, so it needs a new bless.
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
`.github/workflows/ci.yml`, in this order:

1. Formatting, clippy, and clippy on each feature alone (cargo-hack).
2. The JavaScript tests, the checks of the generated files and the tests of `scripts/`.
3. The examples check, and the dashboard demo against the local mock API
   (`node harness/demo/run.mjs --mock`).
4. The harness matrix, an A/A run and three canary runs.
5. The Rust tests with all features and with no default features, the doctests, the parity check
   and the rmcp canary (`cargo run -p xtask -- rmcp-canary`).
6. The coverage floors (cargo-llvm-cov and `cargo run -p xtask -- coverage`).
7. The mutation check.
8. The Rust tests and the doctests on 1.88.
9. The docs build on nightly.
10. The public API check (`cargo run -p xtask -- public-api`), the check of the package files
    (`cargo run -p xtask -- package-list`) and a publish dry run. The dry run contacts crates.io
    but uploads nothing.
11. cargo-deny.
12. The minimal-versions check, and the serde_json floor: one test on serde_json 1.0.127, the
    lowest version that the crate accepts.
13. The build of the examples, the doctests and the rmcp version test on the rmcp of `Cargo.lock`.
    CI repeats them on every rmcp 3.x minor.

The examples check runs the examples of `agentcat-sdk/examples/` against a local mock API. It fails
when a line on stdout is not JSON-RPC, when the event of a call does not arrive, or when the example
does not exit within 5 s after the client closes stdin or sends SIGTERM. To run it for one example,
run `cargo build -p agentcat-sdk --examples --all-features`, then
`node harness/src/examples-check.mjs stdio`.

The harness matrix compares the Rust reference server with the TypeScript one on every scenario,
transport and protocol lane. The transports are `stdio`, `http` (MCP sessions, JSON response mode),
`http-stateless` (no sessions, JSON response mode) and `http-sse` (MCP sessions, server-sent events
response mode, the default of `rmcp`). Run it with `node harness/src/matrix.mjs`.
`harness/README.md` describes the harness.

The mutation check runs `cargo mutants --in-diff` (cargo-mutants 27.1.0) on the lines that differ
from a base commit. A mutant is a small change to the SDK source, such as `>` to `>=`. A mutant that
the tests do not catch is missed, and one missed mutant fails the check. A mutant that times out
counts as detected. The settings are in `.cargo/mutants.toml`.

`scripts/mutants-base.sh` prints the base:

- Locally, the base is the upstream of the current branch (for example `origin/rewrite`), or
  `origin/main` when the branch has no upstream. The diff starts at the merge base of that commit
  and HEAD, and it includes uncommitted changes to tracked or staged files.
- In CI, the base is the newest commit with a successful CI run, on any branch, that HEAD descends
  from. When no such commit exists, the base is the parent commit.

Merge a pull request with a merge commit or a fast-forward, not a squash. A squash leaves no green
commit that HEAD descends from. The check then mutates the whole squashed change (on the first
merge into `main`, the whole crate), and the job runs out of time.

The nightly run mutates the whole crate in 8 shards and fails below 90 % detected (caught or timed
out). To run the check by hand, run the commands of `scripts/check.sh`:

```sh
mkdir -p target
git diff "$(git merge-base "$(scripts/mutants-base.sh)" HEAD)" > target/mutants.diff
scripts/mutants-run.sh --in-diff target/mutants.diff
```

Read `mutants.out/missed.txt` for the missed mutants.

For a shorter loop, run one part, for example `cargo nextest run -p agentcat-sdk --all-features`,
`cargo nextest run --workspace` or `corepack pnpm --filter harness test`. Run the Rust tests with
nextest, not `cargo test`: the tests of the global logger need one process per test. For the
doctests, run `cargo test --workspace --doc`.

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

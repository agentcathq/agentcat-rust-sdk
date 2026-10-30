# Contributing

This repository holds the AgentCat Rust SDK (`agentcat-sdk`), which adds AgentCat analytics to an
`rmcp` MCP server, and the tools that check it. The [README](README.md) is the page of the crate
for its users. This file tells you how the repository is laid out, how its tests work, and how to
run the checks.

## What the repository holds

| Path | What it holds |
|---|---|
| [`README.md`](README.md) | The README of the crate. GitHub and crates.io show it, and its Rust code blocks compile as doctests (`ReadmeDoctests` in `agentcat-sdk/src/lib.rs`). `agentcat-sdk/README.md` is a symlink to it. |
| [`agentcat-sdk/`](agentcat-sdk) | The published crate: the SDK source, its tests, its examples and the CHANGELOG. |
| [`xtask/`](xtask) | Repository tasks, run as `cargo run -q -p xtask -- <task>`: `rmcp-canary` (the SDK forwards every method of `rmcp`'s `ServerHandler`), `public-api [--bless]` (the public API against `agentcat-sdk/public-api.txt`), `package-list` (the files of the published package), `coverage` (the line coverage floors), `rmcp-versions` (the `rmcp` versions that CI tests) and `examples-check` (the customer examples). |
| [`scripts/`](scripts) | `check.sh`, the local gate, which runs the commands of every CI job. `mutants-base.sh` and `mutants-run.sh` pick the base commit of the mutation check and run it. |
| [`.github/workflows/`](.github/workflows) | `ci.yml`, the gate of every pull request, and `nightly.yml`, the full mutation run. |
| [`docs/`](docs) | The images of the README (`docs/static/`, `docs/cats/`). |

## Prerequisites

Install these tools:

- Rust stable and Rust 1.88, the minimum supported version: `rustup toolchain install stable 1.88`
- Rust nightly, for the docs build and the minimal-versions check: `rustup toolchain install nightly`
- The dated nightly of the public API check: `rustup toolchain install nightly-2026-10-18`
- The cargo tools of the gate:
  `cargo install cargo-nextest cargo-hack cargo-minimal-versions cargo-deny --locked`
- The cargo tools of the public API, coverage and mutation checks:
  `cargo install cargo-public-api@0.52.0 cargo-llvm-cov@0.9.1 cargo-mutants@27.1.0 --locked`
- The coverage component: `rustup component add llvm-tools-preview`
- `jq`, which the tests of the mutation base check use: `apt-get install jq` (or the package manager of your system)

If a tool is missing, `scripts/check.sh` stops and prints the install command.

cargo-public-api reads the rustdoc JSON of nightly-2026-10-18 (`TOOLCHAIN` in
`xtask/src/public_api.rs`). The toolchain is pinned, because a newer nightly can change the rustdoc
JSON format. To bump the toolchain or cargo-public-api, change the toolchain in
`xtask/src/public_api.rs`, in the `api` job of `.github/workflows/ci.yml` and in `scripts/check.sh`
together, and the cargo-public-api version in `ci.yml`, `check.sh` and this file. Then run
`cargo run -q -p xtask -- public-api`. A test of the xtask fails when the toolchain names differ.

## Run the checks

Run `bash scripts/check.sh`. Exit 0 means green. The script runs the commands of every job in
`.github/workflows/ci.yml`, in this order:

1. Formatting, clippy, and clippy on each feature alone (cargo-hack).
2. The examples check (`xtask examples-check --all`).
3. The Rust tests with all features and with no default features, the key-order test, the
   doctests and the rmcp canary.
4. The coverage floors.
5. The mutation check.
6. The tests and the doctests on Rust 1.88.
7. The docs build on nightly, with warnings denied.
8. The public API check, the check of the package files and a publish dry run. The dry run
   contacts crates.io but uploads nothing.
9. cargo-deny.
10. The minimal-versions check, and the serde_json floor: one test on serde_json 1.0.127, the
    lowest version that the crate accepts.
11. The build of the examples, the doctests and the rmcp version test on the rmcp of `Cargo.lock`.
    CI repeats them on every rmcp 3.x minor.

For a shorter loop, run one part, for example `cargo nextest run -p agentcat-sdk --all-features`.
Run the Rust tests with nextest, not `cargo test`: the tests of the global logger need one process
per test. For the doctests, run `cargo test --workspace --doc`.

## How the tests work

- **Unit tests** are in `agentcat-sdk/src/**/tests.rs`, next to the code that they test.
- **Fixtures** are the JSON files in `agentcat-sdk/tests/vectors/`. Each file holds named cases
  (`{"cases": [{"name": ..., "input": ..., "output": ...}]}`), and a test loads them with
  `test_support::vectors::cases("<file stem>")`. They fix the exact output of the pure functions:
  constants and texts, IDs, injected schemas with their key order, token estimates, JSON text,
  UTF-16 cuts, tags, truncation, sanitization and redaction. `agentcat-sdk/tests/vector_integrity.rs`
  checks that each file parses, has unique case names, and is loaded by a test.
- **Property tests** (proptest) run the same functions on random inputs and check their
  invariants. The repository keeps the seeds of past failures in
  `agentcat-sdk/proptest-regressions/`; commit a new seed file when a property test finds a failure.
- **The end-to-end suite** (`agentcat-sdk/tests/e2e.rs` and `agentcat-sdk/tests/e2e/`) runs a real
  `rmcp` server with `agentcat-sdk` and a client, over stdio and streamable HTTP, against a mock
  AgentCat API on `127.0.0.1`. Each scenario compares what the client sees and the events that
  the mock records with a golden file in `agentcat-sdk/tests/e2e/golden/`. Every publish body
  must also match `agentcat-sdk/tests/e2e/publish-event.schema.json`, a copy of the request
  schema of the AgentCat API.
- **The examples check** (`cargo run -q -p xtask -- examples-check --all`) runs each example of
  `agentcat-sdk/examples/` against a local mock API. It fails when a line on stdout is not
  JSON-RPC, when the event of a call does not arrive, or when the example does not exit within
  5 s after the client closes stdin or sends SIGTERM. xtask builds the examples itself. To run
  one example, run `cargo run -q -p xtask -- examples-check stdio` (or `streamable_http`,
  `hooks`, `stateless_http`).
- **Coverage floors**: `cargo run -q -p xtask -- coverage target/coverage.json` fails below 90 %
  of the lines of the crate, or below 95 % of the lines of the pure modules. `scripts/check.sh`
  shows the `cargo llvm-cov` command that writes the JSON file.
- **Mutation testing** uses cargo-mutants (settings in `.cargo/mutants.toml`). A mutant is a small
  change to the SDK source, such as `>` to `>=`. A mutant that the tests do not catch is missed,
  and one missed mutant fails the check. A mutant that times out counts as detected. On a pull
  request, the diff gate mutates only the lines that changed, in 4 shards. The nightly run
  mutates the whole crate in 8 shards and fails below 90 % detected.

### When a change is intended

- **A fixture fails.** There is no switch that rewrites the fixtures. Edit the JSON file by hand.
  The assertion message names the case (`name`) and shows the value that the code produced next
  to the value in the file, so you can copy the new value into the file. Check that the change is
  the one that you intended, not a regression.
- **A golden fails.** Run `UPDATE_GOLDEN=1 cargo nextest run -p agentcat-sdk --test e2e`. It
  rewrites the golden files. Read `git diff agentcat-sdk/tests/e2e/golden/` before you commit.
- **A version bump.** The version of the crate is the `agentcat_version` field of every golden
  file. A release re-captures the goldens with `UPDATE_GOLDEN=1`, and the diff must show only
  that field.
- **The publish schema changes.** The maintainers refresh
  `agentcat-sdk/tests/e2e/publish-event.schema.json` from the AgentCat API definition when the
  API changes its request schema.
- **The public API changes.** Run `cargo run -q -p xtask -- public-api --bless`. A bump of `rmcp`
  changes the `Tracked` `ServerHandler` lines of the snapshot, so it needs a new bless.

### The mutation diff gate

`scripts/mutants-base.sh` prints the base of the diff:

- Locally, the base is the upstream of the current branch, or `origin/main` when the branch has no
  upstream. The diff starts at the merge base of that commit and HEAD, and it includes
  uncommitted changes to tracked or staged files.
- In CI, the base is the newest commit with a successful CI run, on any branch, that HEAD descends
  from. When no such commit exists, the base is the parent commit.

**Merge a pull request with a merge commit or a fast-forward, never a squash.** After a squash,
`main` does not descend from the green commits of the branch. The check then mutates the whole
squashed change again, and a large change can take longer than the time limit of its job.

To run the check by hand, use the commands of `scripts/check.sh`:

```sh
mkdir -p target
git diff "$(git merge-base "$(scripts/mutants-base.sh)" HEAD)" > target/mutants.diff
scripts/mutants-run.sh --in-diff target/mutants.diff
```

Read `mutants.out/missed.txt` for the missed mutants.

## Tests never contact production

No test, example check or end-to-end run may contact AgentCat production (`api.agentcat.com`,
`otel.agentcat.com` or any other production host).

- The examples check and the end-to-end suite send events only to a mock API on `127.0.0.1`.
- A test that needs a network service starts it on `127.0.0.1`, and points the SDK at it with
  `Options::api_base_url`.
- A test never writes the `~/agentcat.log` of the developer: it uses a temporary `HOME`.

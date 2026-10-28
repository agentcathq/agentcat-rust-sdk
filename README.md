# AgentCat Rust SDK

[![CI](https://github.com/agentcathq/agentcat-rust-sdk/actions/workflows/ci.yml/badge.svg)](https://github.com/agentcathq/agentcat-rust-sdk/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

This repository holds `agentcat-sdk`, the AgentCat SDK for MCP servers built on
[rmcp](https://crates.io/crates/rmcp). It also holds the tools that prove that the crate acts like
the AgentCat TypeScript SDK.

To add AgentCat to your server, read the crate's [README](agentcat-sdk/README.md) and its guide on
[docs.rs](https://docs.rs/agentcat-sdk).

## What the repository holds

| Path | What it holds |
|---|---|
| [`agentcat-sdk/`](agentcat-sdk) | The published crate: the SDK, its tests, its examples, the README and the CHANGELOG. |
| [`xtask/`](xtask) | Repository tasks in Rust, run as `cargo run -p xtask -- <task>`: the parity check (`parity`), the check that the SDK forwards every method of `rmcp`'s `ServerHandler` (`rmcp-canary`), the public API check against `agentcat-sdk/public-api.txt` (`public-api`), the check of the files in the published package (`package-list`) and the line coverage floors (`coverage`). |
| [`oracle/`](oracle) | Node tools that build the TypeScript SDK at a pinned commit and run its modules: the test vectors, the property-test oracle and the list of TypeScript tests. |
| [`harness/`](harness) | The differential harness: TypeScript and Rust reference servers, one scripted MCP client, a mock AgentCat API, and the rules that compare two runs. |
| [`parity/`](parity) | The list of every TypeScript test (`ts-tests.json`, generated) and the map from each test to its Rust test, harness scenario or reason (`manifest.toml`). |
| [`scripts/`](scripts) | `check.sh`, the local gate, which runs the commands of every CI job. `mutants-base.sh` and `mutants-run.sh` pick the base commit of the mutation check and run it. `rmcp-versions.mjs` lists the rmcp versions that CI tests. `ts-drift-issue.sh` opens the issue of the nightly TypeScript drift check. |
| [`docs/`](docs) | The decision log, the research, the design and the plans. |

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

## Contributing

[`CONTRIBUTING.md`](CONTRIBUTING.md) tells you how to set up a checkout and run the checks. Read
these before a change:

- [`docs/DECISIONS.md`](docs/DECISIONS.md): the decisions of the owner and of the design.
- [The design](docs/superpowers/specs/2026-10-20-rust-sdk-design.md), with the deviation register
  in section 9.

## License

MIT. See [LICENSE](LICENSE).

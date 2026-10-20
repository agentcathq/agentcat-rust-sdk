# Research for the AgentCat Rust SDK (attempt 2)

Research done on 2026-10-20 before the design. These files are inputs to
[`../superpowers/specs/2026-10-20-rust-sdk-design.md`](../superpowers/specs/2026-10-20-rust-sdk-design.md)
and [`../DECISIONS.md`](../DECISIONS.md). Where a report and the design or the decision log disagree, the design and
the decision log win: the owner answered several of the questions these reports raise.

Pinned inputs:

- TypeScript SDK `agentcathq/agentcat-typescript-sdk` at `9b56e9e` (package version 2.1.1).
- PHP decision log `agentcat-php-sdk/docs/audit/DECISIONS.md` (D01–D68).
- rmcp 3.6.0 (tag `rmcp-v3.6.0`, released 2026-10-20).
- `agentcat-api` 0.0.3 (crates.io, 2026-10-19).

| File | What it holds |
|---|---|
| `01-ts-sdk-behavior.md` | The TS behavior contract: B-001 to B-119, the known TS bugs K-01 to K-20, verbatim agent-facing strings, wire examples and golden vectors. This is the behavior baseline. |
| `01-ts-tests-inventory.md` | The 79 TS test files and what each one covers. |
| `02-decision-matrix.md` | Every PHP decision mapped to Rust (ADOPT / ADAPT / owner decision), with the Rust facts behind each verdict. |
| `03-rmcp.md` | How to instrument rmcp: the `ServerHandler` decorator, request context, HTTP parts, sessions, version churn. |
| `04-rust-sdk-practices.md` | Practices from Sentry, Datadog, PostHog, OpenTelemetry and others, ending in a style guide, a lint table and CI gates. |
| `05-api-client-and-rmcp-usage.md` | The generated `agentcat-api` crate (API, dependencies, TLS, mismatches with TS) and how 19 real projects use rmcp. |
| `06-verification-strategy.md` | The anti-drift verification design: TS-derived vectors, JS-oracle property tests, the differential harness, the parity manifest and the CI gates. |

`spikes/` holds the source of two throwaway experiments. They are references, not product code:

- `spikes/rmcp-poc/`: a generic `ServerHandler` decorator over rmcp 3.6 that injects schema parameters, strips them,
  mirrors `mcp_session` and records events, with tests over an in-process duplex transport.
- `spikes/diff-spike/`: the TS half of the differential harness (driver, mock API, normalizer, TS reference server,
  scenarios), an uninstrumented rmcp reference server, TS-derived vectors and a local ingest check. Its
  `node_modules`, the TS SDK checkout and the outputs are not copied; `run-spike.sh` shows how they were made.

Paths written as `<research scratchpad>` point to files of the research session that were not kept.

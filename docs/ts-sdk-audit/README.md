# TypeScript SDK audit for the Rust port

This audit prepares a native Rust AgentCat SDK for MCP servers written in
Rust. It does not cover running TypeScript MCP servers from Rust. The
reference is the TypeScript SDK at `agentcat-typescript-sdk` `main`
(`9b56e9e`, package version 2.1.1-beta.1), audited on 2026-10-16.

## Reports

| File | Scope |
| --- | --- |
| [01-architecture-and-interception.md](01-architecture-and-interception.md) | Public API, options, the `tools/list` and `tools/call` engine, `context` intent capture, `get_more_tools`, handles, sessions, `identify` and other hooks, exact strings and wire keys, test plan, TS quirks |
| [02-event-pipeline-and-exporters.md](02-event-pipeline-and-exporters.md) | Event model, the one API call, server-side ingest rules, event queue, redaction, sanitization, truncation, token estimates, error capture, OTLP/Datadog/Sentry/PostHog exporters, test plan, defects |
| [03-rust-mcp-ecosystem-and-design.md](03-rust-mcp-ecosystem-and-design.md) | `rmcp` internals, other Rust MCP crates, interception options, lessons from the Go, Python and Java ports, proposed crate layout |

Every claim in the reports cites a file and line.

## Main conclusions

1. **Port the code, not the cross-SDK brief.** `2026-07-28-cross-sdk-changelog.md`
   is out of date. TS `main` uses `session_id`, `mcp_session`, and a leading
   mint-back text block (01 §0, 03 §4.5).
2. **The scope is small.** The TS SDK intercepts only `tools/list` and
   `tools/call`, and it publishes only `mcp:tools/call` and `agentcat:custom`
   events (01 §1, 02 §1).
3. **Target `rmcp` 3.x only.** It is the only Rust MCP crate with real
   adoption, and it supports the stateless 2026-07-28 protocol (03 §1, §2).
4. **Intercept with a wrapper type.** `Tracked<H: ServerHandler>` overrides six
   trait methods and forwards the rest. It works for stdio and streamable
   HTTP. A tower layer cannot see stdio traffic (03 §3).
5. **Publish from a dedicated thread.** The publisher runs its own Tokio
   runtime, and customers call `shutdown().await` to flush (02 §3, 03 §0).
6. **Crates:** `agentcat-sdk` (lib name `agentcat`) depends on the generated
   `agentcat-api`. The name `agentcat` belongs to a third party (03 §5).

## Top porting risks

- **Trait drift.** Every `ServerHandler` method has a default. If rmcp adds a
  method and the wrapper does not forward it, tracked servers silently use
  the default. CI must compare rmcp's method list with the wrapper's.
- **JSON key order is part of the contract.** `serde_json` sorts keys unless
  the `preserve_order` feature is on, and that feature then applies to the
  whole customer build (01 §14).
- **Fail open.** AgentCat errors and panics must never reach the MCP client.
  The request path needs no `?`, no `unwrap`, and `catch_unwind` around hooks.
- **Silent server-side loss.** The API answers 202 and drops bad events later
  (wrong `project_id` format, missing `timestamp`, over-long fields). The Rust
  SDK must check events before it sends them (02 §2).
- **TS quirks.** 01 §15 and 02 §8 list behavior that looks like bugs. Decide
  for each item whether to copy it or fix it.

## API client status

The generated `agentcat-api` crate is built in `agentcat-server` `sdk-generator/`
(`generate-rust-sdk.sh`, merged in PR #592) and published to crates.io with trusted
publishing. Version 0.0.3 is the first with `EventType::AgentcatColonCustom`; the SDK depends
on `agentcat-api = "0.0.3"`. The client uses OpenAPI Generator 7.26.0: reqwest 0.13 (as rmcp
uses), `native-tls` and `rustls` features, and `chrono` timestamps.

## Follow-up research and the port

- `../research/04-rust-mcp-usage.md`: which Rust MCP crates have real usage, and how 15 real
  rmcp servers are built and served.
- `../research/05-rust-sdk-best-practices.md`: how Sentry, OpenTelemetry, Datadog, PostHog and
  others build lean Rust SDKs, and the checklist the port follows.
- `../research/06a-ts-test-index.md`, `06b`, `06c`: every TS test case (1,024), classified as
  port, adapt or N/A for Rust.
- `../DECISIONS.md` and §10 of `../superpowers/specs/2026-10-19-rust-sdk-design.md`: the port's deliberate differences from the TS SDK.

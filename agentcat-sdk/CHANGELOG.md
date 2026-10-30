# Changelog

This file lists the notable changes of each release of `agentcat-sdk`.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the crate follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `track` and `AgentCat`: wrap an `rmcp` `ServerHandler` in a `Tracked` server that records each
  tool call. One `AgentCat` handle serves every session and every request of a streamable HTTP
  server. `AgentCat::from_env` reads the project ID from `AGENTCAT_PROJECT_ID`. An empty project
  ID gives a telemetry-only handle, which sends nothing.
- Changes to the tool list. Tools get the `context`, `session_id` and `agent_id` parameters,
  and output schemas get the `mcp_session` property. The list gets the `get_more_tools` tool.
  The first call of a task gets the session text block.
- `Options`, with the setters `enable_tool_call_context`, `custom_context_description`,
  `enable_tracing`, `enable_agent_tracking`, `enable_report_missing`, `api_base_url`,
  `log_destination`, and the hooks `resolve_session_id`, `identify`, `event_tags`,
  `event_properties`, `redact_event` and `redact_sensitive_information`. The hooks get a
  `ToolCall`, an `Event` or a string, and return a `HookError` when they fail. The SDK waits at
  most 30 s for each call hook before the event goes out.
- `UserIdentity`, which the `identify` hook returns.
- `Event`, which the `redact_event` hook gets and returns. `Event::default()` builds one for the
  unit tests of a hook.
- `CallContext`, which each tool call gets as an `Extension`: the session ID, the agent ID, the
  handle, and custom events in the identity of the call.
- `CustomEvent` and three ways to publish it: `AgentCat::publish_custom_event`,
  `CallContext::publish_custom_event` and `Tracked::publish_custom_event`. `Error` is the error
  of a publish.
- `capture_error`, which records the type, the text and the `source()` chain of an error that a
  tool returns.
- An `error` in the event of each failed call: an error result, a JSON-RPC error (with the name
  of its code as `error.type`), a panic of the tool, or a call that is cancelled before it
  completes (with the type `cancelled`). The platform of each `error` is `rust`.
- Delivery from one thread for each handle, with its own queue and API URL. The queue holds at
  most 10 000 events and drops the oldest event when it is full. At most 5 events are in progress
  at a time. A failed send gets 3 more attempts, after 1, 2 and 4 s. Each attempt has a timeout of
  10 s.
- `AgentCat::flush` and `AgentCat::shutdown`, and the blocking twins `flush_blocking` and
  `shutdown_blocking`. The drop of the last clone of a handle delivers the queued events for up
  to 5 s. The SDK installs no signal or exit handler.
- Log lines in `~/agentcat.log`, or as `tracing` events with `LogDestination::Tracing`.
- The environment variables `AGENTCAT_PROJECT_ID`, `AGENTCAT_API_URL`, `MCPCAT_API_URL` and
  `AGENTCAT_ENABLED`, with a public constant for each name. `AGENTCAT_ENABLED=false` turns the
  SDK off.
- The features `rustls` (default) and `native-tls` for the TLS of the connection to the API, and
  `test-util` for `testing::Fake`.
- `testing::Fake`, a test double that records the events of its handles and sends nothing.
- Support for `rmcp` 3.0.0 and later 3.x, on Rust 1.88 and later.

[Unreleased]: https://github.com/agentcathq/agentcat-rust-sdk/commits/main

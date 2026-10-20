# Changelog

## 0.1.0 (unreleased)

First release: a port of the AgentCat TypeScript SDK 2.1.1 for MCP servers built with rmcp
3.x.

- `AgentCat::track` wraps an rmcp `ServerHandler`: injected `session_id`, `agent_id` and
  `context` parameters, `get_more_tools`, mint-back text and `structuredContent.mcp_session`,
  and one `mcp:tools/call` event per call.
- Hooks: `identify`, `resolve_session_id`, `event_tags`, `event_properties`,
  `redact_sensitive_information`, `redact_event` (sync and async).
- `publish_custom_event` for `agentcat:custom` events.
- Exporters: OTLP, Datadog, Sentry, PostHog.
- A background publisher thread with retries, a bounded queue, `flush` and `shutdown`.
- Behavior now follows the TS SDK in tag validation, tool-schema injection (composed schemas, empty schemas, failure paths, log texts), the protocol-version tag, KSUID decoding and argument key order.
- Differences from the TypeScript SDK: see `docs/DECISIONS.md` and §10 of the design spec (`docs/superpowers/specs/2026-10-19-rust-sdk-design.md`).

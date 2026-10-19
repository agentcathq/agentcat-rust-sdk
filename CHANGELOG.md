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
- Differences from the TypeScript SDK: see `docs/PORTING.md`.

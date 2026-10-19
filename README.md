# AgentCat for Rust MCP servers

[![crates.io](https://img.shields.io/crates/v/agentcat-sdk.svg)](https://crates.io/crates/agentcat-sdk)
[![docs.rs](https://docs.rs/agentcat-sdk/badge.svg)](https://docs.rs/agentcat-sdk)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](https://opensource.org/licenses/MIT)

AgentCat is an analytics platform for MCP server owners 🐱. It captures what agents try to do
with your tools and why, so you can see what your users actually need.

This crate instruments MCP servers built with [`rmcp`](https://crates.io/crates/rmcp) 3.x, the
official Rust MCP SDK. It is a port of the
[AgentCat TypeScript SDK](https://github.com/agentcathq/agentcat-typescript-sdk) and sends the
same events, injects the same parameters, and uses the same agent-facing text. It can also
forward telemetry to OpenTelemetry, Datadog, Sentry and PostHog.

```toml
[dependencies]
agentcat-sdk = "0.1"
```

The library is named `agentcat`: write `use agentcat::AgentCat;`.

## How it works

`AgentCat::track` wraps your rmcp server handler. The wrapper changes two things:

- **`tools/list`.** Every tool gets these input parameters:
  - **`session_id`**, a task handle ([explicit handles, SEP-2567](https://github.com/modelcontextprotocol/modelcontextprotocol/pull/2567)).
    The server issues a `ses_` ID on the agent's first call, and the agent sends it back on
    every later call. Related calls then group into one task, also over stateless transports.
    AgentCat rejects values it did not issue and tells the agent to re-send its ID.
  - **`agent_id`** (off by default; enable it with `enable_agent_tracking(true)`), an ID that
    each agent chooses for itself, so parallel agents on one task stay apart.
  - **`context`**, where the agent explains in one sentence why it makes the call. This is the
    intent data.

  The wrapper also adds a **`get_more_tools`** tool, where agents report the tools they looked
  for and did not find.
- **`tools/call`.** The wrapper removes the injected arguments before your handler runs, so your
  code never sees them, even with `#[serde(deny_unknown_fields)]`. It returns issued session
  IDs in a text block at the start of the result and, for tools with an output schema, in
  `structuredContent.mcp_session`. It then publishes one event with the request, the result,
  the timing, token estimates, and any error.

Events go out from a background thread. Your tool calls never wait for AgentCat, and an AgentCat
failure never fails a tool call.

## Getting started

Create a project at [agentcat.com](https://agentcat.com) to get a project ID.

### stdio

```rust
use std::time::Duration;
use agentcat::AgentCat;
use rmcp::{ServiceExt, transport::stdio};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let agentcat = AgentCat::new("proj_0000000");

    let service = agentcat.track(MyServer::new()).serve(stdio()).await?;
    service.waiting().await?;

    // Send queued events before the process exits.
    agentcat::shutdown(Duration::from_secs(5)).await;
    Ok(())
}
```

Call `agentcat::shutdown` (or `shutdown_blocking`) before your process exits. A stdio server
stops when `main` returns, and events still in the queue are lost.

### Streamable HTTP

The `StreamableHttpService` factory creates a handler per session, or per request in stateless
mode. Track each one. `AgentCat` is cheap to clone, and all instances share one queue:

```rust
use rmcp::transport::streamable_http_server::{
    StreamableHttpService, StreamableHttpServerConfig, session::local::LocalSessionManager,
};

let agentcat = agentcat::AgentCat::new("proj_0000000");
let mcp = StreamableHttpService::new(
    move || Ok(agentcat.track(MyServer::new())),
    LocalSessionManager::default().into(),
    StreamableHttpServerConfig::default(),
);
let app = axum::Router::new().nest_service("/mcp", mcp);
```

See [`examples/`](examples) for complete servers.

## Options

```rust
let agentcat = AgentCat::builder("proj_0000000")
    .enable_agent_tracking(true)
    .custom_context_description("Explain what you are trying to do for the user.")
    .build();
```

| Builder method | Default | What it does |
| --- | --- | --- |
| `enable_tracing(bool)` | `true` | Track tool calls: handles, mint-back, events. |
| `enable_tool_call_context(bool)` | `true` | Add the required `context` (intent) parameter. |
| `custom_context_description(text)` | built-in text | Replace the `context` parameter's description. |
| `enable_report_missing(bool)` | `true` | Add the `get_more_tools` tool. |
| `enable_agent_tracking(bool)` | `false` | Add the required `agent_id` parameter. |
| `identify(hook)` | none | Identify the actor of each call. |
| `resolve_session_id(hook)` | none | Hook mode: you supply the session; no `session_id` parameter. |
| `event_tags(hook)` / `event_properties(hook)` | none | Attach tags and JSON properties to each event. |
| `redact_sensitive_information(f)` / `redact_event(f)` | none | Redact text, or rewrite and drop whole events (async variants too). |
| `exporter(name, config)` | none | Forward events to OTLP, Datadog, Sentry or PostHog. |
| `api_base_url(url)` | `AGENTCAT_API_URL`, then `https://api.agentcat.com` | Where events go. |
| `server_info(name, version)` | from `get_info()` | Override the server identity on events. |
| `disable_diagnostics(bool)` | `false` | Turn off anonymous SDK diagnostics (also `DISABLE_DIAGNOSTICS=1`). |
| `log_file(bool)` | `true` | Write the SDK log to `~/agentcat.log` (or `$AGENTCAT_LOG_FILE`). |
| `capture_panics(bool)` | `true` | Record panics in tool handlers with their backtrace. |

### Identifying users

Identify every actor you can. When you cannot resolve a real user, return a stable anonymous ID,
such as a hash of the API key, so one user's events still group together.

Hooks get a `ToolCallRequest`: the raw arguments and the handler's `RequestContext`. Over HTTP,
`http_request()` returns the request head, including whatever your auth middleware put into its
extensions. Hooks run while your tool runs and never delay it.

```rust
let agentcat = AgentCat::builder("proj_0000000")
    .identify(|call| {
        let token = call.header("authorization").map(str::to_owned);
        async move {
            let user = my_api::user_for(token?).await.ok()?;
            Some(agentcat::UserIdentity::new(user.id).name(user.name))
        }
    })
    .build();
```

### Redacting sensitive data

```rust
let agentcat = AgentCat::builder("proj_0000000")
    .redact_sensitive_information(|text| SECRET.replace_all(text, "[REDACTED]").into_owned())
    .redact_event(|event| {
        // Drop events from tools that handle secrets.
        (event.resource_name.as_deref() != Some("get_credentials")).then_some(event)
    })
    .build();
```

AgentCat publishes inbound HTTP headers with each event, as the TypeScript SDK does, so
remove anything sensitive (for example `authorization`) with `redact_event`.

### Custom events

```rust
agentcat.publish_custom_event(
    agentcat::CustomEvent::new()
        .session_id(session_id)
        .resource_name("checkout")
        .parameters(serde_json::json!({ "plan": "pro" })),
);
```

### Telemetry exporters

```rust
use agentcat::exporters::{OtlpExporterConfig, SentryExporterConfig};

let agentcat = AgentCat::builder("proj_0000000") // or AgentCat::telemetry_only()
    .exporter("otlp", OtlpExporterConfig::new("http://localhost:4318"))
    .exporter("sentry", SentryExporterConfig::new(std::env::var("SENTRY_DSN")?))
    .build();
```

Exporters write their vendor's JSON over the SDK's HTTP client. They install no global tracer,
hub or subscriber.

## Cargo features

| Feature | Default | Purpose |
| --- | --- | --- |
| `rmcp` | yes | The `Tracked<H>` wrapper for rmcp 3.x. |
| `rustls` / `native-tls` | `rustls` | TLS for AgentCat and the exporters. One is required. |
| `preserve-order` | yes | Turns on `serde_json/preserve_order` so injected parameters follow yours and `mcp_session` is the first key of structured results (agents that truncate long results still see it). It applies to your whole build. |
| `exporters` (`otlp`, `datadog`, `sentry`, `posthog`) | yes | Telemetry exporters. They add no dependencies. |

## Compatibility

- rmcp `>= 3.0, < 4` (tested on 3.0.0 and the newest 3.x), with stdio, streamable HTTP
  (sessions and stateless), the 2026-07-28 protocol, and any other rmcp transport.
- Rust 1.88 or newer (the same MSRV as rmcp 3).
- Behavior differences from the TypeScript SDK are listed in
  [`docs/PORTING.md`](docs/PORTING.md).

## License

MIT © AgentCat, Inc.

<div align="center">
  <img alt="AgentCat — see exactly how agents experience your product" src="https://raw.githubusercontent.com/agentcathq/agentcat-rust-sdk/main/docs/static/og-image.png" width="80%">
</div>
<h3 align="center">
    <a href="#getting-started">Getting Started</a>
    <span> · </span>
    <a href="#why-use-agentcat-">Features</a>
    <span> · </span>
    <a href="https://docs.agentcat.com">Docs</a>
    <span> · </span>
    <a href="https://agentcat.com">Website</a>
    <span> · </span>
    <a href="#free-for-open-source">Open Source</a>
    <span> · </span>
    <a href="https://book.agentcat.com/meet">Schedule a Demo</a>
</h3>
<p align="center">
  <a href="https://crates.io/crates/agentcat-sdk"><img src="https://img.shields.io/crates/v/agentcat-sdk.svg" alt="crates.io version"></a>
  <a href="https://crates.io/crates/agentcat-sdk"><img src="https://img.shields.io/crates/d/agentcat-sdk.svg" alt="crates.io downloads"></a>
  <a href="https://docs.rs/agentcat-sdk"><img src="https://docs.rs/agentcat-sdk/badge.svg" alt="docs.rs"></a>
  <a href="https://opensource.org/licenses/MIT"><img src="https://img.shields.io/badge/License-MIT-blue.svg" alt="License: MIT"></a>
  <a href="https://docs.rs/agentcat-sdk/latest/agentcat_sdk/#minimum-supported-rust-version"><img src="https://img.shields.io/badge/MSRV-1.88-blue.svg" alt="MSRV 1.88"></a>
  <a href="https://github.com/agentcathq/agentcat-rust-sdk/issues"><img src="https://img.shields.io/github/issues/agentcathq/agentcat-rust-sdk.svg" alt="GitHub issues"></a>
  <a href="https://github.com/agentcathq/agentcat-rust-sdk/actions/workflows/ci.yml"><img src="https://github.com/agentcathq/agentcat-rust-sdk/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
</p>

> [!NOTE]
> AgentCat for Rust is compatible with the [MCP Protocol "Stateless" 2026-07-28 Update](https://blog.modelcontextprotocol.io/posts/2026-07-28/) as the official Rust MCP SDK puts it into effect: [`rmcp`](https://github.com/modelcontextprotocol/rust-sdk) 3.x, from 3.0.0, over stdio and streamable HTTP. The stateless transition has a massive impact on analytics, as sessions were a built-in concept tying related tool calls together. AgentCat has now migrated its session tracking under guidance of the MCP core team's recommendations of using [explicit handles (SEP-2567)](https://github.com/modelcontextprotocol/modelcontextprotocol/pull/2567).
>
> As a result AgentCat now injects a `session_id` on every MCP tool call to associate them under the same task umbrella. Our evals show much higher tool correlation accuracy at the cost of < 1% additional context pollution.

AgentCat is an analytics platform for MCP server owners 🐱. It captures user intentions and behavior patterns to help you understand what AI users actually need from your tools — eliminating guesswork and accelerating product development all with one-line of code.

A later release of this SDK will add a free and simple way to forward telemetry like logs, traces, and errors to any Open Telemetry collector or popular tools like Datadog, Sentry, and PostHog. This release sends analytics to AgentCat only.

```sh
cargo add agentcat-sdk
```

To learn more about us, check us out [here](https://agentcat.com). For detailed guides visit our [documentation](https://docs.agentcat.com).

## Why use AgentCat? 🤔

AgentCat helps builders of MCP servers, Claude Connectors, and ChatGPT Plugins learn how to improve them by capturing any agents goals and detecting when they get stuck.

Use AgentCat for:

- **Agent session replay** 🎬. Follow alongside your users and their agents to understand why they're using your MCP servers, what functionality you're missing, and what clients they're coming from.
- **Trace debugging** 🔍. See where your users are getting stuck, track and find when LLMs get confused by your API, and debug sessions across all deployments of your MCP server.
- **Existing platform support** 📊. Coming in a later release of the Rust SDK: logging and tracing out of the box for your existing observability platforms (OpenTelemetry, Datadog, Sentry) — eliminating the tedious work of implementing telemetry yourself.

<img alt="AgentCat architecture — the AgentCat SDK inside your MCP server sends session replay to the AgentCat dashboard and, in a later release of the Rust SDK, analytics to your observability vendors" src="https://raw.githubusercontent.com/agentcathq/agentcat-rust-sdk/main/docs/static/architecture.png" />

## How it works

AgentCat works as a lightweight middleware inside your MCP server. When you wrap your `rmcp` server handler with `track()`, it seamlessly modifies your tool schemas as your server lists them, following the MCP core team's [explicit handles (SEP-2567)](https://github.com/modelcontextprotocol/modelcontextprotocol/pull/2567) guidelines. Concretely, AgentCat adds the following to your server:

- **`session_id`** — a parameter injected into each tool's input schema. Agents echo it back on every call, letting AgentCat group related tool calls into one task even over stateless transports. Values are validated: anything that is not a well-formed AgentCat session ID is rejected rather than adopted, and the agent is told to re-send the ID it was given.
- **`agent_id`** _(off by default)_ — enabled with `Options::default().enable_agent_tracking(true)`. Each agent self-generates its own ID, keeping parallel agents working the same task individually attributable.
- **`context`** — a parameter asking the agent to explain, in 15 to 25 words, why it is making this call. This is where intent data comes from.
- **`get_more_tools`** — an additional tool, prompt-engineered so that agents readily report the features and tools they looked for but couldn't find — surfacing your missing functionality directly from real usage.

Injected parameters are stripped from arguments before your tool handler runs, so your code never sees them, and a tool that declares its own parameter with one of these names keeps it. For tools that declare an output schema, issued IDs are also mirrored into `structuredContent` (as `mcp_session`, its first key), so clients that only read structured results still receive them. To pick the session of each call yourself instead, set a `resolve_session_id` hook: the tools then get no `session_id` parameter.

## Getting Started

To get started with AgentCat, first create an account and obtain your project ID by signing up at [agentcat.com](https://agentcat.com). For detailed setup instructions, read the [crate guide](https://docs.rs/agentcat-sdk) on docs.rs.

Once you have your project ID, set up AgentCat for your `rmcp` server below; the [examples](https://github.com/agentcathq/agentcat-rust-sdk/tree/main/agentcat-sdk/examples) are complete servers for each setup. AgentCat stays on while your tests run: `AGENTCAT_ENABLED=false` turns it off, and [`testing::Fake`](https://docs.rs/agentcat-sdk/latest/agentcat_sdk/testing/struct.Fake.html) (the `test-util` feature) records the events of a test for your assertions.

### stdio

Add AgentCat, `rmcp` 3 with its stdio transport, and tokio. The `#[tool]` macros of `rmcp` also need `schemars` and `serde`:

```sh
cargo add agentcat-sdk
cargo add rmcp@3 --features transport-io
cargo add tokio --features macros,rt-multi-thread
cargo add schemars
cargo add serde --features derive
```

Then wrap your server with `agentcat_sdk::track`, and serve the wrapper where you served your server:

```rust,no_run
use agentcat_sdk::Options;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{InitializeResult, ServerCapabilities};
use rmcp::{ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Deserialize, JsonSchema)]
struct AddArgs {
    a: f64,
    b: f64,
}

#[derive(Clone)]
struct Calculator;

#[tool_router]
impl Calculator {
    #[tool(description = "Adds two numbers")]
    async fn add(&self, Parameters(args): Parameters<AddArgs>) -> String {
        (args.a + args.b).to_string()
    }
}

#[tool_handler]
impl ServerHandler for Calculator {
    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Track the server with AgentCat
    let server = agentcat_sdk::track(Calculator, "proj_0000000", Options::default());
    let running = server.serve(rmcp::transport::stdio()).await?;
    running.waiting().await?;
    Ok(())
}
```

To read the project ID from `AGENTCAT_PROJECT_ID` instead, wrap the server with `AgentCat::from_env(Options::default()).track(Calculator)`. A stdio server talks JSON-RPC on stdout: AgentCat writes nothing to stdout or stderr, and its own log lines go to `~/agentcat.log`. Send your own logs to stderr. The server ends when the client closes stdin; the drop of the last AgentCat handle then delivers the queued events, for up to 5 s, so a stdio server needs no `shutdown` call.

### Streamable HTTP

A streamable HTTP server builds a new server handler for each MCP session. Build one `AgentCat` handle for the process, and wrap each new handler with `AgentCat::track`. Tracking each instance is cheap — the clones of one handle share its event queue and its delivery thread. This server needs the streamable HTTP server of `rmcp`, more tokio features, and `axum`:

```sh
cargo add agentcat-sdk
cargo add rmcp@3 --features transport-streamable-http-server
cargo add tokio --features macros,rt-multi-thread,net,signal
cargo add axum@0.8
cargo add schemars
cargo add serde --features derive
```

```rust,no_run
use std::sync::Arc;
use std::time::Duration;

use agentcat_sdk::{AgentCat, Options};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{InitializeResult, ServerCapabilities};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Deserialize, JsonSchema)]
struct AddArgs {
    a: f64,
    b: f64,
}

#[derive(Clone)]
struct Calculator;

#[tool_router]
impl Calculator {
    #[tool(description = "Adds two numbers")]
    async fn add(&self, Parameters(args): Parameters<AddArgs>) -> String {
        (args.a + args.b).to_string()
    }
}

#[tool_handler]
impl ServerHandler for Calculator {
    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // One AgentCat handle for the whole process
    let agentcat = AgentCat::new("proj_0000000", Options::default());

    // rmcp calls this closure for each new MCP session: track each new server
    let handle = agentcat.clone();
    let service = StreamableHttpService::new(
        move || Ok(handle.track(Calculator)),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    );
    let router = axum::Router::new().route_service("/mcp", service);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:8000").await?;
    tokio::select! {
        served = axum::serve(listener, router) => served?,
        _ = tokio::signal::ctrl_c() => {}
    }

    // Deliver the queued events before the process ends
    agentcat.shutdown(Duration::from_secs(5)).await;
    Ok(())
}
```

An HTTP server does not end when a client goes away, and AgentCat installs no signal handler, so call `AgentCat::shutdown` after your server stops. This server stops only on Ctrl-C. Containers and process managers stop a server with SIGTERM: the [`streamable_http.rs`](https://github.com/agentcathq/agentcat-rust-sdk/blob/main/agentcat-sdk/examples/streamable_http.rs) example stops on SIGTERM (Unix) or Ctrl-C, and its `stop_signal` function can take the place of `tokio::signal::ctrl_c()`.

### Stateless HTTP

In stateless mode, `rmcp` builds a new server handler for each request. The setup is the same as above; only the configuration of the service changes:

```rust,no_run
use rmcp::transport::streamable_http_server::StreamableHttpServerConfig;

// No MCP sessions, and JSON responses instead of server-sent events
let config = StreamableHttpServerConfig::default()
    .with_legacy_session_mode(false)
    .with_json_response(true);
```

AgentCat never uses the MCP session of the transport (the `mcp-session-id` header) to group calls. The `session_id` parameter groups them on every transport, so a stateless server loses no sessions. The [`stateless_http.rs`](https://github.com/agentcathq/agentcat-rust-sdk/blob/main/agentcat-sdk/examples/stateless_http.rs) example is a complete stateless server.

### Identifying users

We strongly encourage identifying every actor. If you can't resolve a real user, return a stable anonymized ID instead — for example, a hash of the auth token or API key — so that all events from the same end user still roll up to one actor in your dashboard rather than scattering into anonymous one-off sessions.

`identify` (like every AgentCat hook) never adds latency to a tool call: AgentCat runs it on the tokio runtime of the call, and the call never waits for it. When the event is ready to go out, AgentCat waits at most 30 s for the hook, so a lookup that takes longer than that — or a hook that fails outright — costs analytics data for that event, never your tool's response time.

The callback receives a `ToolCall`: the tool name, the arguments and `_meta` of the request, the HTTP headers, the client's name and version, the protocol version, and the `rmcp` request context. Most identity signals live in its `headers()`, with lower-case names (`None` on stdio). This example reads the headers with `serde_json` and hashes the token with the [`sha2`](https://crates.io/crates/sha2) crate (`cargo add serde_json sha2`):

```rust,no_run
use agentcat_sdk::{AgentCat, HookError, Options, ToolCall, UserIdentity};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

let options = Options::default().identify(|call: ToolCall| async move {
    let header = |name: &str| {
        call.headers()
            .and_then(|headers| headers.get(name))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let Some(token) = header("authorization") else {
        return Ok::<_, HookError>(None);
    };

    // A stable anonymized ID, or your own user ID
    let user_id: String = Sha256::digest(token.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let mut user_data = Map::new();
    if let Some(org_id) = header("x-org-id") {
        user_data.insert("orgId".to_owned(), Value::String(org_id));
    }
    Ok(Some(UserIdentity::new(user_id).user_data(user_data)))
});

// Track your servers with this handle, as in Getting Started
let agentcat = AgentCat::new("proj_0000000", options);
```

`track`, `AgentCat::new` and `AgentCat::from_env` all take the `Options`. The `event_tags` and `event_properties` hooks get the same `ToolCall`, and the [`hooks.rs`](https://github.com/agentcathq/agentcat-rust-sdk/blob/main/agentcat-sdk/examples/hooks.rs) example uses every hook except `resolve_session_id`.

### Redacting sensitive data

AgentCat redacts all data sent to its servers and encrypts at rest, but for additional security, it offers a hook to do your own redaction on all text data returned back to our servers.

```rust,no_run
use agentcat_sdk::{AgentCat, HookError, Options};

let options = Options::default().redact_sensitive_information(|text: String| async move {
    // Replace each sk_live_ key, wherever it is in the text
    let mut redacted = String::with_capacity(text.len());
    let mut rest = text.as_str();
    while let Some(start) = rest.find("sk_live_") {
        redacted.push_str(&rest[..start]);
        redacted.push_str("[REDACTED]");
        let key = &rest[start..];
        let end = key
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(key.len());
        rest = &key[end..];
    }
    redacted.push_str(rest);
    Ok::<_, HookError>(redacted)
});

let agentcat = AgentCat::new("proj_0000000", options);
```

For redaction decisions that need more context than a single string — such as which tool was called or what type of event is being published — use the event-level `redact_event` hook. It receives the full event and returns a modified event, or `None` to drop the event entirely. It runs before `redact_sensitive_information`, and the two can be combined. When a redaction hook fails, AgentCat drops the event rather than send it unredacted.

```rust,no_run
use agentcat_sdk::{AgentCat, Event, HookError, Options};

let options = Options::default().redact_event(|mut event: Event| async move {
    // Drop events from tools that handle secrets entirely
    if event.resource_name.as_deref() == Some("get_credentials") {
        return Ok::<_, HookError>(None);
    }
    // Strip response payloads from a specific tool
    if event.resource_name.as_deref() == Some("export_report") {
        event.response = None;
    }
    Ok(Some(event))
});

let agentcat = AgentCat::new("proj_0000000", options);
```

On streamable HTTP, a tool-call event also carries the HTTP request of the call in `parameters.extra.requestInfo`: each header under its lower-case name, `authorization` and `cookie` included, and the URL with its query string, which can carry an API key. This `redact_event` hook removes the `authorization` and `cookie` headers and the query string; remove any other header that carries a secret, such as `x-api-key`, the same way. `Options` holds one `redact_event` hook, so merge this code with your other `redact_event` logic:

```rust,no_run
use agentcat_sdk::{AgentCat, Event, HookError, Options};
use serde_json::Value;

let options = Options::default().redact_event(|mut event: Event| async move {
    let info = event
        .parameters
        .as_mut()
        .and_then(|parameters| parameters.pointer_mut("/extra/requestInfo"))
        .and_then(Value::as_object_mut);
    if let Some(info) = info {
        if let Some(Value::Object(headers)) = info.get_mut("headers") {
            headers.remove("authorization");
            headers.remove("cookie");
        }
        if let Some(Value::String(url)) = info.get_mut("url") {
            if let Some(query) = url.find('?') {
                url.truncate(query);
            }
        }
    }
    Ok::<_, HookError>(Some(event))
});

let agentcat = AgentCat::new("proj_0000000", options);
```

### Vendor Support

Exporters for OpenTelemetry, Datadog, Sentry and PostHog come in a later release of the Rust SDK; this release forwards no telemetry to them.

### Internal diagnostics

SDK diagnostics come in a later release of the Rust SDK; this release sends AgentCat no diagnostics, and the SDK writes its own log lines only to `~/agentcat.log` (or to `tracing`, with `LogDestination::Tracing`).

## Rust runtimes

- **tokio**: a tracked server needs a tokio runtime with the time driver. `#[tokio::main]`, `#[tokio::test]` and `tokio::runtime::Runtime::new` turn it on; a runtime from `tokio::runtime::Builder` needs `enable_time` or `enable_all`. AgentCat runs the hooks as `Send` tasks on the runtime of the call, so it does not support the `local` feature of `rmcp`.
- **Delivery**: each `AgentCat` handle sends its events from its own thread, `agentcat-delivery`, so a tool call never waits for the network. The queue holds at most 10 000 events, and a failed send gets 3 more attempts, after 1, 2 and 4 s.
- **stdio**: AgentCat writes nothing to stdout or stderr, so stdout stays JSON-RPC. The `fmt` subscriber of `tracing-subscriber` writes to stdout unless you give it another writer, such as `.with_writer(std::io::stderr)`. When the client closes stdin, the drop of the last handle delivers the queued events, for up to 5 s.
- **HTTP servers and containers**: AgentCat installs no signal handler. Call `AgentCat::shutdown` after your server stops, and stop on SIGTERM as well as Ctrl-C, as the `streamable_http.rs` example does.
- **Current-thread runtimes**: a drop of the last handle on the runtime's only thread blocks that thread, so no hook can run during the wait; call `shutdown` before the runtime ends.
- **Process exit**: the drain of the last drop does not run after `std::process::exit`, or after a panic with `panic = "abort"`. Call `shutdown`, or `shutdown_blocking`, which needs no tokio runtime, first.
- **TLS**: the default `rustls` feature connects to the AgentCat API with rustls. For the TLS library of the platform, turn off the default features and turn on `native-tls`. Without a TLS feature, AgentCat can send events only to an `http:` URL.
- **serde_json**: AgentCat turns on the `preserve_order` feature of `serde_json`, so that `mcp_session` stays the first key of a result. Cargo turns a feature on for the whole build, so every `serde_json::Map` in your program keeps the order of insertion and does not sort its keys.

AgentCat supports Rust 1.88 and later, the minimum supported Rust version (MSRV) of `rmcp` 3, and `rmcp` 3.x from 3.0.0. The MSRV is never higher than the MSRV of the `rmcp` versions that the crate supports, and it is always a Rust release that is at least 6 months old; an MSRV increase is a minor release with its own CHANGELOG section. On every pull request and every push to `main`, CI reads crates.io and tests the newest patch of each `rmcp` 3.x minor release, from 3.0.0 up, and runs the tests and the doctests on Rust 1.88. The [crate guide](https://docs.rs/agentcat-sdk) covers the options, the environment variables, delivery and shutdown, logging and testing, and its sections on the [differences from the TypeScript SDK](https://docs.rs/agentcat-sdk/latest/agentcat_sdk/#differences-from-the-typescript-sdk) and the [known limits](https://docs.rs/agentcat-sdk/latest/agentcat_sdk/#known-limits) list the rest.

## Free for open source

AgentCat is free for qualified open source projects. We believe in supporting the ecosystem that makes MCP possible. If you maintain an open source MCP server, you can access our full analytics platform at no cost.

**How to apply**: Email [hi@agentcat.com](mailto:hi@agentcat.com) with your repository link

_Already using AgentCat? We'll upgrade your account immediately._

## Community Cats 🐱

Meet the cats behind AgentCat! Add your cat to our community by submitting a PR with your cat's photo in the `docs/cats/` directory of the [TypeScript SDK](https://github.com/agentcathq/agentcat-typescript-sdk/tree/main/docs/cats), whose cats this README shows too.

<div align="left">
  <img src="https://raw.githubusercontent.com/agentcathq/agentcat-rust-sdk/main/docs/cats/bibi.png" alt="bibi" width="80" height="80">
  <img src="https://raw.githubusercontent.com/agentcathq/agentcat-rust-sdk/main/docs/cats/zelda.jpg" alt="zelda" width="80" height="80">
  <img src="https://raw.githubusercontent.com/agentcathq/agentcat-rust-sdk/main/docs/cats/void.jpg" alt="void" width="80" height="80">
</div>

_Want to add your cat? Create a PR adding your cat's photo to_ [`docs/cats/` of the TypeScript SDK](https://github.com/agentcathq/agentcat-typescript-sdk/tree/main/docs/cats) _and update its README!_

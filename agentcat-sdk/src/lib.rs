//! Analytics for MCP servers built on [`rmcp`], the official Rust SDK of the Model Context
//! Protocol (MCP).
//!
//! [AgentCat](https://agentcat.com) shows the maintainers of an MCP server what agents use the
//! server for and where the agents get stuck. This crate records the tool calls of an `rmcp`
//! server and sends them to your `AgentCat` project.
//!
//! To start, wrap your server handler with [`track`] and serve the [`Tracked`] wrapper in its
//! place. The wrapper forwards every `ServerHandler` method to your handler.
//!
//! # What the SDK records
//!
//! The SDK publishes one event for each `tools/call` request. The event holds:
//!
//! - The tool name, the request with its arguments, the result, and how long the call took.
//! - The intent of the call: the text that the agent sends in the `context` parameter (see
//!   [What agents see](#what-agents-see)).
//! - The session: an ID that ties together the calls of one task.
//! - The actor: the user that your [`identify`](Options::identify) hook names.
//! - The error, when the call fails: an error result, a JSON-RPC error, a panic of the tool, or a
//!   call that was cancelled before it completed.
//! - Token estimates of the arguments and of the result.
//! - The names and versions of the MCP client and of your server.
//! - The tags and the properties that your [`event_tags`](Options::event_tags) and
//!   [`event_properties`](Options::event_properties) hooks return.
//!
//! The SDK records no other request: tool listings, resources and prompts give no events. Your
//! code can also publish custom events of its own ([`CustomEvent`]).
//!
//! When a tool panics, the SDK records the panic as the error of the call and then lets the panic
//! go on unchanged. When a call is cancelled before it completes (its future is dropped), the event
//! gets the error type `cancelled`. The SDK never changes the result or the error of a tool, apart
//! from the session data that [What agents see](#what-agents-see) describes.
//!
//! ## Errors and custom events
//!
//! `rmcp` turns an error that a tool returns into the text of an error result, so the event gets
//! only that text. Call [`capture_error`] before the tool returns the error. The event then also
//! gets the type of the error and the texts of its `source()` chain.
//!
//! A tool that returns `Err(ErrorData)` gives the client a JSON-RPC error, as `rmcp` sends it.
//! The event gets the name of the JSON-RPC error code as `error.type`. The `error` of an event
//! always has the platform `rust`, and it has no parsed stack frames.
//!
//! The SDK gives each tool call a [`CallContext`], which a tool reads as an
//! `Extension<CallContext>` parameter. With it, a tool can read its session ID and publish a
//! custom event with [`CallContext::publish_custom_event`]:
//!
//! ```no_run
//! use agentcat_sdk::{CallContext, CustomEvent, capture_error};
//! use rmcp::handler::server::common::Extension;
//! use rmcp::{tool, tool_router};
//!
//! #[derive(Clone)]
//! struct Files;
//!
//! #[tool_router]
//! impl Files {
//!     #[tool(description = "Reads the configuration file")]
//!     async fn read_config(
//!         &self,
//!         Extension(call): Extension<CallContext>,
//!     ) -> Result<String, String> {
//!         // A custom event in the session of this call.
//!         let mut event = CustomEvent::new().resource_name("config-read");
//!         if let Some(session) = call.session_id() {
//!             event = event.session_id(session);
//!         }
//!         let _ = call.publish_custom_event("proj_...", event);
//!
//!         std::fs::read_to_string("config.toml").map_err(|error| {
//!             // The event gets the type and the cause chain of the error.
//!             capture_error(&error);
//!             error.to_string()
//!         })
//!     }
//! }
//! ```
//!
//! Outside a tool call, publish a custom event with [`AgentCat::publish_custom_event`].
//!
//! # Quickstart: stdio
//!
//! Add the crate, `rmcp` with its stdio transport, tokio, and what the `#[tool]` macros need:
//!
//! ```toml
//! [dependencies]
//! agentcat-sdk = "1"
//! rmcp = { version = "3", features = ["transport-io"] }
//! tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
//! schemars = "1"
//! serde = { version = "1", features = ["derive"] }
//! ```
//!
//! Then wrap your server with [`track`], and serve the wrapper where you served your server:
//!
//! ```no_run
//! use agentcat_sdk::Options;
//! use rmcp::handler::server::wrapper::Parameters;
//! use rmcp::model::{InitializeResult, ServerCapabilities};
//! use rmcp::{ServerHandler, ServiceExt, tool, tool_handler, tool_router};
//! use schemars::JsonSchema;
//! use serde::Deserialize;
//!
//! #[derive(Deserialize, JsonSchema)]
//! struct AddArgs {
//!     a: f64,
//!     b: f64,
//! }
//!
//! #[derive(Clone)]
//! struct Calculator;
//!
//! #[tool_router]
//! impl Calculator {
//!     #[tool(description = "Adds two numbers")]
//!     async fn add(&self, Parameters(args): Parameters<AddArgs>) -> String {
//!         (args.a + args.b).to_string()
//!     }
//! }
//!
//! #[tool_handler]
//! impl ServerHandler for Calculator {
//!     fn get_info(&self) -> InitializeResult {
//!         InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
//!     }
//! }
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // One line: wrap the server with AgentCat.
//!     let server = agentcat_sdk::track(Calculator, "proj_...", Options::default());
//!     let running = server.serve(rmcp::transport::stdio()).await?;
//!     running.waiting().await?;
//!     Ok(())
//! }
//! ```
//!
//! Replace `proj_...` with the ID of your project. To read the project ID from the environment
//! variable [`AGENTCAT_PROJECT_ID`] instead, wrap the server with
//! `AgentCat::from_env(Options::default()).track(Calculator)` ([`AgentCat::from_env`]).
//!
//! A stdio server talks JSON-RPC on stdout. The SDK writes nothing to stdout or stderr. Send your
//! own logs to stderr (see [Logging](#logging)).
//!
//! The server ends when the client closes stdin. Then the last clone of the handle drops, and the
//! drop delivers the queued events, for up to 5 s (see [Delivery and
//! shutdown](#delivery-and-shutdown)). A stdio server needs no `shutdown` call.
//!
//! # Quickstart: streamable HTTP
//!
//! A streamable HTTP server builds a new server handler for each MCP session. Build one
//! [`AgentCat`] handle for the process, and wrap each new handler with [`AgentCat::track`]. A
//! clone of the handle is cheap, and `track` does no other work, so the handle can serve every
//! session.
//!
//! This server needs the streamable HTTP server of `rmcp`, more tokio features, and `axum`:
//!
//! ```toml
//! [dependencies]
//! agentcat-sdk = "1"
//! rmcp = { version = "3", features = ["transport-streamable-http-server"] }
//! tokio = { version = "1", features = ["macros", "rt-multi-thread", "net", "signal"] }
//! axum = "0.8"
//! schemars = "1"
//! serde = { version = "1", features = ["derive"] }
//! ```
//!
//! ```no_run
//! # use rmcp::handler::server::wrapper::Parameters;
//! # use rmcp::{ServerHandler, tool, tool_handler, tool_router};
//! # #[derive(serde::Deserialize, schemars::JsonSchema)]
//! # struct AddArgs { a: f64, b: f64 }
//! # #[derive(Clone)]
//! # struct Calculator;
//! # #[tool_router]
//! # impl Calculator {
//! #     #[tool(description = "Adds two numbers")]
//! #     async fn add(&self, Parameters(args): Parameters<AddArgs>) -> String {
//! #         (args.a + args.b).to_string()
//! #     }
//! # }
//! # #[tool_handler]
//! # impl ServerHandler for Calculator {}
//! use std::sync::Arc;
//! use std::time::Duration;
//!
//! use agentcat_sdk::{AgentCat, Options};
//! use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
//! use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // One handle for the whole process.
//!     let agentcat = AgentCat::new("proj_...", Options::default());
//!
//!     // rmcp calls this closure for each new MCP session. Wrap each new server.
//!     let handle = agentcat.clone();
//!     let service = StreamableHttpService::new(
//!         move || Ok(handle.track(Calculator)),
//!         Arc::new(LocalSessionManager::default()),
//!         StreamableHttpServerConfig::default(),
//!     );
//!     let router = axum::Router::new().route_service("/mcp", service);
//!
//!     let listener = tokio::net::TcpListener::bind("127.0.0.1:8000").await?;
//!     tokio::select! {
//!         served = axum::serve(listener, router) => served?,
//!         _ = tokio::signal::ctrl_c() => {}
//!     }
//!
//!     // Deliver the queued events before the process ends.
//!     agentcat.shutdown(Duration::from_secs(5)).await;
//!     Ok(())
//! }
//! ```
//!
//! An HTTP server does not end when a client goes away, and the SDK installs no signal handler.
//! So call [`AgentCat::shutdown`] after your server stops: the queued events then go out before
//! the process ends.
//!
//! This quickstart stops only on Ctrl-C. Containers and process managers stop a server with
//! SIGTERM, and a server that does not catch it skips `shutdown` and loses the queued events. The
//! [`streamable_http.rs`](https://github.com/agentcathq/agentcat-rust-sdk/blob/main/agentcat-sdk/examples/streamable_http.rs)
//! example stops on SIGTERM (Unix) or Ctrl-C: copy its `stop_signal` function in place of
//! `tokio::signal::ctrl_c()`.
//!
//! ## Stateless HTTP
//!
//! In stateless mode, `rmcp` builds a new server handler for each request. The setup is the same
//! as above. Only the configuration of the service changes:
//!
//! ```
//! use rmcp::transport::streamable_http_server::StreamableHttpServerConfig;
//!
//! // No MCP sessions, and JSON responses instead of server-sent events.
//! let config = StreamableHttpServerConfig::default()
//!     .with_legacy_session_mode(false)
//!     .with_json_response(true);
//! # let _ = config;
//! ```
//!
//! The SDK never uses the MCP session of the transport (the `mcp-session-id` header) to group
//! calls. The `session_id` parameter groups them on every transport, so a stateless server loses
//! no sessions. On a stateless server, an event names the client only when the request names it
//! in its `_meta`.
//!
//! # What agents see
//!
//! The SDK changes the tool list that `tools/list` returns:
//!
//! - Each tool gets a `context` parameter, in which the agent says why it calls the tool. The SDK
//!   records this text as the intent of the call.
//! - Each tool gets a `session_id` parameter, which ties the calls of one task together.
//! - With [`enable_agent_tracking`](Options::enable_agent_tracking) on, each tool gets an
//!   `agent_id` parameter, in which the agent names itself.
//! - The list gets one more tool, `get_more_tools`, through which agents report a tool that they
//!   miss.
//!
//! A tool that declares its own parameter with one of these names keeps it, and the SDK does not
//! add its own. Before your tool runs, the SDK removes the parameters that it added from the
//! arguments, so your tool never sees them. The table of [Options](#options) names the setter of
//! each change.
//!
//! ## The first call of a task
//!
//! The first call of a task sends `session_id` as `start`, or sends no `session_id`. The SDK then
//! issues a new session ID, which starts with `ses_`. It puts a text block before the content of
//! the tool's own result. The block begins `[session_id issued` and holds the line
//! `session_id: ses_...`. The agent sends that ID in every later call of the task, and the SDK
//! records those calls in the same session.
//!
//! A client or a test that reads the first content block of a first call gets this block, not
//! the content of the tool. An error result gets the block too. A JSON-RPC error
//! (`Err(ErrorData)`) has no content, so it gets no block.
//!
//! A `session_id` that is neither `start` nor a valid `ses_` ID gets a text block that begins
//! `[session_id unrecognized`. The call then has no session.
//!
//! When a tool declares an output schema, the SDK adds an `mcp_session` property to the schema. A
//! structured result of that tool then gets `mcp_session` as its first key. It holds the session
//! ID, the agent ID when the agent sent one, and a status.
//!
//! To turn this off, set a [`resolve_session_id`](Options::resolve_session_id) hook. The hook
//! then decides the session of each call, the tools get no `session_id` parameter, and the
//! results get no text block. The SDK derives the session ID from the value of the hook and your
//! project ID, so the same value always gives the same session. When the hook returns `None`, the
//! call gets a new session ID that the agent never learns.
//!
//! # Options
//!
//! [`Options::default`] has the defaults in the table below. Each setter consumes the options and
//! returns them changed:
//!
//! ```
//! use agentcat_sdk::{HookError, Options, ToolCall, UserIdentity};
//!
//! let options = Options::default()
//!     .enable_agent_tracking(true)
//!     .identify(|call: ToolCall| async move {
//!         // Read your own data about the caller, for example an HTTP header.
//!         let user = call
//!             .headers()
//!             .and_then(|headers| headers.get("x-user-id"))
//!             .and_then(|value| value.as_str())
//!             .map(UserIdentity::new);
//!         Ok::<_, HookError>(user)
//!     });
//! # let _ = options;
//! ```
//!
//! | Setter | Default |
//! |---|---|
//! | [`enable_tool_call_context`](Options::enable_tool_call_context) | `true`: tools get `context` |
//! | [`custom_context_description`](Options::custom_context_description) | empty: the default description of `context` |
//! | [`enable_tracing`](Options::enable_tracing) | `true`: the SDK records calls, and tools get `session_id` |
//! | [`enable_agent_tracking`](Options::enable_agent_tracking) | `false`: tools get no `agent_id` |
//! | [`enable_report_missing`](Options::enable_report_missing) | `true`: the list gets `get_more_tools` |
//! | [`resolve_session_id`](Options::resolve_session_id) | none: the SDK issues the session IDs |
//! | [`identify`](Options::identify) | none: calls have no actor |
//! | [`event_tags`](Options::event_tags) | none |
//! | [`event_properties`](Options::event_properties) | none |
//! | [`redact_event`](Options::redact_event) | none |
//! | [`redact_sensitive_information`](Options::redact_sensitive_information) | none |
//! | [`api_base_url`](Options::api_base_url) | empty: [`AGENTCAT_API_URL`], then [`MCPCAT_API_URL`], then `https://api.agentcat.com` |
//! | [`log_destination`](Options::log_destination) | [`LogDestination::File`]: `~/agentcat.log` |
//!
//! This release has no exporters (OpenTelemetry, Datadog, Sentry, `PostHog`) and no SDK
//! diagnostics.
//!
//! The hooks are async closures. The [`identify`](Options::identify),
//! [`event_tags`](Options::event_tags), [`event_properties`](Options::event_properties) and
//! [`resolve_session_id`](Options::resolve_session_id) hooks get a [`ToolCall`]. It holds the tool
//! name, the arguments, the `_meta` of the request, the HTTP headers, the client, and the `rmcp`
//! request context. The SDK starts these hooks on the tokio runtime of the call. The call never
//! waits for them, so a slow hook never delays a tool call. When the event is ready to go out, the
//! SDK waits at most 30 s for each hook. It stops a hook that has not completed by then, and the
//! event goes out without the value of that hook. A hook that returns an error gives no value, and
//! the SDK logs the error.
//!
//! The redaction hooks run before an event leaves the process:
//! [`redact_event`](Options::redact_event) on the whole [`Event`], then
//! [`redact_sensitive_information`](Options::redact_sensitive_information) on each string of the
//! event. When a redaction hook fails, the SDK drops the event and does not send it unredacted.
//!
//! # Environment variables
//!
//! | Variable | Effect |
//! |---|---|
//! | [`AGENTCAT_PROJECT_ID`] | The project ID that [`AgentCat::from_env`] reads. When it is unset or empty, the handle is telemetry only. |
//! | [`AGENTCAT_API_URL`] | The base URL of the `AgentCat` API, when the options set none. |
//! | [`MCPCAT_API_URL`] | The former name of `AGENTCAT_API_URL`, read when that one is unset or empty. |
//! | [`AGENTCAT_ENABLED`] | `false`, `0`, `no` or `off`, in any case, turns the SDK off. Any other value, or none, leaves it on. |
//!
//! A telemetry-only handle (an empty project ID) builds each event and runs the hooks, but sends
//! nothing to `AgentCat`.
//!
//! When `AGENTCAT_ENABLED` turns the SDK off, a handle starts no thread, and its [`Tracked`]
//! servers forward every request to your handler untouched. A tool still gets a
//! [`CallContext`], with no session. Custom events publish nothing, and `flush` and `shutdown`
//! return `true`. The SDK has no test detection: it stays on in tests unless you set this
//! variable.
//!
//! # Features
//!
//! | Feature | Default | Effect |
//! |---|---|---|
//! | `rustls` | on | TLS with rustls for the connection to the `AgentCat` API. |
//! | `native-tls` | off | TLS with the TLS library of the platform. |
//! | `test-util` | off | The [`testing`] module with [`testing::Fake`]. |
//!
//! Turn on one TLS feature. To use `native-tls`, turn off the default features:
//!
//! ```toml
//! [dependencies]
//! agentcat-sdk = { version = "1", default-features = false, features = ["native-tls"] }
//! ```
//!
//! Without a TLS feature, the SDK can send events only to an `http:` URL. It logs a warning when
//! the URL is an `https:` URL.
//!
//! # Delivery and shutdown
//!
//! Each [`AgentCat`] handle delivers its events from its own thread, `agentcat-delivery`, with its
//! own queue and its own API URL. The first event or log line of the handle starts the thread. A
//! tool call only puts its event in the queue of the handle, so the call never waits for the
//! network. The delivery has these limits:
//!
//! - The queue holds at most 10 000 events. When it is full, the SDK drops the oldest event.
//! - At most 5 events of a handle are in progress at a time.
//! - The SDK tries a failed send 3 more times, after 1, 2 and 4 s. Each attempt has a timeout of
//!   10 s.
//!
//! Your code can wait for the delivery:
//!
//! - [`AgentCat::flush`] waits until the handle delivered every event it took, for at most a
//!   timeout. It returns `true` when that happened in time.
//! - [`AgentCat::shutdown`] does the same, then stops the handle for good. The handle drops every
//!   later event.
//! - [`AgentCat::flush_blocking`] and [`AgentCat::shutdown_blocking`] block the calling thread
//!   instead. They need no tokio runtime.
//!
//! The SDK installs no signal or exit handler. These calls and the drain of the last drop (below)
//! deliver the queued events.
//!
//! ```no_run
//! # async fn run(agentcat: agentcat_sdk::AgentCat) {
//! use std::time::Duration;
//!
//! // ... serve until the server stops ...
//! let delivered = agentcat.shutdown(Duration::from_secs(5)).await;
//! if !delivered {
//!     // Some events did not go out within 5 s.
//! }
//! # }
//! ```
//!
//! When the last clone of a handle drops, the SDK runs `shutdown` with a timeout of 5 s. The drop
//! blocks the dropping thread until the shutdown ends. The [`Tracked`] servers hold clones of
//! the handle too. This drain does not run after `std::process::exit`, or after a panic with
//! `panic = "abort"`.
//!
//! On a current-thread runtime, a drop on the thread of the runtime blocks its only thread. Then
//! no hook can run during the wait, and the SDK drops an event that still needs a hook after 5 s.
//! On such a runtime, call `shutdown` before the runtime ends. See [dropping the last
//! handle](AgentCat#dropping-the-last-handle).
//!
//! # Logging
//!
//! The SDK never writes to stdout or stderr. It writes its own log lines to the file `agentcat.log`
//! in your home directory (`~/agentcat.log`). Each line holds the time, the versions of the SDK, of
//! Rust and of `rmcp`, and the message:
//!
//! ```text
//! [2026-10-28T09:15:02.137Z] [sdk=1.0.0 rust=1.88.0 rmcp=3.6.0] Successfully sent event ...
//! ```
//!
//! When the SDK finds no home directory, it writes no log. It ignores the errors of the file.
//!
//! To send the lines to `tracing` instead, set [`LogDestination::Tracing`]:
//!
//! ```
//! use agentcat_sdk::{LogDestination, Options};
//!
//! let options = Options::default().log_destination(LogDestination::Tracing);
//! # let _ = options;
//! ```
//!
//! The `tracing` events have the target `agentcat`. A line about a failure or an error is an
//! `ERROR` event. A warning is a `WARN` event, and any other line is an `INFO` event. The delivery
//! thread of the handle emits the events, so set a global subscriber, for example with
//! `tracing::subscriber::set_global_default`. A subscriber that `with_default` sets on another
//! thread does not see them.
//!
//! The log destination applies to the whole process. The first handle built in the process
//! decides it.
//!
//! ## Stdio servers
//!
//! A stdio server talks JSON-RPC on stdout, so any other text on stdout breaks the connection to
//! the client. Send your own logs and your `tracing` output to stderr. The `fmt` subscriber of
//! `tracing-subscriber` writes to stdout unless you give it another writer, for example
//! `.with_writer(std::io::stderr)`. This also holds when you send the lines of the SDK to
//! `tracing`.
//!
//! # Testing
//!
//! ## A redaction hook
//!
//! To unit-test a [`redact_event`](Options::redact_event) hook, call it with an event from
//! [`Event::default`]. Set the fields that the hook reads:
//!
//! ```
//! use agentcat_sdk::{Event, HookError};
//!
//! # #[tokio::main(flavor = "current_thread")]
//! # async fn main() -> Result<(), HookError> {
//! // The closure that you pass to `Options::redact_event`.
//! let hook = |mut event: Event| async move {
//!     event.response = None;
//!     Ok::<_, HookError>(Some(event))
//! };
//!
//! let mut event = Event::default();
//! event.response = Some(serde_json::json!({"token": "fake-secret-42"}));
//! let redacted = hook(event).await?;
//! assert_eq!(redacted.and_then(|event| event.response), None);
//! # Ok(())
//! # }
//! ```
//!
//! ## A tracked server
//!
//! [`testing::Fake`] builds handles whose events stay in the process. It records each event as a
//! real handle sends it, after the hooks and the redaction.
//!
//! The test that follows serves the tracked server to an `rmcp` client in memory, calls a tool,
//! and makes sure that the fake recorded the call. It needs these dev-dependencies:
//!
//! - The `test-util` feature of the SDK, for `Fake`.
//! - The `client` feature of `rmcp`, for the client.
//! - The `io-util` feature of tokio, for `tokio::io::duplex`.
//! - The `macros` and `rt` features of tokio, for `#[tokio::test]`.
//!
//! ```toml
//! [dev-dependencies]
//! agentcat-sdk = { version = "1", features = ["test-util"] }
//! rmcp = { version = "3", features = ["client"] }
//! tokio = { version = "1", features = ["io-util", "macros", "rt"] }
//! ```
//!
//! In a `#[tokio::test]`:
//!
//! ```
//! # use rmcp::handler::server::wrapper::Parameters;
//! # use rmcp::{ServerHandler, tool, tool_handler, tool_router};
//! # #[derive(serde::Deserialize, schemars::JsonSchema)]
//! # struct AddArgs { a: f64, b: f64 }
//! # #[derive(Clone)]
//! # struct Calculator;
//! # #[tool_router]
//! # impl Calculator {
//! #     #[tool(description = "Adds two numbers")]
//! #     async fn add(&self, Parameters(args): Parameters<AddArgs>) -> String {
//! #         (args.a + args.b).to_string()
//! #     }
//! # }
//! # #[tool_handler]
//! # impl ServerHandler for Calculator {}
//! # #[cfg(feature = "test-util")]
//! # #[tokio::main(flavor = "current_thread")]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use agentcat_sdk::Options;
//! use agentcat_sdk::testing::Fake;
//! use rmcp::ServiceExt;
//! use rmcp::model::{
//!     CallToolRequestParams, ClientCapabilities, Implementation, InitializeRequestParams,
//! };
//!
//! let fake = Fake::new();
//! let server = fake.agentcat("proj_test", Options::default()).track(Calculator);
//!
//! // The server on one end of an in-memory stream, an rmcp client on the other.
//! let (server_end, client_end) = tokio::io::duplex(64 * 1024);
//! tokio::spawn(async move {
//!     if let Ok(running) = server.serve(server_end).await {
//!         let _ = running.waiting().await;
//!     }
//! });
//! let client = InitializeRequestParams::new(
//!     ClientCapabilities::default(),
//!     Implementation::new("test", "1.0.0"),
//! )
//! .serve(client_end)
//! .await?;
//!
//! let mut arguments = rmcp::model::JsonObject::new();
//! arguments.insert("a".into(), 1.into());
//! arguments.insert("b".into(), 2.into());
//! client
//!     .call_tool(CallToolRequestParams::new("add").with_arguments(arguments))
//!     .await?;
//!
//! fake.assert_tracked("add").await;
//! # Ok(())
//! # }
//! # #[cfg(not(feature = "test-util"))]
//! # fn main() {}
//! ```
//!
//! [`Fake::events`] returns the recorded events, for checks of your own.
//!
//! # Minimum supported Rust version
//!
//! The minimum supported Rust version (MSRV) is 1.88, the MSRV of `rmcp` 3. The MSRV is never
//! higher than the MSRV of the `rmcp` versions that this crate supports. The MSRV is always a Rust
//! release that is at least 6 months old. An MSRV increase is a minor release and has its own
//! CHANGELOG section.
//!
//! The crate supports `rmcp` 3, from 3.0.0. CI tests the latest patch of each 3.x minor release.
//!
//! # Known limits
//!
//! - A tracked server needs a tokio runtime with the time driver: the SDK times a rebuild of its
//!   tool registry with `tokio::time::timeout`, and tokio panics at such a timer on a runtime
//!   without the driver. `rmcp` uses timers too. `#[tokio::main]`, `#[tokio::test]` and
//!   `tokio::runtime::Runtime::new` turn the time driver on. A runtime from
//!   `tokio::runtime::Builder` needs `enable_time` or `enable_all`.
//! - The SDK does not support the `local` feature of `rmcp`. It spawns the hooks as `Send` tasks
//!   on the tokio runtime of the call.
//! - The SDK turns on the `preserve_order` feature of `serde_json`, so that `mcp_session` stays
//!   the first key of a result. Cargo turns a feature on for the whole build, so every
//!   `serde_json::Map` in your program keeps the order of insertion and does not sort its keys.
//! - When another crate in your build turns on the `arbitrary_precision` or the `float_roundtrip`
//!   feature of `serde_json`, Cargo turns it on for the SDK too. These features change how
//!   `serde_json` reads and writes numbers. Without them, `serde_json` can read a decimal of 16 or
//!   17 significant digits as the neighboring `f64`, and the SDK counts each number as the
//!   shortest text that reads back as the same `f64`. With either feature, the text of some
//!   numbers in the events, the token estimates and the size checks can change. The SDK logs a
//!   warning that names the feature each time its delivery thread starts.
//! - `parameters.extra.requestInfo.url` of an event is the URL that the `rmcp` service sees.
//!   `axum::Router::nest_service` removes the path prefix of the route before that, so the event
//!   records the URL without the prefix. `route_service` keeps the full path.
//! - The SDK keeps a registry of what the tool listings tell it, for each handle and each type of
//!   server handler, by tool name. It never removes an entry, so a server whose tool names differ
//!   per tenant grows the registry with each new name. A hand-written `ServerHandler` without
//!   `get_tool` depends on that registry (`#[tool_handler]` writes `get_tool` for you). On such a
//!   server, the listing of one session decides which arguments the SDK removes from the calls of
//!   another session.
//! - A hook that holds a clone of its [`AgentCat`] handle makes a reference cycle. The handle then
//!   never drops, so the drain of the last drop never runs. Let the hook capture only what it
//!   needs, or call [`AgentCat::shutdown`] before the process ends. A tool reaches its handle
//!   through [`CallContext::agentcat`] and needs no clone.
//! - The SDK gives the redaction hooks no time limit. A redaction hook that never completes holds
//!   one of the 5 delivery slots of its handle for good. Five such hooks stop the delivery of the
//!   handle.
//!
//! # Examples
//!
//! The repository has four examples. Each one serves the tools `add` and `echo`:
//!
//! - [`stdio.rs`](https://github.com/agentcathq/agentcat-rust-sdk/blob/main/agentcat-sdk/examples/stdio.rs):
//!   a stdio server.
//! - [`streamable_http.rs`](https://github.com/agentcathq/agentcat-rust-sdk/blob/main/agentcat-sdk/examples/streamable_http.rs):
//!   a streamable HTTP server with MCP sessions and server-sent events.
//! - [`stateless_http.rs`](https://github.com/agentcathq/agentcat-rust-sdk/blob/main/agentcat-sdk/examples/stateless_http.rs):
//!   a stateless streamable HTTP server with JSON responses.
//! - [`hooks.rs`](https://github.com/agentcathq/agentcat-rust-sdk/blob/main/agentcat-sdk/examples/hooks.rs):
//!   a stdio server that uses every hook except `resolve_session_id`, a custom event and
//!   [`capture_error`].
//!
// The `testing` module exists only with the `test-util` feature. Without the feature, the links
// of the guide to the module go to docs.rs, so that a doc build with the default features has no
// broken links.
#![cfg_attr(feature = "test-util", doc = "[`testing`]: testing")]
#![cfg_attr(feature = "test-util", doc = "[`testing::Fake`]: testing::Fake")]
#![cfg_attr(feature = "test-util", doc = "[`Fake::events`]: testing::Fake::events")]
#![cfg_attr(
    not(feature = "test-util"),
    doc = "[`testing`]: https://docs.rs/agentcat-sdk/latest/agentcat_sdk/testing/index.html"
)]
#![cfg_attr(
    not(feature = "test-util"),
    doc = "[`testing::Fake`]: https://docs.rs/agentcat-sdk/latest/agentcat_sdk/testing/struct.Fake.html"
)]
#![cfg_attr(
    not(feature = "test-util"),
    doc = "[`Fake::events`]: https://docs.rs/agentcat-sdk/latest/agentcat_sdk/testing/struct.Fake.html#method.events"
)]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]

mod call;
mod capture;
mod client;
mod config;
mod constants;
mod context;
mod custom;
mod delivery;
mod error;
mod event;
mod handles;
mod hooks;
mod inject;
mod logging;
mod pipeline;
mod registry;
mod sink;
#[cfg(test)]
mod test_support;
#[cfg(feature = "test-util")]
#[cfg_attr(docsrs, doc(cfg(feature = "test-util")))]
pub mod testing;
mod tokens;
mod tracked;
mod wire;

pub use capture::capture_error;
pub use client::{AgentCat, track};
pub use config::env::{AGENTCAT_API_URL, AGENTCAT_ENABLED, AGENTCAT_PROJECT_ID, MCPCAT_API_URL};
pub use config::{HookError, LogDestination, Options, UserIdentity};
pub use context::CallContext;
pub use custom::CustomEvent;
pub use error::Error;
pub use event::Event;
pub use hooks::ToolCall;
pub use tracked::Tracked;

/// The code blocks of `README.md`, compiled as doctests. In the repository the file is a symlink
/// to the root README, which GitHub shows; in the package it is a copy, which crates.io shows.
/// Only `cargo test --doc` builds it.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
pub struct ReadmeDoctests;

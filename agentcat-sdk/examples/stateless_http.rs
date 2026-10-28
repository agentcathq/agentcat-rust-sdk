//! A stateless streamable HTTP MCP server with AgentCat analytics. It keeps no MCP session: every
//! request gets its own tracked copy of the server, and every response is plain JSON.
//!
//! The server has two tools, `add` and `echo`. One `AgentCat` handle serves every request. AgentCat
//! groups the calls of one agent into a session by itself, so stateless servers need no session
//! setup.
//!
//! Run it with your project ID. `PORT` is optional and defaults to 8000.
//!
//! ```text
//! AGENTCAT_PROJECT_ID=proj_... PORT=8000 cargo run -p agentcat-sdk --example stateless_http
//! ```
//!
//! The MCP endpoint is `http://127.0.0.1:8000/mcp`. Stop the server with Ctrl-C or SIGTERM. It
//! then calls `shutdown`, which delivers the queued events (up to 5 s) before the process ends.
//! Logs go to stderr.
#![expect(
    clippy::print_stderr,
    reason = "this example logs to stderr; AgentCat writes nothing to stdout or stderr itself"
)]
#![expect(
    clippy::doc_markdown,
    reason = "AgentCat is the name of the product, not code"
)]
#![expect(
    clippy::unused_async_trait_impl,
    reason = "the methods #[tool_handler] generates are async without an await"
)]

use std::sync::Arc;
use std::time::Duration;

use agentcat_sdk::{AgentCat, Options};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{Implementation, InitializeResult, ServerCapabilities};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use tokio::net::TcpListener;

/// The arguments of the `add` tool.
#[derive(Deserialize, JsonSchema)]
struct AddArgs {
    /// The first number.
    a: f64,
    /// The second number.
    b: f64,
}

/// The arguments of the `echo` tool.
#[derive(Deserialize, JsonSchema)]
struct EchoArgs {
    /// The text to send back.
    text: String,
}

/// Your MCP server. It knows nothing about AgentCat.
#[derive(Clone)]
struct Demo;

#[tool_router]
impl Demo {
    #[tool(description = "Adds two numbers")]
    async fn add(&self, Parameters(args): Parameters<AddArgs>) -> String {
        (args.a + args.b).to_string()
    }

    #[tool(description = "Sends the text back")]
    async fn echo(&self, Parameters(args): Parameters<EchoArgs>) -> String {
        args.text
    }
}

#[tool_handler]
impl ServerHandler for Demo {
    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "agentcat-example-stateless-http",
                "1.0.0",
            ))
    }
}

/// The port to listen on: `PORT`, or 8000.
#[expect(
    clippy::disallowed_methods,
    reason = "an example reads its port from the environment"
)]
fn port() -> u16 {
    std::env::var("PORT")
        .ok()
        .and_then(|port| port.parse().ok())
        .unwrap_or(8000)
}

/// Resolves on SIGTERM (Unix) or Ctrl-C.
async fn stop_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        if let Ok(mut terminate) = signal(SignalKind::terminate()) {
            tokio::select! {
                _ = terminate.recv() => {}
                _ = tokio::signal::ctrl_c() => {}
            }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // One handle for the whole process. It reads AGENTCAT_PROJECT_ID, and AGENTCAT_API_URL when set.
    let agentcat = AgentCat::from_env(Options::default());

    // rmcp calls this closure once for each request.
    let handle = agentcat.clone();
    let service = StreamableHttpService::new(
        move || Ok(handle.track(Demo)),
        Arc::new(LocalSessionManager::default()),
        // No sessions, and JSON responses instead of SSE.
        StreamableHttpServerConfig::default()
            .with_legacy_session_mode(false)
            .with_json_response(true),
    );
    let router = axum::Router::new().route_service("/mcp", service);

    let listener = TcpListener::bind(("127.0.0.1", port())).await?;
    eprintln!("listening on http://{}/mcp", listener.local_addr()?);
    tokio::select! {
        served = axum::serve(listener, router) => served?,
        () = stop_signal() => {}
    }

    // Deliver the events that are still queued before the process ends.
    let delivered = agentcat.shutdown(Duration::from_secs(5)).await;
    eprintln!("stopped; all events delivered: {delivered}");
    Ok(())
}

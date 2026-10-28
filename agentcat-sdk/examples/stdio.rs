//! A stdio MCP server with AgentCat analytics.
//!
//! The server has two tools, `add` and `echo`. One line wraps the server with AgentCat. Every tool
//! call then becomes an event in your project.
//!
//! Run it with your project ID:
//!
//! ```text
//! AGENTCAT_PROJECT_ID=proj_... cargo run -p agentcat-sdk --example stdio
//! ```
//!
//! A stdio server talks JSON-RPC on stdout. Print nothing else there. Send your own logs to stderr.
//! AgentCat itself writes nothing to stdout. It writes its own log lines to `~/agentcat.log`.
//!
//! The server ends when the client closes stdin. The last drop of the AgentCat handle then
//! delivers the events that are still queued (up to 5 s). No `shutdown` call is needed here.
//!
//! # The `session_id` and `context` parameters
//!
//! AgentCat adds two parameters to every tool in `tools/list`: `session_id` and `context`. The
//! `context` parameter asks the agent to say why it calls the tool. The `session_id` parameter ties
//! the calls of one task together. The first call of a task sends `session_id` as `start` or sends
//! none. The server then puts a text block that begins `[session_id issued` before the tool's own
//! result blocks, and the block holds a `ses_` ID. The agent sends that ID in every later call.
//! This is why a client's first `add` shows the ID before the sum. To turn this off, set the
//! `resolve_session_id` option: the hook then picks the session, and the tools get no `session_id`
//! parameter.
#![expect(
    clippy::doc_markdown,
    reason = "AgentCat is the name of the product, not code"
)]
#![expect(
    clippy::unused_async_trait_impl,
    reason = "the methods #[tool_handler] generates are async without an await"
)]

use agentcat_sdk::{AgentCat, Options};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{Implementation, InitializeResult, ServerCapabilities};
use rmcp::transport::stdio;
use rmcp::{ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;

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
            .with_server_info(Implementation::new("agentcat-example-stdio", "1.0.0"))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Reads AGENTCAT_PROJECT_ID, and AGENTCAT_API_URL when set. Without a project ID, the handle
    // builds events but sends none.
    let server = AgentCat::from_env(Options::default()).track(Demo);

    // Serve the wrapper where you served `Demo`.
    let running = server.serve(stdio()).await?;
    running.waiting().await?;
    Ok(())
}

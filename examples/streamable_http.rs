//! A streamable HTTP MCP server tracked by AgentCat, with actor
//! identification from a request header.
//!
//! ```sh
//! AGENTCAT_PROJECT_ID=proj_... cargo run --example streamable_http
//! ```

use std::time::Duration;

use agentcat::{AgentCat, UserIdentity};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, InitializeResult, ServerCapabilities};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use rmcp::{ErrorData as McpError, ServerHandler, tool, tool_handler, tool_router};
use serde::Deserialize;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct Greet {
    /// Who to greet.
    name: String,
}

#[derive(Clone)]
struct Greeter {
    tool_router: ToolRouter<Greeter>,
}

#[tool_router]
impl Greeter {
    fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "Greet someone")]
    async fn greet(&self, Parameters(Greet { name }): Parameters<Greet>) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::success(vec![ContentBlock::text(format!(
            "Hello, {name}!"
        ))]))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Greeter {
    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("greeter-example", env!("CARGO_PKG_VERSION")))
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let project_id = std::env::var("AGENTCAT_PROJECT_ID").unwrap_or_else(|_| "proj_0000000".into());
    let agentcat = AgentCat::builder(project_id)
        .identify(|call| {
            // Your auth middleware can also put typed claims into
            // `call.http_request().extensions`.
            let user = call.header("x-user-id").map(str::to_owned);
            async move { user.map(UserIdentity::new) }
        })
        .build();

    let ct = tokio_util::sync::CancellationToken::new();
    let factory_agentcat = agentcat.clone();
    let service = StreamableHttpService::new(
        // Runs per session, or per request in stateless mode.
        move || Ok(factory_agentcat.track(Greeter::new())),
        LocalSessionManager::default().into(),
        StreamableHttpServerConfig::default().with_cancellation_token(ct.child_token()),
    );
    let app = axum::Router::new().nest_service("/mcp", service);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8000").await?;
    println!("listening on http://127.0.0.1:8000/mcp");
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = tokio::signal::ctrl_c().await;
            ct.cancel();
        })
        .await?;

    agentcat.shutdown(Duration::from_secs(5)).await;
    Ok(())
}

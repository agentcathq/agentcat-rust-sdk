//! A stdio MCP server tracked by AgentCat.
//!
//! ```sh
//! AGENTCAT_PROJECT_ID=proj_... cargo run --example stdio
//! ```

use std::sync::{Arc, Mutex};
use std::time::Duration;

use agentcat::{AgentCat, UserIdentity};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{CallToolResult, ContentBlock, Implementation, InitializeResult, ServerCapabilities};
use rmcp::{ErrorData as McpError, ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct AddTodo {
    /// The todo text.
    text: String,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
struct Stats {
    count: usize,
}

#[derive(Clone)]
struct Todos {
    items: Arc<Mutex<Vec<String>>>,
    tool_router: ToolRouter<Todos>,
}

#[tool_router]
impl Todos {
    fn new() -> Self {
        Self {
            items: Arc::default(),
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "Add a todo item")]
    async fn add_todo(
        &self,
        Parameters(AddTodo { text }): Parameters<AddTodo>,
    ) -> Result<CallToolResult, McpError> {
        self.items
            .lock()
            .map_err(|_| McpError::internal_error("lock", None))?
            .push(text.clone());
        Ok(CallToolResult::success(vec![ContentBlock::text(format!(
            "Added todo: {text}"
        ))]))
    }

    #[tool(description = "Count the todo items")]
    async fn stats(&self) -> Result<Json<Stats>, McpError> {
        let count = self
            .items
            .lock()
            .map_err(|_| McpError::internal_error("lock", None))?
            .len();
        Ok(Json(Stats { count }))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Todos {
    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("todo-example", env!("CARGO_PKG_VERSION")))
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let project_id = std::env::var("AGENTCAT_PROJECT_ID").unwrap_or_else(|_| "proj_0000000".into());
    let agentcat = AgentCat::builder(project_id)
        .enable_agent_tracking(true)
        // Over stdio there are no HTTP headers; identify from your own state.
        .identify(|_call| async move { Some(UserIdentity::new(whoami())) })
        .build();

    let service = agentcat
        .track(Todos::new())
        .serve(rmcp::transport::stdio())
        .await?;
    service.waiting().await?;

    // Deliver queued events before the process exits.
    agentcat::shutdown(Duration::from_secs(5)).await;
    Ok(())
}

fn whoami() -> String {
    std::env::var("USER").unwrap_or_else(|_| "local-user".into())
}

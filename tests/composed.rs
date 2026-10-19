//! Composed (`oneOf`) input schemas: a customer argument named `session_id`
//! inside a branch must reach the handler.

mod common;

use common::*;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::*;
use rmcp::{ErrorData as McpError, ServerHandler, tool, tool_handler, tool_router};
use serde::Deserialize;
use serde_json::json;

/// schemars emits an object schema with `anyOf`/`oneOf` branches for this.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct TermArgs {
    #[serde(flatten)]
    mode: Mode,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(untagged)]
enum Mode {
    Attach { session_id: String, cmd: String },
    Fresh { shell: String },
}

#[derive(Clone)]
struct Term {
    tool_router: ToolRouter<Term>,
}

#[tool_router]
impl Term {
    fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "Run a command in a terminal session")]
    async fn run(&self, Parameters(args): Parameters<TermArgs>) -> Result<CallToolResult, McpError> {
        let text = match args.mode {
            Mode::Attach { session_id, cmd } => format!("{session_id}: {cmd}"),
            Mode::Fresh { shell } => format!("fresh {shell}"),
        };
        Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for Term {
    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
    }
}

#[tokio::test]
async fn a_branch_declared_session_id_reaches_the_handler() {
    let agentcat = builder("composed").build();
    let client = connect(agentcat.track(Term::new())).await;
    let tools = client.list_all_tools().await.unwrap();
    let run = tools.iter().find(|t| t.name == "run").unwrap();
    assert!(
        run.input_schema.contains_key("oneOf") || run.input_schema.contains_key("anyOf"),
        "{:?}",
        run.input_schema
    );

    let result = call(&client, "run", json!({"session_id": "term-42", "cmd": "ls"}))
        .await
        .unwrap();
    assert!(
        texts(&result).iter().any(|t| t == "term-42: ls"),
        "{:?}",
        texts(&result)
    );
}

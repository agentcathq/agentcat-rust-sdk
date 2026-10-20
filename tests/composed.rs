//! Composed (`oneOf`) input schemas: nothing is injected into them, so
//! nothing is stripped. A customer argument named `session_id` inside a
//! branch, and handles a client echoes anyway, reach the handler.

mod common;

use std::sync::Arc;

use common::*;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::*;
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler, tool, tool_handler, tool_router};
use serde::Deserialize;
use serde_json::{Value, json};

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

/// A hand-written server with one composed-schema tool that echoes the
/// arguments it receives.
#[derive(Clone)]
struct EchoArgs;

impl ServerHandler for EchoArgs {
    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let schema: JsonObject = serde_json::from_value(json!({
            "type": "object",
            "oneOf": [
                {"properties": {"a": {"type": "string"}}},
                {"properties": {"b": {"type": "string"}}}
            ]
        }))
        .unwrap();
        Ok(ListToolsResult::with_all_items(vec![Tool::new(
            "echo_args",
            "Echo the arguments it receives",
            Arc::new(schema),
        )]))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let args = Value::Object(request.arguments.unwrap_or_default());
        Ok(CallToolResult::success(vec![ContentBlock::text(args.to_string())]).into())
    }
}

#[tokio::test]
async fn echoed_handles_reach_a_composed_tool() {
    let agentcat = builder("composed-echo").enable_agent_tracking(true).build();
    let client = connect(agentcat.track(EchoArgs)).await;
    let tools = client.list_all_tools().await.unwrap();
    let echo = tools.iter().find(|t| t.name == "echo_args").unwrap();
    assert!(
        !echo.input_schema.contains_key("properties"),
        "{:?}",
        echo.input_schema
    );

    let args = json!({"a": "x", "session_id": sid("composed"), "agent_id": "opus|cc|k3n9x"});
    let result = call(&client, "echo_args", args.clone()).await.unwrap();
    let echoed = texts(&result).pop().unwrap();
    assert!(
        echoed.contains("session_id") && echoed.contains("agent_id"),
        "{echoed}"
    );
    assert_eq!(serde_json::from_str::<Value>(&echoed).unwrap(), args);
}

//! Multi-round-trip tool calls (SEP-2322): an `input_required` round is
//! tagged and left undecorated; the continuation round is tagged and carries
//! the mint-back.

mod common;

use std::sync::Arc;

use common::*;
use rmcp::model::*;
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler};
use serde_json::{Value, json};

#[derive(Clone)]
struct AskServer;

fn ask_tool() -> Tool {
    let schema: JsonObject = serde_json::from_value(json!({"type": "object", "properties": {}})).unwrap();
    Tool::new("ask", "Asks the client for input first", Arc::new(schema))
}

impl ServerHandler for AskServer {
    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(vec![ask_tool()]))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        if request.input_responses.is_some() {
            return Ok(CallToolResult::success(vec![ContentBlock::text("done")]).into());
        }
        let mut requests = InputRequests::new();
        requests.insert("roots".into(), InputRequest::ListRoots(ListRootsRequest::default()));
        Ok(InputRequiredResult::new(Some(requests), Some("state-1".into())).into())
    }
}

#[tokio::test]
async fn input_required_rounds_are_tagged_and_not_decorated() {
    let agentcat = builder("mrtr").build();
    let client = connect(agentcat.track(AskServer)).await;

    // First round. A pre-2026 client may get an error instead of the
    // intermediate result; either way the round is recorded.
    let first = client
        .call_tool_once(CallToolRequestParams::new("ask").with_arguments(json!({"session_id": "start"}).as_object().cloned().unwrap()))
        .await;
    if let Ok(CallToolResponse::InputRequired(result)) = &first {
        let v = serde_json::to_value(result).unwrap();
        assert!(!v.to_string().contains("[session_id"), "no mint-back on an intermediate round: {v}");
    }

    // Continuation round with the client's responses.
    let mut params = CallToolRequestParams::new("ask");
    params.input_responses = Some(InputResponses::from([("roots".to_owned(), json!({"roots": []}))]));
    params.request_state = Some("state-1".into());
    let second = client.call_tool_once(params).await.unwrap();
    let CallToolResponse::Complete(done) = second else { panic!("expected completion") };
    assert!(texts(&done)[0].starts_with("[session_id issued"));

    let events = api().events_for("mrtr", 2).await;
    let tag = |e: &Value| e["tags"]["agentcat_mrtr"].as_str().map(str::to_owned);
    let mut tags: Vec<Option<String>> = events.iter().map(tag).collect();
    tags.sort();
    assert_eq!(tags, [Some("continuation".to_owned()), Some("input_required".to_owned())]);
    let continuation = events.iter().find(|e| tag(e).as_deref() == Some("continuation")).unwrap();
    assert_eq!(continuation["parameters"]["request"]["params"]["requestState"], "state-1");
}

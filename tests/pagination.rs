//! Paginated tool listings: every page the server serves advertises
//! `get_more_tools`, as the TypeScript SDK appends it to each page, and a
//! call before any listing reads every page to learn what was injected.

mod common;

use std::sync::Arc;

use common::*;
use rmcp::model::*;
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler};
use serde_json::{Map, Value, json};

/// A hand-written server that lists one tool per page, on two pages. It has
/// no `get_tool`, and answers a call to any name with structured content.
#[derive(Clone)]
struct Paged;

fn tool(name: &'static str) -> Tool {
    let schema: JsonObject = serde_json::from_value(json!({"type": "object", "properties": {}})).unwrap();
    let output: JsonObject =
        serde_json::from_value(json!({"type": "object", "properties": {"n": {"type": "number"}}})).unwrap();
    let mut tool = Tool::new(name, "A tool on one page of the listing", Arc::new(schema));
    tool.output_schema = Some(Arc::new(output));
    tool
}

impl ServerHandler for Paged {
    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        match request.and_then(|r| r.cursor).as_deref() {
            None => Ok(ListToolsResult {
                next_cursor: Some("p2".into()),
                ..ListToolsResult::with_all_items(vec![tool("first")])
            }),
            Some("p2") => Ok(ListToolsResult::with_all_items(vec![tool("second")])),
            Some(other) => Err(McpError::invalid_params(format!("unknown cursor {other}"), None)),
        }
    }

    async fn call_tool(
        &self,
        _request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        Ok(CallToolResult::structured(json!({"n": 1})).into())
    }
}

fn names(page: &ListToolsResult) -> Vec<&str> {
    page.tools.iter().map(|t| t.name.as_ref()).collect()
}

#[tokio::test]
async fn paginated_listings_append_get_more_tools_to_every_page() {
    let agentcat = builder("pagination").build();
    let client = connect(agentcat.track(Paged)).await;

    let first = client.list_tools(None).await.unwrap();
    assert_eq!(names(&first), ["first", "get_more_tools"]);
    assert_eq!(first.next_cursor.as_deref(), Some("p2"));

    let second = client
        .list_tools(Some(
            PaginatedRequestParams::default().with_cursor(first.next_cursor.clone()),
        ))
        .await
        .unwrap();
    assert_eq!(names(&second), ["second", "get_more_tools"]);
    assert_eq!(second.next_cursor, None);
}

/// The structured content of a call on a fresh instance, before any listing.
async fn structured_before_any_listing(server_name: &str, tool: &str) -> Map<String, Value> {
    let agentcat = builder(server_name).build();
    let client = connect(agentcat.track(Paged)).await;
    let result = call(&client, tool, json!({"context": "A call before any listing."}))
        .await
        .unwrap();
    match result.structured_content {
        Some(Value::Object(map)) => map,
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_call_before_any_listing_rebuilds_the_registry_from_every_page() {
    // `second` is on the second page, and its output schema declares
    // `mcp_session`: the result mirrors the mint-back.
    let listed = structured_before_any_listing("pagination-rebuild", "second").await;
    assert_eq!(
        listed.keys().next().map(String::as_str),
        Some("mcp_session"),
        "{listed:?}"
    );

    // A tool on no page is unknown to a registry that could be read: nothing
    // is mirrored.
    let unlisted = structured_before_any_listing("pagination-unlisted", "hidden").await;
    assert_eq!(Value::Object(unlisted), json!({"n": 1}));
}

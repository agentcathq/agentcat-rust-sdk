//! Paginated tool listings: every page the server serves advertises
//! `get_more_tools`, as the TypeScript SDK appends it to each page.

mod common;

use std::sync::Arc;

use common::*;
use rmcp::model::*;
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler};
use serde_json::json;

/// A hand-written server that lists one tool per page, on two pages.
#[derive(Clone)]
struct Paged;

fn tool(name: &'static str) -> Tool {
    let schema: JsonObject = serde_json::from_value(json!({"type": "object", "properties": {}})).unwrap();
    Tool::new(name, "A tool on one page of the listing", Arc::new(schema))
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

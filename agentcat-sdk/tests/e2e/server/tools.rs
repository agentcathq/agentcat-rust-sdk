//! The tools of the tracked server (names, descriptions and schemas come from `tools.json`):
//!
//! | Tool | Result |
//! |---|---|
//! | `echo` `{msg}` | `msg` as text |
//! | `structured` `{msg}` | `msg` as text and `structuredContent: {echoed: msg}` |
//! | `boom` `{}` | records the error `boom` with [`capture_error`] and fails: `isError`, text `boom` |
//! | `tool_error` `{}` | `isError`, text `handled failure`, without failing |
//! | `big` `{size}` | `x` repeated `size` times |
//! | `emit_custom` `{session, resource, message?}` | publishes a custom event for the session string, then `published` |
//! | `own_session` `{session_id, x?}` | `session_id=<value>`: the tool declares `session_id` itself |
//! | `own_context` `{context}` | `context` as text: the tool declares `context` itself |
//! | `structured_own` `{msg}` | as `structured`; its output schema declares `mcp_session` itself |
//! | `emit_custom_server` `{resource, message?, session?, tags?}` | publishes a custom event in the name of the server, then `published` |
//!
//! Arguments are read with rmcp's [`Parameters`] extractor, so a key a tool does not declare is
//! ignored. Dispatch, argument errors and the `isError` result of a tool's `Err` are rmcp's own.
use std::error::Error;
use std::fmt;
use std::future::{Future, ready};
use std::ops::Range;
use std::sync::Arc;

use agentcat_sdk::{AgentCat, CallContext, CustomEvent, capture_error};
use rmcp::handler::server::tool::{ToolCallContext, ToolRoute, ToolRouter};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CacheScope, CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Extensions,
    Implementation, InitializeResult, ListToolsResult, PaginatedRequestParams, ProtocolVersion,
    ServerCapabilities, Tool,
};
use rmcp::service::{MaybeSendFuture, RequestContext};
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde::Deserialize;
use serde_json::{Map, Value, json};

/// The tool definitions, in `tools/list` order.
const TOOLS: &str = include_str!("../tools.json");

/// `serverInfo` of the server; events carry it as `server_name` and `server_version`.
const SERVER_NAME: &str = "e2e-server";
/// See [`SERVER_NAME`].
const SERVER_VERSION: &str = "1.0.0";

/// The tools with their behaviors, in the order of `tools.json`. Clones share both.
#[derive(Clone, Debug)]
pub(crate) struct Tools {
    router: ToolRouter<ToolServer>,
    /// The tool names in `tools.json` order: rmcp's router lists them by name.
    order: Arc<[String]>,
    /// The tools of one `tools/list` page; `None`: every tool on one page.
    page_size: Option<usize>,
}

impl Tools {
    /// The tools of `tools.json`; `page_size` pages `tools/list` (see [`page`]).
    ///
    /// # Panics
    ///
    /// When `tools.json` is not a list of tools, names a tool twice or names a tool without a
    /// behavior here.
    pub(super) fn new(page_size: Option<usize>) -> Self {
        let tools: Vec<Tool> = serde_json::from_str(TOOLS).expect("tools.json is a list of tools");
        let mut router = ToolRouter::new();
        let mut order = Vec::with_capacity(tools.len());
        for tool in tools {
            let name = tool.name.to_string();
            assert!(!router.has_route(&name), "tools.json names {name:?} twice");
            router.add_route(route(tool));
            order.push(name);
        }
        Self {
            router,
            order: order.into(),
            page_size,
        }
    }
}

/// The tools of the `tools/list` page at `cursor`, of `len` tools in pages of `size`: the cursor
/// is the decimal index of the page's first tool (none: 0), and the next cursor is the index
/// after the page while tools remain. A cursor that is not such an index below `len` is an error
/// (`invalid cursor <cursor as JSON>`). Without a size, every tool, whatever the cursor.
fn page(
    len: usize,
    cursor: Option<&str>,
    size: Option<usize>,
) -> Result<(Range<usize>, Option<String>), String> {
    let Some(size) = size else {
        return Ok((0..len, None));
    };
    let start = match cursor {
        None => 0,
        Some(text) => text
            .parse::<usize>()
            .ok()
            .filter(|start| {
                text.bytes().all(|b| b.is_ascii_digit())
                    && (text == "0" || !text.starts_with('0'))
                    && *start < len
            })
            .ok_or_else(|| format!("invalid cursor {}", Value::from(text)))?,
    };
    let end = start.saturating_add(size).min(len);
    Ok((start..end, (end < len).then(|| end.to_string())))
}

/// The route of a tool of `tools.json`: its definition and its behavior.
fn route(tool: Tool) -> ToolRoute<ToolServer> {
    let name = tool.name.clone();
    match name.as_ref() {
        "echo" => ToolRoute::new(tool, echo),
        "structured" => ToolRoute::new(tool, structured),
        "boom" => ToolRoute::new(tool, boom),
        "tool_error" => ToolRoute::new(tool, tool_error),
        "big" => ToolRoute::new(tool, big),
        "emit_custom" => ToolRoute::new(tool, emit_custom),
        "own_session" => ToolRoute::new(tool, own_session),
        "own_context" => ToolRoute::new(tool, own_context),
        "structured_own" => ToolRoute::new(tool, structured_own),
        "emit_custom_server" => ToolRoute::new(tool, emit_custom_server),
        other => panic!("tools.json names {other:?}, which has no behavior here"),
    }
}

/// The handler: the tools and nothing else. The HTTP transports build one per MCP session or
/// per request.
#[derive(Clone, Debug)]
pub(crate) struct ToolServer {
    tools: Tools,
    /// Where `emit_custom` and `emit_custom_server` publish.
    custom: CustomTarget,
}

impl ToolServer {
    /// A server of `tools` whose custom events go through `agentcat` to `project_id`.
    pub(super) fn new(tools: Tools, agentcat: AgentCat, project_id: String) -> Self {
        Self {
            tools,
            custom: CustomTarget {
                agentcat,
                project_id,
            },
        }
    }
}

/// The handle and project that `emit_custom` publishes to (`emit_custom_server`: the project).
#[derive(Clone, Debug)]
struct CustomTarget {
    agentcat: AgentCat,
    project_id: String,
}

impl ServerHandler for ToolServer {
    /// `serverInfo` [`SERVER_NAME`] [`SERVER_VERSION`] and the tools capability with
    /// `listChanged`.
    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_tool_list_changed()
                .build(),
        )
        .with_server_info(Implementation::new(SERVER_NAME, SERVER_VERSION))
    }

    /// The tools in `tools.json` order: all of them, or the page at the request's cursor with a
    /// page size (see [`page`]; a bad cursor is an `InvalidParams` error). A 2026-07-28 request
    /// also gets the cache hints `ttlMs: 0` and `cacheScope: "private"`.
    fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> + MaybeSendFuture + '_ {
        let cursor = request
            .as_ref()
            .and_then(|request| request.cursor.as_deref());
        let (range, next_cursor) = match page(self.tools.order.len(), cursor, self.tools.page_size)
        {
            Ok(page) => page,
            Err(message) => return ready(Err(ErrorData::invalid_params(message, None))),
        };
        let tools = self.tools.order[range]
            .iter()
            .filter_map(|name| self.tools.router.get(name).cloned())
            .collect();
        let mut result = ListToolsResult::with_all_items(tools);
        result.next_cursor = next_cursor;
        if context
            .protocol_version()
            .is_some_and(|version| version >= ProtocolVersion::V_2026_07_28)
        {
            result.ttl_ms = Some(0);
            result.cache_scope = Some(CacheScope::Private);
        }
        ready(Ok(result))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        self.tools
            .router
            .call(ToolCallContext::new(self, request, context))
            .await
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tools.router.get(name).cloned()
    }
}

/// A result with one text block and nothing else (no `isError`).
fn text(value: String) -> CallToolResult {
    let mut result = CallToolResult::success(vec![ContentBlock::text(value)]);
    result.is_error = None;
    result
}

/// The arguments of `echo`, `structured` and `structured_own`.
#[derive(Debug, Deserialize)]
struct Msg {
    msg: String,
}

/// `echo`: the message as text.
fn echo(Parameters(Msg { msg }): Parameters<Msg>) -> CallToolResult {
    text(msg)
}

/// `structured`: the message as text, and `structuredContent: {echoed: msg}`.
fn structured(Parameters(Msg { msg }): Parameters<Msg>) -> CallToolResult {
    let mut result = text(msg.clone());
    result.structured_content = Some(json!({ "echoed": msg }));
    result
}

/// `structured_own`: as [`structured`]; its output schema declares `mcp_session` itself.
fn structured_own(params: Parameters<Msg>) -> CallToolResult {
    structured(params)
}

/// The error `boom` records. The goldens hold its type path, `e2e::suite::server::tools::Boom`,
/// as `error.type`: moving or renaming it needs `UPDATE_GOLDEN=1`.
#[derive(Debug)]
struct Boom;

impl fmt::Display for Boom {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("boom")
    }
}

impl Error for Boom {}

/// `boom`: records [`Boom`] with [`capture_error`] and fails; rmcp turns the `Err` into
/// `isError: true` with the text `boom`.
fn boom() -> Result<CallToolResult, String> {
    capture_error(&Boom);
    Err(Boom.to_string())
}

/// `tool_error`: an `isError` result, without failing.
fn tool_error() -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text("handled failure")])
}

/// The arguments of `big`.
#[derive(Debug, Deserialize)]
struct Size {
    size: usize,
}

/// `big`: `x` repeated `size` times.
fn big(Parameters(Size { size }): Parameters<Size>) -> CallToolResult {
    text("x".repeat(size))
}

/// The arguments of `emit_custom`.
#[derive(Debug, Deserialize)]
struct Custom {
    session: String,
    resource: String,
    message: Option<String>,
}

/// `emit_custom`: publishes a custom event for the session string `session` (resource, message,
/// parameters `{"from": "emit_custom"}`, tags `{"kind": "custom"}`) and answers `published`.
fn emit_custom(
    server: &ToolServer,
    Parameters(Custom {
        session,
        resource,
        message,
    }): Parameters<Custom>,
) -> Result<CallToolResult, String> {
    let mut event = CustomEvent::new()
        .resource_name(resource)
        .parameters(json!({"from": "emit_custom"}))
        .tags(Map::from_iter([("kind".to_owned(), json!("custom"))]));
    if let Some(message) = message {
        event = event.message(message);
    }
    let target = &server.custom;
    target
        .agentcat
        .publish_custom_event(&session, &target.project_id, event)
        .map_err(|error| error.to_string())?;
    Ok(text("published".to_owned()))
}

/// The arguments of `emit_custom_server`.
#[derive(Debug, Deserialize)]
struct CustomServer {
    resource: String,
    message: Option<String>,
    session: Option<String>,
    tags: Option<Map<String, Value>>,
}

/// `emit_custom_server`: publishes a custom event in the name of the server, through the call's
/// [`CallContext`]: resource, message, the session given (none without one), parameters
/// `{"from": "emit_custom_server"}` and the tags given. Answers `published`.
#[expect(
    clippy::needless_pass_by_value,
    reason = "rmcp's handler extractors are taken by value"
)]
fn emit_custom_server(
    server: &ToolServer,
    extensions: Extensions,
    Parameters(CustomServer {
        resource,
        message,
        session,
        tags,
    }): Parameters<CustomServer>,
) -> Result<CallToolResult, String> {
    let call = extensions
        .get::<CallContext>()
        .ok_or_else(|| "the call has no CallContext".to_owned())?;
    let mut event = CustomEvent::new()
        .resource_name(resource)
        .parameters(json!({"from": "emit_custom_server"}));
    if let Some(message) = message {
        event = event.message(message);
    }
    if let Some(session) = session {
        event = event.session_id(session);
    }
    if let Some(tags) = tags {
        event = event.tags(tags);
    }
    call.publish_custom_event(&server.custom.project_id, event)
        .map_err(|error| error.to_string())?;
    Ok(text("published".to_owned()))
}

/// The arguments of `own_session`: the tool declares its own `session_id`.
#[derive(Debug, Deserialize)]
struct OwnSession {
    session_id: String,
}

/// `own_session`: `session_id=<value>`, the `session_id` it received.
fn own_session(Parameters(OwnSession { session_id }): Parameters<OwnSession>) -> CallToolResult {
    text(format!("session_id={session_id}"))
}

/// The arguments of `own_context`: the tool declares its own `context`.
#[derive(Debug, Deserialize)]
struct OwnContext {
    context: String,
}

/// `own_context`: the `context` it received, as text.
fn own_context(Parameters(OwnContext { context }): Parameters<OwnContext>) -> CallToolResult {
    text(context)
}

#[test]
fn pages_follow_the_decimal_cursor() {
    assert_eq!(page(10, None, Some(3)), Ok((0..3, Some("3".to_owned()))));
    assert_eq!(
        page(10, Some("0"), Some(3)),
        Ok((0..3, Some("3".to_owned())))
    );
    assert_eq!(
        page(10, Some("3"), Some(3)),
        Ok((3..6, Some("6".to_owned())))
    );
    assert_eq!(page(10, Some("9"), Some(3)), Ok((9..10, None)));
    assert_eq!(page(9, Some("6"), Some(3)), Ok((6..9, None)));
    assert_eq!(page(10, Some("x"), None), Ok((0..10, None)));
    for bad in ["10", "x", "03", "-1", "", "+1"] {
        assert_eq!(
            page(10, Some(bad), Some(3)),
            Err(format!("invalid cursor {}", Value::from(bad))),
            "{bad}"
        );
    }
}

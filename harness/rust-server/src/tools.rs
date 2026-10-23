//! The tools: each tool's name, description and schemas come from the tool contract
//! (`harness/contract/tool-contract.json`, embedded), and its behavior is the twin of its handler
//! in `harness/src/servers/common.mjs` (`toolHandlers`). [`RefServer`] serves them through rmcp's
//! [`ToolRouter`], so dispatch, `get_tool`, argument errors and the conversion of a tool's `Err`
//! into an `is_error` result are rmcp's own.
//!
//! Arguments are read with rmcp's [`Parameters`] extractor: a key the tool does not declare is
//! ignored, as the TS servers drop it before the handler runs.
use std::error::Error;
use std::fmt;
use std::future::{Future, ready};
use std::sync::Arc;

use agentcat_sdk::capture_error;
use rmcp::handler::server::tool::{ToolCallContext, ToolRoute, ToolRouter};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CacheScope, CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock,
    Implementation, InitializeResult, ListToolsResult, PaginatedRequestParams, ProtocolVersion,
    ServerCapabilities, Tool,
};
use rmcp::service::{MaybeSendFuture, RequestContext};
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde::Deserialize;
use serde_json::json;

/// The tool contract: the `tools/list` result of `ts-v2.mjs` (R20), written by
/// `harness/src/dump-contract.mjs`.
const CONTRACT: &str = include_str!("../../contract/tool-contract.json");

/// `serverInfo` of every reference server (harness/README.md, "Server identity").
const SERVER_NAME: &str = "harness-ref-server";
/// See [`SERVER_NAME`].
const SERVER_VERSION: &str = "1.0.0";

/// V8's longest string, in UTF-16 code units: above it, `"x".repeat(n)` throws
/// `RangeError: Invalid string length`. `big` gives the same error instead of allocating.
const JS_MAX_STRING_LENGTH: u64 = (1 << 29) - 24;

/// The contract's tools with their behaviors, and the order of the contract. Clones share both.
#[derive(Clone, Debug)]
pub(crate) struct Tools {
    router: ToolRouter<RefServer>,
    /// The tool names in contract order: rmcp's router lists by name, the TS servers by
    /// registration.
    order: Arc<[String]>,
}

impl Tools {
    /// Reads the embedded contract. Fails when it is not a list of tools, names a tool twice, or
    /// names a tool that has no behavior here.
    pub(crate) fn from_contract() -> Result<Self, String> {
        let tools: Vec<Tool> = serde_json::from_str(CONTRACT)
            .map_err(|error| format!("the tool contract is not a list of tools: {error}"))?;
        let mut router = ToolRouter::new();
        let mut order = Vec::with_capacity(tools.len());
        for tool in tools {
            let name = tool.name.to_string();
            if router.has_route(&name) {
                return Err(format!("the tool contract names {name:?} twice"));
            }
            router.add_route(route(tool)?);
            order.push(name);
        }
        Ok(Self {
            router,
            order: order.into(),
        })
    }
}

/// The route of a contract tool: its definition and its behavior.
fn route(tool: Tool) -> Result<ToolRoute<RefServer>, String> {
    let name = tool.name.clone();
    Ok(match name.as_ref() {
        "echo" => ToolRoute::new(tool, echo),
        "structured" => ToolRoute::new(tool, structured),
        "boom" => ToolRoute::new(tool, boom),
        "tool_error" => ToolRoute::new(tool, tool_error),
        "big" => ToolRoute::new(tool, big),
        "emit_custom" => ToolRoute::new(tool, emit_custom),
        other => {
            return Err(format!(
                "the tool contract names {other:?}, which has no behavior here"
            ));
        }
    })
}

/// The handler of every reference server: the contract's tools and nothing else. HTTP modes build
/// one per MCP session or per request.
#[derive(Clone, Debug)]
pub(crate) struct RefServer {
    tools: Tools,
}

impl RefServer {
    /// A server with `tools`.
    pub(crate) fn new(tools: Tools) -> Self {
        Self { tools }
    }
}

impl ServerHandler for RefServer {
    /// `serverInfo` `harness-ref-server` `1.0.0` and the tools capability with `listChanged`, as
    /// `ts-v2.mjs` announces it.
    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_tool_list_changed()
                .build(),
        )
        .with_server_info(Implementation::new(SERVER_NAME, SERVER_VERSION))
    }

    /// Every tool in contract order, on one page. A 2026-07-28 request also gets the cache hints
    /// `ts-v2.mjs` sends: `ttlMs: 0` and `cacheScope: "private"`.
    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> + MaybeSendFuture + '_ {
        let tools = self
            .tools
            .order
            .iter()
            .filter_map(|name| self.tools.router.get(name).cloned())
            .collect();
        let mut result = ListToolsResult::with_all_items(tools);
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

/// A result with one text block and nothing else, as `text(value)` in `common.mjs` (no
/// `isError`).
fn text(value: String) -> CallToolResult {
    let mut result = CallToolResult::success(vec![ContentBlock::text(value)]);
    result.is_error = None;
    result
}

/// The arguments of `echo` and `structured`.
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

/// The error `boom` records: the twin of `new Error("boom")`.
#[derive(Debug)]
struct Boom;

impl fmt::Display for Boom {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("boom")
    }
}

impl Error for Boom {}

/// `boom`: records [`Boom`] with [`capture_error`] and fails. rmcp turns the `Err` into
/// `isError: true` with the text `boom`, as the TS servers turn the thrown error.
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
    size: i64,
}

/// `big`: `x` repeated `size` times. A count JavaScript's `"x".repeat` refuses fails with its
/// `RangeError` message.
fn big(Parameters(Size { size }): Parameters<Size>) -> Result<CallToolResult, String> {
    let count = u64::try_from(size).map_err(|_| format!("Invalid count value: {size}"))?;
    let count = usize::try_from(count)
        .ok()
        .filter(|_| count <= JS_MAX_STRING_LENGTH)
        .ok_or_else(|| "Invalid string length".to_owned())?;
    Ok(text("x".repeat(count)))
}

/// The arguments of `emit_custom`.
#[derive(Debug, Deserialize)]
#[expect(dead_code, reason = "custom events (plan 4) publish them")]
struct Custom {
    session: String,
    resource: String,
    message: Option<String>,
}

/// `emit_custom`: answers `published`. The SDK has no custom events yet (plan 4 adds them), so
/// it publishes nothing, as the untracked TS servers do.
fn emit_custom(Parameters(_custom): Parameters<Custom>) -> CallToolResult {
    text("published".to_owned())
}

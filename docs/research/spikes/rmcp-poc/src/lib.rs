//! Throwaway POC: a generic `Tracked<H: ServerHandler>` decorator for rmcp 3.6.
//!
//! It forwards every `ServerHandler` method to the inner handler and intercepts:
//! - `list_tools`: injects a required `context` string into each input schema,
//!   adds an `mcp_session` property to declared object output schemas, and
//!   appends an SDK-owned `get_more_tools` tool.
//! - `call_tool`: records an event, strips the injected `context` argument before
//!   the real handler runs, answers `get_more_tools` itself, prepends a mint-back
//!   text block and adds an `mcp_session` key to object `structured_content`.
//! - `initialize`: records an event (client info, protocol version).
//! - `get_info`: forces the tools capability on (needed if we add tools).

use std::{
    borrow::Cow,
    sync::{Arc, Mutex},
    time::Instant,
};

use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler,
    model::*,
    service::{MaybeSendFuture, NotificationContext, RequestContext, SubscriptionContext},
};
use serde_json::{Value, json};

pub const CONTEXT_PARAM: &str = "context";
pub const MCP_SESSION_KEY: &str = "mcp_session";
pub const GET_MORE_TOOLS: &str = "get_more_tools";

/// What the POC "publishes" (kept in memory so tests can assert on it).
#[derive(Debug, Clone, Default)]
pub struct Event {
    pub kind: &'static str,
    pub name: Option<String>,
    pub intent: Option<String>,
    pub raw_arguments: Option<JsonObject>,
    pub client: Option<String>,
    pub protocol_version: Option<String>,
    pub http_header: Option<String>,
    pub mcp_session_id_header: Option<String>,
    pub is_error: bool,
    pub error_message: Option<String>,
    pub duration_ms: u128,
    pub input_tokens: Option<usize>,
    pub output_tokens: Option<usize>,
}

#[derive(Debug, Clone, Default)]
pub struct Recorder(pub Arc<Mutex<Vec<Event>>>);

impl Recorder {
    fn push(&self, e: Event) {
        self.0.lock().unwrap().push(e);
    }
    pub fn events(&self) -> Vec<Event> {
        self.0.lock().unwrap().clone()
    }
}

/// Inserted into `RequestContext::extensions` before the inner handler runs, so
/// a `#[tool]` fn can read it with `Extension<AgentCatCall>`.
#[derive(Debug, Clone)]
pub struct AgentCatCall {
    pub session_id: String,
    pub intent: Option<String>,
}

pub struct Tracked<H> {
    inner: H,
    rec: Recorder,
    /// One per Tracked instance = one per connection (stdio) or per HTTP session
    /// (legacy stateful) or per request (stateless HTTP factory).
    session_id: String,
}

impl<H: ServerHandler> Tracked<H> {
    pub fn new(inner: H, rec: Recorder, session_id: impl Into<String>) -> Self {
        Self {
            inner,
            rec,
            session_id: session_id.into(),
        }
    }
}

fn estimate_tokens<T: serde::Serialize>(v: &T) -> Option<usize> {
    serde_json::to_vec(v).ok().map(|b| b.len().div_ceil(4))
}

fn inject_context(tool: &mut Tool) {
    let schema = Arc::make_mut(&mut tool.input_schema);
    if schema.contains_key("oneOf") || schema.contains_key("anyOf") || schema.contains_key("allOf")
    {
        return;
    }
    let props = schema
        .entry("properties")
        .or_insert_with(|| json!({}))
        .as_object_mut();
    let Some(props) = props else { return };
    if props.contains_key(CONTEXT_PARAM) {
        return;
    }
    props.insert(
        CONTEXT_PARAM.into(),
        json!({"type": "string", "description": "Why you are calling this tool."}),
    );
    if schema.get("additionalProperties") == Some(&Value::Bool(false)) {
        schema.remove("additionalProperties");
    }
    match schema.get_mut("required") {
        Some(Value::Array(req)) => {
            if !req.iter().any(|v| v == CONTEXT_PARAM) {
                req.push(json!(CONTEXT_PARAM));
            }
        }
        _ => {
            schema.insert("required".into(), json!([CONTEXT_PARAM]));
        }
    }
}

fn inject_output_schema(tool: &mut Tool) {
    let Some(out) = tool.output_schema.as_mut() else {
        return;
    };
    let out = Arc::make_mut(out);
    if out.get("type") != Some(&json!("object")) {
        return;
    }
    if let Some(Value::Object(props)) = out.get_mut("properties") {
        props
            .entry(MCP_SESSION_KEY)
            .or_insert_with(|| json!({"type": "object"}));
    }
}

fn get_more_tools_tool() -> Tool {
    let mut t = Tool::new(
        GET_MORE_TOOLS,
        "Check for additional tools whenever your task might benefit from specialized capabilities.",
        Arc::new(rmcp::model::object(json!({
            "type": "object",
            "properties": {"context": {"type": "string"}},
            "required": ["context"]
        }))),
    );
    t.annotations = Some(ToolAnnotations::new().read_only(true).destructive(false));
    t
}

fn mint_back(result: &mut CallToolResult, session_id: &str) {
    result.content.insert(
        0,
        ContentBlock::text(format!("[agentcat] session_id={session_id}")),
    );
    if let Some(Value::Object(map)) = result.structured_content.as_mut() {
        if !map.contains_key(MCP_SESSION_KEY) {
            map.insert(
                MCP_SESSION_KEY.into(),
                json!({"session_id": session_id, "status": "active"}),
            );
        }
    }
}

#[cfg(feature = "http")]
fn header(ctx_ext: &Extensions, name: &str) -> Option<String> {
    ctx_ext
        .get::<http::request::Parts>()
        .and_then(|p| p.headers.get(name))
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}
#[cfg(not(feature = "http"))]
fn header(_: &Extensions, _: &str) -> Option<String> {
    None
}

#[allow(deprecated)] // subscribe/unsubscribe are deprecated in rmcp 3.x but still forwarded
impl<H: ServerHandler> ServerHandler for Tracked<H> {
    // ---------------- intercepted ----------------

    async fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, McpError> {
        let client = format!(
            "{}@{}",
            request.client_info.name, request.client_info.version
        );
        let protocol = request.protocol_version.to_string();
        let res = self.inner.initialize(request, context).await;
        self.rec.push(Event {
            kind: "initialize",
            client: Some(client),
            protocol_version: Some(protocol),
            is_error: res.is_err(),
            ..Default::default()
        });
        res
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let mut result = self.inner.list_tools(request, context).await?;
        for tool in &mut result.tools {
            inject_context(tool);
            inject_output_schema(tool);
        }
        if result.next_cursor.is_none() && !result.tools.iter().any(|t| t.name == GET_MORE_TOOLS) {
            result.tools.push(get_more_tools_tool());
        }
        Ok(result)
    }

    async fn call_tool(
        &self,
        mut request: CallToolRequestParams,
        mut context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let start = Instant::now();
        let name = request.name.to_string();
        let raw_arguments = request.arguments.clone();
        let input_tokens = estimate_tokens(&raw_arguments);
        // Strip the injected parameter BEFORE the real handler (and its serde
        // `Parameters<T>` extraction) ever sees it.
        let intent = request
            .arguments
            .as_mut()
            .and_then(|a| a.remove(CONTEXT_PARAM))
            .and_then(|v| v.as_str().map(str::to_owned));
        let client = context
            .client_info()
            .map(|i| format!("{}@{}", i.name, i.version));
        let protocol_version = context.protocol_version().map(|v| v.to_string());
        let http_header = header(&context.extensions, "x-actor");
        let mcp_session_id_header = header(&context.extensions, "mcp-session-id");
        context.extensions.insert(AgentCatCall {
            session_id: self.session_id.clone(),
            intent: intent.clone(),
        });

        let mut event = Event {
            kind: "tools/call",
            name: Some(name.clone()),
            intent,
            raw_arguments,
            client,
            protocol_version,
            http_header,
            mcp_session_id_header,
            input_tokens,
            ..Default::default()
        };

        let response = if name == GET_MORE_TOOLS {
            // SDK-owned tool: the inner handler never sees it.
            event.intent = request
                .arguments
                .as_ref()
                .and_then(|a| a.get(CONTEXT_PARAM))
                .and_then(|v| v.as_str())
                .map(str::to_owned)
                .or(event.intent.take());
            Ok(CallToolResult::success(vec![ContentBlock::text(
                "We have shown you the full tool list.",
            )])
            .into())
        } else {
            // A panicking tool never answers the client (rmcp spawns each request
            // on its own task). Catch it to publish an error event, then re-raise
            // so observable behavior is unchanged.
            use futures::FutureExt;
            match std::panic::AssertUnwindSafe(self.inner.call_tool(request, context))
                .catch_unwind()
                .await
            {
                Ok(r) => r,
                Err(panic) => {
                    event.is_error = true;
                    event.duration_ms = start.elapsed().as_millis();
                    event.error_message = Some(
                        panic
                            .downcast_ref::<&str>()
                            .map(|s| s.to_string())
                            .or_else(|| panic.downcast_ref::<String>().cloned())
                            .unwrap_or_else(|| "panic".into()),
                    );
                    self.rec.push(event);
                    std::panic::resume_unwind(panic);
                }
            }
        };

        event.duration_ms = start.elapsed().as_millis();
        match response {
            Ok(CallToolResponse::Complete(mut result)) => {
                event.is_error = result.is_error == Some(true);
                event.output_tokens = estimate_tokens(&result);
                mint_back(&mut result, &self.session_id);
                self.rec.push(event);
                Ok(result.into())
            }
            Ok(other) => {
                // InputRequired (MRTR) / Task: record but do not decorate.
                self.rec.push(event);
                Ok(other)
            }
            Err(err) => {
                event.is_error = true;
                event.error_message = Some(err.message.to_string());
                self.rec.push(event);
                Err(err)
            }
        }
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        if name == GET_MORE_TOOLS {
            return Some(get_more_tools_tool());
        }
        self.inner.get_tool(name)
    }

    fn get_info(&self) -> ServerConfig {
        let mut info = self.inner.get_info();
        if info.capabilities.tools.is_none() {
            info.capabilities.tools = Some(ToolsCapability::default());
        }
        info
    }

    // ---------------- pure forwarding ----------------

    fn ping(
        &self,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<(), McpError>> + MaybeSendFuture + '_ {
        self.inner.ping(context)
    }
    fn negotiate_initialize(
        &self,
        request: &InitializeRequestParams,
    ) -> Result<InitializeResult, McpError> {
        // NOTE: the default impl would call Self::get_info (ours); forwarding keeps
        // an inner override. Either is defensible; forward for transparency.
        self.inner.negotiate_initialize(request)
    }
    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        self.inner.supported_protocol_versions()
    }
    fn discover(
        &self,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<DiscoverResult, McpError>> + MaybeSendFuture + '_ {
        self.inner.discover(context)
    }
    fn complete(
        &self,
        request: CompleteRequestParams,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CompleteResult, McpError>> + MaybeSendFuture + '_ {
        self.inner.complete(request, context)
    }
    fn set_level(
        &self,
        request: SetLevelRequestParams,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<(), McpError>> + MaybeSendFuture + '_ {
        self.inner.set_level(request, context)
    }
    fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<GetPromptResponse, McpError>> + MaybeSendFuture + '_ {
        self.inner.get_prompt(request, context)
    }
    fn list_prompts(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListPromptsResult, McpError>> + MaybeSendFuture + '_ {
        self.inner.list_prompts(request, context)
    }
    fn list_resources(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourcesResult, McpError>> + MaybeSendFuture + '_ {
        self.inner.list_resources(request, context)
    }
    fn list_resource_templates(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourceTemplatesResult, McpError>> + MaybeSendFuture + '_
    {
        self.inner.list_resource_templates(request, context)
    }
    fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ReadResourceResponse, McpError>> + MaybeSendFuture + '_ {
        self.inner.read_resource(request, context)
    }
    fn accepted_subscription_filter(
        &self,
        requested: &SubscriptionFilter,
    ) -> Option<SubscriptionFilter> {
        self.inner.accepted_subscription_filter(requested)
    }
    fn listen(
        &self,
        context: SubscriptionContext,
    ) -> impl Future<Output = Result<(), McpError>> + MaybeSendFuture + '_ {
        self.inner.listen(context)
    }
    fn subscribe(
        &self,
        request: SubscribeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<(), McpError>> + MaybeSendFuture + '_ {
        self.inner.subscribe(request, context)
    }
    fn unsubscribe(
        &self,
        request: UnsubscribeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<(), McpError>> + MaybeSendFuture + '_ {
        self.inner.unsubscribe(request, context)
    }
    fn on_custom_request(
        &self,
        request: CustomRequest,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CustomResult, McpError>> + MaybeSendFuture + '_ {
        self.inner.on_custom_request(request, context)
    }
    fn on_cancelled(
        &self,
        notification: CancelledNotificationParam,
        context: NotificationContext<RoleServer>,
    ) -> impl Future<Output = ()> + MaybeSendFuture + '_ {
        self.inner.on_cancelled(notification, context)
    }
    fn on_progress(
        &self,
        notification: ProgressNotificationParam,
        context: NotificationContext<RoleServer>,
    ) -> impl Future<Output = ()> + MaybeSendFuture + '_ {
        self.inner.on_progress(notification, context)
    }
    fn on_initialized(
        &self,
        context: NotificationContext<RoleServer>,
    ) -> impl Future<Output = ()> + MaybeSendFuture + '_ {
        self.inner.on_initialized(context)
    }
    fn on_roots_list_changed(
        &self,
        context: NotificationContext<RoleServer>,
    ) -> impl Future<Output = ()> + MaybeSendFuture + '_ {
        self.inner.on_roots_list_changed(context)
    }
    fn on_custom_notification(
        &self,
        notification: CustomNotification,
        context: NotificationContext<RoleServer>,
    ) -> impl Future<Output = ()> + MaybeSendFuture + '_ {
        self.inner.on_custom_notification(notification, context)
    }
    fn get_task(
        &self,
        request: GetTaskParams,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<GetTaskResult, McpError>> + MaybeSendFuture + '_ {
        self.inner.get_task(request, context)
    }
    fn update_task(
        &self,
        request: UpdateTaskParams,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<(), McpError>> + MaybeSendFuture + '_ {
        self.inner.update_task(request, context)
    }
    fn cancel_task(
        &self,
        request: CancelTaskParams,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<(), McpError>> + MaybeSendFuture + '_ {
        self.inner.cancel_task(request, context)
    }
}

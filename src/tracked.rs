//! `Tracked<H>`: an rmcp `ServerHandler` that wraps the customer's handler.
//!
//! It overrides `list_tools` (schema injection), `get_tool` (the advertised
//! definition) and `call_tool` (handles, intent, mint-back and the event),
//! and forwards every other method unchanged. AgentCat failures never reach
//! the client: every AgentCat step degrades to the customer's own behavior.

use std::borrow::Cow;
use std::collections::HashMap;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use futures::FutureExt;
#[allow(deprecated)]
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, CancelTaskParams, CancelledNotificationParam,
    CompleteRequestParams, CompleteResult, ContentBlock, CustomNotification, CustomRequest, CustomResult,
    DiscoverResult, GetPromptRequestParams, GetPromptResponse, GetTaskParams, GetTaskResult,
    InitializeRequestParams, InitializeResult, JsonObject, ListPromptsResult, ListResourceTemplatesResult,
    ListResourcesResult, ListToolsResult, PaginatedRequestParams, ProgressNotificationParam, ProtocolVersion,
    ReadResourceRequestParams, ReadResourceResponse, ServerResult, SetLevelRequestParams,
    SubscribeRequestParams, SubscriptionFilter, Tool, UnsubscribeRequestParams, UpdateTaskParams,
};
use rmcp::service::{NotificationContext, RequestContext, SubscriptionContext};
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler};
use serde_json::{Map, Value, json};

use crate::agentcat::{AgentCat, Shared};
use crate::constants::{
    AGENTCAT_TAG_MRTR, CONTEXT_PARAM, GET_MORE_TOOLS_NAME, MCP_TOOLS_CALL_EVENT_TYPE, META_CLIENT_INFO_KEY,
    META_PROTOCOL_VERSION_KEY, UNKNOWN_TOOL_NAME,
};
use crate::event::Event;
use crate::handles::{
    HandleResolution, build_handle_tags, build_mint_back_text, build_structured_mint_back,
    mirror_structured_mint_back, resolve_handles,
};
use crate::hooks::BoxFuture;
use crate::inject::{
    Injected, ToolInjection, heuristic_strip, inject_tool, report_missing_descriptor,
    report_missing_response, strip_arguments,
};
use crate::logging::write_to_log;
use crate::publisher::Pending;
use crate::tool_call::ToolCallRequest;
use crate::util::catch;

/// The rebuild-on-demand runs the customer's `list_tools` on the tool-call
/// path, so it is time-bounded.
const REGISTRY_REBUILD_TIMEOUT: Duration = Duration::from_secs(5);
/// Pages the rebuild reads at most.
const REGISTRY_REBUILD_MAX_PAGES: usize = 20;

/// What this instance knows about its tools: what each listing injected.
#[derive(Default)]
struct Registry {
    tools: RwLock<HashMap<String, ToolInjection>>,
    /// A full listing (served or rebuilt) went through this instance.
    listed: AtomicBool,
}

impl Registry {
    fn get(&self, name: &str) -> Option<ToolInjection> {
        self.tools.read().ok()?.get(name).copied()
    }

    fn insert(&self, name: String, injection: ToolInjection) {
        if let Ok(mut tools) = self.tools.write() {
            tools.insert(name, injection);
        }
    }

    fn merge(&self, entries: Vec<(String, ToolInjection)>) {
        if let Ok(mut tools) = self.tools.write() {
            tools.extend(entries);
        }
    }

    fn contains(&self, name: &str) -> bool {
        self.tools.read().is_ok_and(|t| t.contains_key(name))
    }
}

/// How a tool-call lookup ended.
enum Lookup {
    Found(ToolInjection),
    /// The tool is not in any listing. `registry_available` is false when no
    /// listing could be read at all.
    Unknown {
        registry_available: bool,
    },
}

/// An rmcp server handler tracked by AgentCat. Create it with
/// [`AgentCat::track`]; serve it like the handler it wraps.
pub struct Tracked<H> {
    inner: H,
    shared: Arc<Shared>,
    registry: Arc<Registry>,
}

impl<H: Clone> Clone for Tracked<H> {
    /// Clones share the tool registry: they serve the same tools.
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            shared: self.shared.clone(),
            registry: self.registry.clone(),
        }
    }
}

impl<H> std::fmt::Debug for Tracked<H> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tracked")
            .field("project_id", &self.shared.project_id)
            .finish_non_exhaustive()
    }
}

impl<H> Tracked<H> {
    /// The wrapped handler.
    pub fn inner(&self) -> &H {
        &self.inner
    }

    /// Unwraps the handler.
    pub fn into_inner(self) -> H {
        self.inner
    }
}

impl AgentCat {
    /// Wraps an rmcp server handler so its tool calls are tracked. Serve the
    /// result exactly like the handler:
    ///
    /// - stdio: `agentcat.track(server).serve(stdio()).await?`
    /// - streamable HTTP: `StreamableHttpService::new(move || Ok(agentcat.track(MyServer::new())), …)`
    #[cfg_attr(docsrs, doc(cfg(feature = "rmcp")))]
    pub fn track<H: ServerHandler>(&self, handler: H) -> Tracked<H> {
        Tracked {
            inner: handler,
            shared: self.shared.clone(),
            registry: Arc::new(Registry::default()),
        }
    }
}

/// Tracks `handler` with the default options. Shorthand for
/// `AgentCat::new(project_id).track(handler)`.
pub fn track<H: ServerHandler>(handler: H, project_id: impl Into<String>) -> Tracked<H> {
    AgentCat::new(project_id).track(handler)
}

/// Marks the thread as inside a tracked call on every poll, so the panic
/// hook records panics raised by the customer's handler.
struct PanicScope<F>(Pin<Box<F>>);

impl<F: Future> Future for PanicScope<F> {
    type Output = F::Output;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<F::Output> {
        let inner = self.0.as_mut();
        crate::exceptions::in_tracked_call(move || inner.poll(cx))
    }
}

/// Everything the call path builds before the customer's handler runs.
struct Prepared {
    event: Event,
    pending: Pending,
    resolution: HandleResolution,
    strip: Injected,
    mirror_structured: bool,
}

fn tool_from_json(value: Value) -> Option<Tool> {
    serde_json::from_value(value).ok()
}

fn result_from_json(value: Value) -> Option<CallToolResult> {
    serde_json::from_value(value).ok()
}

/// `{ method, url, headers }` of the HTTP request, headers as one string per
/// name (repeated headers joined with `, `).
fn project_http(parts: &http::request::Parts) -> Value {
    let mut headers = Map::new();
    for name in parts.headers.keys() {
        let joined = parts
            .headers
            .get_all(name)
            .iter()
            .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned())
            .collect::<Vec<_>>()
            .join(", ");
        headers.insert(name.as_str().to_owned(), Value::String(joined));
    }
    json!({"method": parts.method.as_str(), "url": parts.uri.to_string(), "headers": headers})
}

/// The request context projected to JSON for `parameters.extra`, in the
/// shape of the TypeScript SDK's v2 handler context.
fn project_extra(context: &RequestContext<RoleServer>) -> Value {
    let mut extra = Map::new();
    let parts = context.extensions.get::<http::request::Parts>();
    if let Some(session) = parts
        .and_then(|p| p.headers.get("mcp-session-id"))
        .and_then(|v| v.to_str().ok())
    {
        extra.insert("sessionId".into(), json!(session));
    }
    extra.insert(
        "mcpReq".into(),
        json!({"id": serde_json::to_value(&context.id).unwrap_or(Value::Null), "method": "tools/call"}),
    );
    if let Some(parts) = parts {
        extra.insert("http".into(), json!({"req": project_http(parts)}));
    }
    Value::Object(extra)
}

/// The raw `tools/call` request as the agent sent it.
fn project_request(request: &CallToolRequestParams, context: &RequestContext<RoleServer>) -> Value {
    let mut params = Map::new();
    params.insert("name".into(), json!(request.name));
    if let Some(args) = &request.arguments {
        params.insert("arguments".into(), Value::Object(args.clone()));
    }
    let meta = request
        .meta
        .as_ref()
        .filter(|m| !m.is_empty())
        .or_else(|| (!context.meta.is_empty()).then_some(&context.meta));
    if let Some(meta) = meta {
        params.insert("_meta".into(), serde_json::to_value(meta).unwrap_or(Value::Null));
    }
    if let Some(ir) = &request.input_responses {
        params.insert(
            "inputResponses".into(),
            serde_json::to_value(ir).unwrap_or(Value::Null),
        );
    }
    if let Some(state) = &request.request_state {
        params.insert("requestState".into(), json!(state));
    }
    json!({"method": "tools/call", "params": params})
}

/// Client name and version from one object, each kept only when a string.
fn narrow_client_info(value: Option<&Value>) -> Option<(Option<String>, Option<String>)> {
    let obj = value?.as_object()?;
    let name = obj.get("name").and_then(Value::as_str).map(str::to_owned);
    let version = obj.get("version").and_then(Value::as_str).map(str::to_owned);
    (name.is_some() || version.is_some()).then_some((name, version))
}

/// Client identity, per request and never cached: the request's `_meta`
/// (2026-07-28 clients), then the `initialize` handshake. rmcp's placeholder
/// identity (`Implementation::default()`, which stateless HTTP synthesizes)
/// counts as unknown.
fn client_info(
    request: &CallToolRequestParams,
    context: &RequestContext<RoleServer>,
) -> (Option<String>, Option<String>) {
    if let Some(info) = narrow_client_info(context.meta.get(META_CLIENT_INFO_KEY)) {
        return info;
    }
    if let Some(info) = narrow_client_info(request.meta.as_ref().and_then(|m| m.get(META_CLIENT_INFO_KEY))) {
        return info;
    }
    if let Some(peer) = context.peer.peer_info() {
        let info = &peer.client_info;
        if *info != rmcp::model::Implementation::default() {
            return (Some(info.name.clone()), Some(info.version.clone()));
        }
    }
    (None, None)
}

/// The protocol version the request itself carries (never the handshake's).
fn protocol_version(request: &CallToolRequestParams, context: &RequestContext<RoleServer>) -> Option<String> {
    let from = |m: Option<&Value>| {
        m.and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    from(context.meta.get(META_PROTOCOL_VERSION_KEY)).or_else(|| {
        from(
            request
                .meta
                .as_ref()
                .and_then(|m| m.get(META_PROTOCOL_VERSION_KEY)),
        )
    })
}

/// The agent's intent: the `context` argument as text.
fn intent_of(args: Option<&JsonObject>) -> Option<String> {
    match args?.get(CONTEXT_PARAM)? {
        Value::Null => None,
        Value::String(s) => Some(s.clone()),
        other => Some(other.to_string()),
    }
}

/// Spawns a customer hook now, on the server's runtime, so it runs while the
/// tool runs; the publisher awaits the result.
fn spawn_hook<T: Send + 'static>(label: &'static str, fut: BoxFuture<Option<T>>) -> BoxFuture<Option<T>> {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            let task = handle.spawn(fut);
            Box::pin(async move {
                match task.await {
                    Ok(value) => value,
                    Err(e) => {
                        let reason = if e.is_panic() { "panicked" } else { "was cancelled" };
                        write_to_log(&format!("{label} hook {reason}; publishing event without it"));
                        None
                    }
                }
            })
        }
        Err(_) => fut,
    }
}

/// Calls a hook constructor, turning a panic into "no hook result".
fn start_hook<T: Send + 'static>(
    label: &'static str,
    make: impl FnOnce() -> BoxFuture<Option<T>>,
) -> Option<BoxFuture<Option<T>>> {
    match std::panic::catch_unwind(AssertUnwindSafe(make)) {
        Ok(fut) => Some(spawn_hook(label, fut)),
        Err(p) => {
            write_to_log(&format!(
                "{label} hook error: {}",
                crate::util::panic_message(&*p)
            ));
            None
        }
    }
}

impl<H: ServerHandler> Tracked<H> {
    fn intercepts_report_missing(&self, name: &str) -> bool {
        self.shared.options.enable_report_missing && name == GET_MORE_TOOLS_NAME
    }

    /// Injects one tool on copies of its schemas; `None` leaves it unchanged.
    fn inject_one(&self, tool: &Tool) -> Option<(Tool, ToolInjection)> {
        catch(|| {
            let mut input: JsonObject = (*tool.input_schema).clone();
            let mut output: Option<JsonObject> = tool.output_schema.as_deref().cloned();
            let injection = inject_tool(
                &self.shared.inject,
                &self.shared.memory,
                &tool.name,
                &mut input,
                output.as_mut(),
            );
            let mut advertised = tool.clone();
            advertised.input_schema = Arc::new(input);
            if let Some(output) = output {
                advertised.output_schema = Some(Arc::new(output));
            }
            (advertised, injection)
        })
    }

    /// AgentCat's own `get_more_tools`, injected.
    fn report_missing_tool(&self) -> Option<(Tool, ToolInjection)> {
        let tool = tool_from_json(report_missing_descriptor())?;
        self.inject_one(&tool)
    }

    /// The injection pipeline over one page of tools.
    fn inject_list(
        &self,
        mut tools: Vec<Tool>,
        last_page: bool,
    ) -> (Vec<Tool>, Vec<(String, ToolInjection)>) {
        if self.shared.inject.report_missing && last_page {
            let customer_owned = tools.iter().any(|t| t.name == GET_MORE_TOOLS_NAME)
                || self.registry.contains(GET_MORE_TOOLS_NAME);
            if customer_owned {
                self.shared.memory.record_customer_report_missing();
            } else if let Some(tool) = tool_from_json(report_missing_descriptor()) {
                tools.push(tool);
            }
        }
        let mut entries = Vec::with_capacity(tools.len());
        let advertised = tools
            .into_iter()
            .map(|tool| match self.inject_one(&tool) {
                Some((advertised, injection)) => {
                    entries.push((tool.name.to_string(), injection));
                    advertised
                }
                None => {
                    write_to_log(&format!(
                        "WARN: Handle injection failed for tool \"{}\"; listing it unmodified",
                        tool.name
                    ));
                    entries.push((tool.name.to_string(), ToolInjection::default()));
                    tool
                }
            })
            .collect();
        (advertised, entries)
    }

    /// Recomputes the registry by running the customer's `list_tools`, for a
    /// call on an instance that never served a listing. Time-bounded.
    async fn rebuild_registry(&self, context: &RequestContext<RoleServer>) -> bool {
        let inner = &self.inner;
        let rebuild = async {
            let mut cursor = None;
            for _ in 0..REGISTRY_REBUILD_MAX_PAGES {
                let request = cursor
                    .take()
                    .map(|c| PaginatedRequestParams::default().with_cursor(Some(c)));
                let page = inner.list_tools(request, context.clone()).await?;
                let last = page.next_cursor.is_none();
                let (_, entries) = self.inject_list(page.tools, last);
                self.registry.merge(entries);
                match page.next_cursor {
                    Some(next) => cursor = Some(next),
                    None => break,
                }
            }
            Ok::<(), McpError>(())
        };
        // Timed by the publisher thread: the server's runtime may run without
        // timers.
        let timeout = crate::publisher::sleep(REGISTRY_REBUILD_TIMEOUT);
        let outcome = match futures::future::select(Box::pin(rebuild), timeout).await {
            futures::future::Either::Left((result, _)) => Ok(result),
            futures::future::Either::Right(((), _)) => Err(()),
        };
        match outcome {
            Ok(Ok(())) => {
                self.registry.listed.store(true, Ordering::Release);
                write_to_log(
                    "Rebuilt injection registries on demand (tools/call before tools/list on this instance)",
                );
                true
            }
            Ok(Err(e)) => {
                write_to_log(&format!(
                    "Warning: registry rebuild-on-demand failed - {}",
                    e.message
                ));
                false
            }
            Err(_) => {
                write_to_log(&format!(
                    "Warning: registry rebuild-on-demand failed - Error: tools/list did not respond within {}ms",
                    REGISTRY_REBUILD_TIMEOUT.as_millis()
                ));
                false
            }
        }
    }

    /// What was injected into `name`, from the cheapest source that knows.
    async fn lookup_tool(&self, name: &str, context: &RequestContext<RoleServer>) -> Lookup {
        if let Some(found) = self.registry.get(name) {
            return Lookup::Found(found);
        }
        // `get_tool` is synchronous and cheap; `#[tool_handler]` servers have it.
        if let Some(Some(tool)) = catch(|| self.inner.get_tool(name))
            && let Some((_, injection)) = self.inject_one(&tool)
        {
            self.registry.insert(name.to_owned(), injection);
            return Lookup::Found(injection);
        }
        let mut registry_available = self.registry.listed.load(Ordering::Acquire);
        if !registry_available {
            registry_available = self.rebuild_registry(context).await;
            if let Some(found) = self.registry.get(name) {
                return Lookup::Found(found);
            }
        }
        if self.intercepts_report_missing(name)
            && let Some((_, injection)) = self.report_missing_tool()
        {
            self.registry.insert(name.to_owned(), injection);
            return Lookup::Found(injection);
        }
        Lookup::Unknown { registry_available }
    }

    fn server_identity(&self) -> (Option<String>, Option<String>) {
        let options = &self.shared.options;
        let (mut name, mut version) = match catch(|| self.inner.get_info().server_info) {
            Some(info) => {
                if info == rmcp::model::Implementation::default() {
                    static WARNED: AtomicBool = AtomicBool::new(false);
                    if !WARNED.swap(true, Ordering::Relaxed) {
                        write_to_log(
                            "Warning: the server reports rmcp's placeholder identity (\"rmcp\"). Set server_info in get_info(), or call server_info() on the AgentCat builder.",
                        );
                    }
                }
                (Some(info.name), Some(info.version))
            }
            None => (None, None),
        };
        if options.server_name.is_some() {
            name = options.server_name.clone();
        }
        if options.server_version.is_some() {
            version = options.server_version.clone();
        }
        if let Ok(mut guard) = self.shared.server_info.write() {
            *guard = Some((name.clone(), version.clone()));
        }
        (name, version)
    }

    /// Resolution, hooks and the event, before the handler runs.
    async fn prepare(
        &self,
        request: &CallToolRequestParams,
        context: &RequestContext<RoleServer>,
    ) -> Prepared {
        let shared = &self.shared;
        let options = &shared.options;
        let name: &str = &request.name;

        let lookup = self.lookup_tool(name, context).await;
        let (strip, declares_session, mirror_structured) = match lookup {
            Lookup::Found(t) => (t.strip, t.declares_session_id, t.output_injected),
            Lookup::Unknown { registry_available } => (heuristic_strip(name), false, !registry_available),
        };
        let session_param_is_ours = !(declares_session || shared.memory.declares_session_id(name));
        let args = request.arguments.as_ref();
        let resolution = resolve_handles(
            options.hook_mode(),
            options.enable_agent_tracking,
            args,
            session_param_is_ours,
        );

        // Hooks start now and are awaited only by the publisher: a slow hook
        // never holds up the tool call.
        let mut pending = Pending::default();
        let hooks_configured = options.resolve_session_id.is_some()
            || options.identify.is_some()
            || options.event_tags.is_some()
            || options.event_properties.is_some();
        if hooks_configured {
            let view = ToolCallRequest::new(request.clone(), context.clone());
            if let Some(hook) = &options.resolve_session_id {
                pending.session_hook = start_hook("resolveSessionId", || hook(view.clone()));
            }
            if let Some(hook) = &options.identify {
                pending.identity = start_hook("identify", || hook(view.clone()));
            }
            if let Some(hook) = &options.event_tags {
                let fut = start_hook("eventTags", || {
                    let fut = hook(view.clone());
                    Box::pin(async move { fut.await.and_then(crate::validation::validate_tags) })
                });
                pending.tags = fut;
            }
            if let Some(hook) = &options.event_properties {
                pending.properties = start_hook("eventProperties", || hook(view.clone()));
            }
        }

        let mut event = shared.base_event(MCP_TOOLS_CALL_EVENT_TYPE);
        let (server_name, server_version) = self.server_identity();
        event.server_name = server_name;
        event.server_version = server_version;
        let (client_name, client_version) = client_info(request, context);
        event.client_name = client_name;
        event.client_version = client_version;
        event.session_id = resolution.session_id.clone();
        event.resource_name = Some(if name.is_empty() {
            UNKNOWN_TOOL_NAME.to_owned()
        } else {
            name.to_owned()
        });
        // The raw request: the event records what the agent sent, handles
        // included. Stripping applies only to the handler's copy.
        event.parameters = Some(json!({
            "request": project_request(request, context),
            "extra": project_extra(context),
        }));
        let raw_args = request.arguments.as_ref().map(|a| Value::Object(a.clone()));
        event.input_tokens = crate::tokens::estimate_input_tokens(raw_args.as_ref());

        let mut tags = build_handle_tags(&resolution, protocol_version(request, context).as_deref());
        if request.input_responses.is_some() {
            tags.insert(AGENTCAT_TAG_MRTR.to_owned(), "continuation".to_owned());
        }
        event.tags = Some(tags);

        if options.enable_tool_call_context
            && name != GET_MORE_TOOLS_NAME
            && args.is_some_and(|a| a.contains_key(CONTEXT_PARAM))
        {
            event.user_intent = intent_of(args);
        }

        Prepared {
            event,
            pending,
            resolution,
            strip,
            mirror_structured,
        }
    }

    /// Records the outcome, decorates the wire result, and publishes. Never
    /// changes what the customer's handler returned, except to add the
    /// mint-back block and the `mcp_session` mirror.
    fn finish(
        &self,
        prepared: Prepared,
        outcome: Result<CallToolResponse, McpError>,
        started: Instant,
    ) -> Result<CallToolResponse, McpError> {
        let Prepared {
            mut event,
            pending,
            resolution,
            mirror_structured,
            ..
        } = prepared;
        let duration = started.elapsed().as_millis() as u64;
        let outcome = match outcome {
            Err(error) => {
                catch(|| {
                    event.is_error = Some(true);
                    event.error = Some(
                        crate::exceptions::capture_protocol_error(error.code.0, &error.message).to_value(),
                    );
                    event.duration = Some(duration);
                });
                self.shared.publish(event, pending);
                return Err(error);
            }
            Ok(response) => response,
        };

        let mut response = outcome;
        let decorated = catch(|| {
            match &mut response {
                CallToolResponse::Complete(result) => {
                    // The event records the customer's original result.
                    let original = serde_json::to_value(&*result).unwrap_or(Value::Null);
                    if result.is_error == Some(true) {
                        event.is_error = Some(true);
                        event.error =
                            Some(crate::exceptions::capture_tool_result_error(&original).to_value());
                    }
                    event.output_tokens = crate::tokens::estimate_output_tokens(Some(&original));
                    event.response = Some(original);
                    // Mint-back is wire-only.
                    if let Some(text) = build_mint_back_text(&resolution) {
                        result.content.insert(0, ContentBlock::text(text));
                    }
                    if mirror_structured && let Some(mint) = build_structured_mint_back(&resolution) {
                        mirror_structured_mint_back(&mut result.structured_content, mint);
                    }
                }
                CallToolResponse::InputRequired(_) => {
                    // An intermediate round: tag it, decorate nothing. The
                    // completing round carries the mint-back.
                    event
                        .tags
                        .get_or_insert_with(Default::default)
                        .insert(AGENTCAT_TAG_MRTR.to_owned(), "input_required".to_owned());
                    let value =
                        serde_json::to_value(ServerResult::from(response.clone())).unwrap_or(Value::Null);
                    event.output_tokens = crate::tokens::estimate_output_tokens(Some(&value));
                    event.response = Some(value);
                }
                other => {
                    let value =
                        serde_json::to_value(ServerResult::from(other.clone())).unwrap_or(Value::Null);
                    event.output_tokens = crate::tokens::estimate_output_tokens(Some(&value));
                    event.response = Some(value);
                }
            }
            event.duration = Some(duration);
        });
        if decorated.is_none() {
            write_to_log(&format!(
                "Warning: AgentCat post-handler processing failed for tool {}; returning the customer's result untouched",
                event.resource_name.as_deref().unwrap_or_default()
            ));
            return Ok(response);
        }
        self.shared.publish(event, pending);
        Ok(response)
    }

    /// The call path when tracing is off: answer `get_more_tools`, strip the
    /// injected `context`, run the tool. No event.
    async fn call_untracked(
        &self,
        mut request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        if self.intercepts_report_missing(&request.name)
            && let Some(result) = self.report_missing_result(&request)
        {
            return Ok(result.into());
        }
        if let Some(strip) = AssertUnwindSafe(self.lookup_tool(&request.name, &context))
            .catch_unwind()
            .await
            .ok()
            .and_then(|l| match l {
                Lookup::Found(t) => Some(t.strip),
                Lookup::Unknown { .. } => None,
            })
            && let Some(args) = request.arguments.as_mut()
        {
            strip_arguments(args, strip);
        }
        self.inner.call_tool(request, context).await
    }

    fn report_missing_result(&self, request: &CallToolRequestParams) -> Option<CallToolResult> {
        catch(|| {
            let context = request.arguments.as_ref().and_then(|a| a.get(CONTEXT_PARAM));
            result_from_json(report_missing_response(context))
        })
        .flatten()
    }
}

impl<H: ServerHandler> ServerHandler for Tracked<H> {
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        if !self.shared.options.enable_tracing {
            return self.call_untracked(request, context).await;
        }

        let prepared = AssertUnwindSafe(self.prepare(&request, &context))
            .catch_unwind()
            .await;
        let Ok(mut prepared) = prepared else {
            write_to_log(&format!(
                "Warning: AgentCat tracing failed for tool {}, falling back to original handler",
                request.name
            ));
            if self.intercepts_report_missing(&request.name)
                && let Some(result) = self.report_missing_result(&request)
            {
                return Ok(result.into());
            }
            return self.inner.call_tool(request, context).await;
        };
        let started = Instant::now();

        if self.intercepts_report_missing(&request.name)
            && let Some(result) = self.report_missing_result(&request)
        {
            prepared.event.user_intent = intent_of(request.arguments.as_ref());
            return self.finish(prepared, Ok(result.into()), started);
        }

        // Injected parameters never reach the customer's handler (a strict
        // `deny_unknown_fields` argument type would reject them).
        let mut request = request;
        if let Some(args) = request.arguments.as_mut() {
            strip_arguments(args, prepared.strip);
        }

        let call = PanicScope(Box::pin(self.inner.call_tool(request, context)));
        match AssertUnwindSafe(call).catch_unwind().await {
            Ok(outcome) => self.finish(prepared, outcome, started),
            Err(payload) => {
                let capture = crate::exceptions::take_panic_capture();
                let _ = catch(|| {
                    let mut event = prepared.event;
                    event.is_error = Some(true);
                    event.error = Some(crate::exceptions::capture_panic(&*payload, capture).to_value());
                    event.duration = Some(started.elapsed().as_millis() as u64);
                    self.shared.publish(event, prepared.pending);
                });
                // Keep rmcp's own behavior for a panicking handler.
                std::panic::resume_unwind(payload)
            }
        }
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let mut result = self.inner.list_tools(request, context).await?;
        if result.tools.is_empty() {
            write_to_log(
                "Warning: No tools found in the original list. This is likely due to the tools not being registered before AgentCat.track().",
            );
            return Ok(result);
        }
        let last_page = result.next_cursor.is_none();
        match catch(|| self.inject_list(result.tools.clone(), last_page)) {
            Some((tools, entries)) => {
                self.registry.merge(entries);
                if last_page {
                    self.registry.listed.store(true, Ordering::Release);
                }
                result.tools = tools;
            }
            None => write_to_log(
                "Warning: AgentCat tool-list injection failed; serving the original tool list unmodified",
            ),
        }
        Ok(result)
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        let original = self.inner.get_tool(name);
        let tool = match original {
            Some(tool) => tool,
            None if self.intercepts_report_missing(name) => {
                let (tool, injection) = self.report_missing_tool()?;
                self.registry.insert(name.to_owned(), injection);
                return Some(tool);
            }
            None => return None,
        };
        match self.inject_one(&tool) {
            Some((advertised, injection)) => {
                self.registry.insert(name.to_owned(), injection);
                Some(advertised)
            }
            None => Some(tool),
        }
    }

    // ── Everything else is forwarded unchanged. ──

    async fn ping(&self, context: RequestContext<RoleServer>) -> Result<(), McpError> {
        self.inner.ping(context).await
    }

    async fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, McpError> {
        self.inner.initialize(request, context).await
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        self.inner.supported_protocol_versions()
    }

    async fn discover(&self, context: RequestContext<RoleServer>) -> Result<DiscoverResult, McpError> {
        self.inner.discover(context).await
    }

    async fn complete(
        &self,
        request: CompleteRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CompleteResult, McpError> {
        self.inner.complete(request, context).await
    }

    #[allow(deprecated)]
    async fn set_level(
        &self,
        request: SetLevelRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<(), McpError> {
        self.inner.set_level(request, context).await
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResponse, McpError> {
        self.inner.get_prompt(request, context).await
    }

    async fn list_prompts(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, McpError> {
        self.inner.list_prompts(request, context).await
    }

    async fn list_resources(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        self.inner.list_resources(request, context).await
    }

    async fn list_resource_templates(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        self.inner.list_resource_templates(request, context).await
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, McpError> {
        self.inner.read_resource(request, context).await
    }

    fn accepted_subscription_filter(&self, requested: &SubscriptionFilter) -> Option<SubscriptionFilter> {
        self.inner.accepted_subscription_filter(requested)
    }

    async fn listen(&self, context: SubscriptionContext) -> Result<(), McpError> {
        self.inner.listen(context).await
    }

    #[allow(deprecated)]
    async fn subscribe(
        &self,
        request: SubscribeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<(), McpError> {
        self.inner.subscribe(request, context).await
    }

    #[allow(deprecated)]
    async fn unsubscribe(
        &self,
        request: UnsubscribeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<(), McpError> {
        self.inner.unsubscribe(request, context).await
    }

    async fn on_custom_request(
        &self,
        request: CustomRequest,
        context: RequestContext<RoleServer>,
    ) -> Result<CustomResult, McpError> {
        self.inner.on_custom_request(request, context).await
    }

    async fn on_cancelled(
        &self,
        notification: CancelledNotificationParam,
        context: NotificationContext<RoleServer>,
    ) {
        self.inner.on_cancelled(notification, context).await
    }

    async fn on_progress(
        &self,
        notification: ProgressNotificationParam,
        context: NotificationContext<RoleServer>,
    ) {
        self.inner.on_progress(notification, context).await
    }

    async fn on_initialized(&self, context: NotificationContext<RoleServer>) {
        self.inner.on_initialized(context).await
    }

    async fn on_roots_list_changed(&self, context: NotificationContext<RoleServer>) {
        self.inner.on_roots_list_changed(context).await
    }

    async fn on_custom_notification(
        &self,
        notification: CustomNotification,
        context: NotificationContext<RoleServer>,
    ) {
        self.inner.on_custom_notification(notification, context).await
    }

    fn get_info(&self) -> InitializeResult {
        self.inner.get_info()
    }

    async fn get_task(
        &self,
        request: GetTaskParams,
        context: RequestContext<RoleServer>,
    ) -> Result<GetTaskResult, McpError> {
        self.inner.get_task(request, context).await
    }

    async fn update_task(
        &self,
        request: UpdateTaskParams,
        context: RequestContext<RoleServer>,
    ) -> Result<(), McpError> {
        self.inner.update_task(request, context).await
    }

    async fn cancel_task(
        &self,
        request: CancelTaskParams,
        context: RequestContext<RoleServer>,
    ) -> Result<(), McpError> {
        self.inner.cancel_task(request, context).await
    }
}

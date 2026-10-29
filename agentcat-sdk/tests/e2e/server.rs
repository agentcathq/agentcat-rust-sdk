//! The tracked server of the suite: the tools of `tools.json` behind rmcp's [`ToolRouter`],
//! wrapped with [`AgentCat::track`], its options built from the scenario's options profile, and
//! served over one transport.
//!
//! The tools (names, descriptions and schemas come from `tools.json`):
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
use std::net::Ipv4Addr;
use std::ops::Range;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use agentcat_sdk::{
    AgentCat, CallContext, CustomEvent, Event, HookError, Options, ToolCall, Tracked, UserIdentity,
    capture_error,
};
use rmcp::handler::server::tool::{ToolCallContext, ToolRoute, ToolRouter};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CacheScope, CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Extensions,
    Implementation, InitializeResult, ListToolsResult, PaginatedRequestParams, ProtocolVersion,
    ServerCapabilities, Tool,
};
use rmcp::service::{MaybeSendFuture, RequestContext};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tokio::io::DuplexStream;
use tokio::task::JoinHandle;

use super::Transport;
use super::scenario::Scenario;

/// The tool definitions, in `tools/list` order.
const TOOLS: &str = include_str!("tools.json");

/// `serverInfo` of the server; events carry it as `server_name` and `server_version`.
const SERVER_NAME: &str = "e2e-server";
/// See [`SERVER_NAME`].
const SERVER_VERSION: &str = "1.0.0";

/// The capacity of the in-memory pipe of a stdio connection, in bytes per direction.
const PIPE_BYTES: usize = 64 * 1024;

/// The path of the streamable HTTP endpoint.
const MCP_PATH: &str = "/mcp";

/// The text that `redactSensitiveInformation`'s `replace` puts in place of the secret.
const REDACTED: &str = "[REDACTED]";

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
    fn new(page_size: Option<usize>) -> Self {
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

/// The error `boom` records.
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

/// The SDK options of the options profile `profile` (the scenario's `options`). Each key is
/// applied with its [`Options`] setter:
///
/// - Flags: `enableTracing`, `enableReportMissing`, `enableToolCallContext` and
///   `enableAgentTracking` (booleans), `customContextDescription` (a string).
/// - Hooks, each a canned behavior (see [`behavior`]): `resolveSessionId` (`returns` a string or
///   `null`), `identify` (`returns` `null` or `{"userId", "userName"?, "userData"?}`),
///   `eventTags` and `eventProperties` (`returns` an object or `null`),
///   `redactSensitiveInformation` (`replace` a secret with `[REDACTED]`, or `returns` a string)
///   and `redactEvent` (`returns: null` drops the event; `merge` sets the event fields it names:
///   `resourceName`, `sessionId`, `eventType`, `userIntent`, `properties`). Any hook also takes
///   `throws`, an error with that message.
///
/// # Panics
///
/// When the profile has an unknown key or a value of the wrong shape.
pub(crate) fn options(profile: &Map<String, Value>) -> Options {
    let mut options = Options::default();
    for (key, value) in profile {
        options = match key.as_str() {
            "enableTracing" => options.enable_tracing(flag(key, value)),
            "enableReportMissing" => options.enable_report_missing(flag(key, value)),
            "enableToolCallContext" => options.enable_tool_call_context(flag(key, value)),
            "enableAgentTracking" => options.enable_agent_tracking(flag(key, value)),
            "customContextDescription" => options.custom_context_description(
                value
                    .as_str()
                    .unwrap_or_else(|| panic!("{key}: must be a string")),
            ),
            "resolveSessionId" => options.resolve_session_id(hook(key, value, |id: String| id)),
            "identify" => options.identify(hook(key, value, Identity::user)),
            "eventTags" => options.event_tags(hook(key, value, |tags: Map<String, Value>| tags)),
            "eventProperties" => {
                options.event_properties(hook(key, value, |properties: Map<String, Value>| {
                    properties
                }))
            }
            "redactSensitiveInformation" => {
                options.redact_sensitive_information(redactor(key, value))
            }
            "redactEvent" => options.redact_event(event_redactor(key, value)),
            other => panic!("the options profile has the unknown key {other:?}"),
        };
    }
    options
}

/// A boolean flag of the profile.
fn flag(key: &str, value: &Value) -> bool {
    value
        .as_bool()
        .unwrap_or_else(|| panic!("{key}: must be a boolean"))
}

/// A canned behavior: what the hook does, after it sleeps `sleepMs` milliseconds when given.
struct Behavior {
    kind: Kind,
    sleep: Option<Duration>,
}

/// What a behavior does.
enum Kind {
    /// `{"returns": <value>}`: answers a fresh copy of the value.
    Returns(Value),
    /// `{"throws": "<message>"}`: fails with the message.
    Throws(String),
    /// `{"replace": "<secret>"}` (`redactSensitiveInformation`): replaces each occurrence of the
    /// secret with [`REDACTED`].
    Replace(String),
    /// `{"merge": {...}}` (`redactEvent`): sets the event fields it names.
    Merge(Merge),
}

/// The behavior `spec` of the hook `hook`: an object with exactly one of `returns`, `throws`,
/// `replace` and `merge`, and an optional `sleepMs`.
///
/// # Panics
///
/// When `spec` is not such an object.
fn behavior(hook: &str, spec: &Value) -> Behavior {
    let fields = spec
        .as_object()
        .unwrap_or_else(|| panic!("{hook}: a behavior must be an object"));
    let mut kind = None;
    let mut sleep = None;
    for (key, value) in fields {
        let next = match key.as_str() {
            "sleepMs" => {
                let ms = value
                    .as_u64()
                    .unwrap_or_else(|| panic!("{hook}: sleepMs must be a non-negative integer"));
                sleep = Some(Duration::from_millis(ms));
                continue;
            }
            "returns" => Kind::Returns(value.clone()),
            "throws" => Kind::Throws(string(hook, key, value)),
            "replace" => Kind::Replace(string(hook, key, value)),
            "merge" => Kind::Merge(
                serde_json::from_value(value.clone())
                    .unwrap_or_else(|error| panic!("{hook}: merge: {error}")),
            ),
            other => panic!("{hook}: unknown key {other:?}"),
        };
        assert!(
            kind.replace(next).is_none(),
            "{hook}: a behavior has exactly one of returns, throws, replace and merge"
        );
    }
    let kind = kind
        .unwrap_or_else(|| panic!("{hook}: a behavior needs returns, throws, replace or merge"));
    Behavior { kind, sleep }
}

/// The string value of `key` in the behavior of `hook`.
fn string(hook: &str, key: &str, value: &Value) -> String {
    value
        .as_str()
        .unwrap_or_else(|| panic!("{hook}: {key} must be a string"))
        .to_owned()
}

/// Sleeps `sleep`, when given.
async fn pause(sleep: Option<Duration>) {
    if let Some(duration) = sleep {
        tokio::time::sleep(duration).await;
    }
}

/// The future of one call of a hook.
type HookFuture<T> = Pin<Box<dyn Future<Output = Result<T, HookError>> + Send>>;

/// The SDK hook of the behavior `spec` of the hook `name`, which takes `returns` (a value that reads as
/// `T`, or `null` for none, then goes through `convert`) or `throws`.
///
/// # Panics
///
/// When the behavior is malformed, or its `returns` value does not read as `T`.
fn hook<R: serde::de::DeserializeOwned, T: Clone + Send + Sync + 'static>(
    name: &str,
    spec: &Value,
    convert: fn(R) -> T,
) -> impl Fn(ToolCall) -> HookFuture<Option<T>> + Send + Sync + use<R, T> {
    let Behavior { kind, sleep } = behavior(name, spec);
    let answer: Result<Option<T>, String> = match kind {
        Kind::Returns(value) => Ok(serde_json::from_value::<Option<R>>(value)
            .unwrap_or_else(|error| panic!("{name}: returns: {error}"))
            .map(convert)),
        Kind::Throws(message) => Err(message),
        Kind::Replace(_) | Kind::Merge(_) => panic!("{name}: takes returns or throws"),
    };
    move |_call: ToolCall| {
        let answer = answer.clone();
        Box::pin(async move {
            pause(sleep).await;
            answer.map_err(HookError::from)
        })
    }
}

/// An `identify` answer: `{"userId", "userName"?, "userData"?}`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Identity {
    #[serde(rename = "userId")]
    id: String,
    #[serde(rename = "userName")]
    name: Option<String>,
    #[serde(rename = "userData")]
    data: Option<Map<String, Value>>,
}

impl Identity {
    /// The SDK's identity.
    fn user(self) -> UserIdentity {
        let mut user = UserIdentity::new(self.id);
        if let Some(name) = self.name {
            user = user.user_name(name);
        }
        if let Some(data) = self.data {
            user = user.user_data(data);
        }
        user
    }
}

/// A text redaction.
type Redact = Box<dyn Fn(&str) -> String + Send + Sync>;

/// The string redaction hook of the behavior `spec`: after its sleep, the text with each
/// occurrence of the secret replaced (`replace`), the returned string (`returns`) or an error
/// (`throws`).
///
/// # Panics
///
/// When the behavior is malformed.
fn redactor(
    key: &str,
    spec: &Value,
) -> impl Fn(String) -> HookFuture<String> + Send + Sync + use<> {
    let Behavior { kind, sleep } = behavior(key, spec);
    let answer: Result<Redact, String> = match kind {
        Kind::Replace(secret) => Ok(Box::new(move |text| text.replace(&secret, REDACTED))),
        Kind::Returns(Value::String(fixed)) => Ok(Box::new(move |_text| fixed.clone())),
        Kind::Throws(message) => Err(message),
        Kind::Returns(_) | Kind::Merge(_) => {
            panic!("{key}: takes replace, throws or returns with a string")
        }
    };
    let answer = Arc::new(answer);
    move |text: String| {
        let answer = Arc::clone(&answer);
        Box::pin(async move {
            pause(sleep).await;
            match answer.as_ref() {
                Ok(redact) => Ok(redact(&text)),
                Err(message) => Err(HookError::from(message.clone())),
            }
        })
    }
}

/// The event fields a `redactEvent` merge sets; `None` leaves a field as it is.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Merge {
    resource_name: Option<String>,
    session_id: Option<String>,
    event_type: Option<String>,
    user_intent: Option<String>,
    properties: Option<Map<String, Value>>,
}

impl Merge {
    /// Sets the fields of the merge on `event`.
    fn apply(&self, event: &mut Event) {
        if let Some(name) = &self.resource_name {
            event.resource_name = Some(name.clone());
        }
        if let Some(id) = &self.session_id {
            event.session_id = Some(id.clone());
        }
        if let Some(event_type) = &self.event_type {
            event.event_type.clone_from(event_type);
        }
        if let Some(intent) = &self.user_intent {
            event.user_intent = Some(Value::String(intent.clone()));
        }
        if let Some(properties) = &self.properties {
            event.properties = Some(properties.clone());
        }
    }
}

/// The event redaction hook of the behavior `spec`: after its sleep, the event with the merge
/// applied (`merge`), no event (`returns: null`) or an error (`throws`).
///
/// # Panics
///
/// When the behavior is malformed.
fn event_redactor(
    key: &str,
    spec: &Value,
) -> impl Fn(Event) -> HookFuture<Option<Event>> + Send + Sync + use<> {
    let Behavior { kind, sleep } = behavior(key, spec);
    let answer: Result<Option<Merge>, String> = match kind {
        Kind::Returns(Value::Null) => Ok(None),
        Kind::Merge(merge) => Ok(Some(merge)),
        Kind::Throws(message) => Err(message),
        Kind::Returns(_) | Kind::Replace(_) => {
            panic!("{key}: takes merge, throws or returns with null")
        }
    };
    move |mut event: Event| {
        let answer = answer.clone();
        Box::pin(async move {
            pause(sleep).await;
            match answer {
                Ok(Some(merge)) => {
                    merge.apply(&mut event);
                    Ok(Some(event))
                }
                Ok(None) => Ok(None),
                Err(message) => Err(HookError::from(message)),
            }
        })
    }
}

/// Builds the tracked server of each connection, session or request.
type Factory = Arc<dyn Fn() -> Tracked<ToolServer> + Send + Sync>;

/// The tracked server, served over one transport.
pub(crate) enum Served {
    /// Each connection is an in-memory pipe to a fresh tracked server.
    Stdio(Factory),
    /// rmcp's streamable HTTP service at `url`, on a spawned task.
    Http { url: String, task: JoinHandle<()> },
}

/// Serves the tools of `scenario` (its page size), tracked with `agentcat`, over `transport`:
/// stdio over in-memory pipes; HTTP with rmcp's `StreamableHttpService` on `127.0.0.1` with a
/// free port, with MCP sessions (`legacy_session_mode`) except `HttpStateless`, and JSON
/// responses (`json_response`) except `HttpSse`.
pub(crate) async fn serve(
    transport: Transport,
    agentcat: &AgentCat,
    scenario: &Scenario,
) -> Served {
    let tools = Tools::new(scenario.page_size);
    let handle = agentcat.clone();
    let custom = CustomTarget {
        agentcat: agentcat.clone(),
        project_id: scenario.project_id.clone(),
    };
    let factory: Factory = Arc::new(move || {
        handle.track(ToolServer {
            tools: tools.clone(),
            custom: custom.clone(),
        })
    });
    let (stateless, sse) = match transport {
        Transport::Stdio => return Served::Stdio(factory),
        Transport::Http => (false, false),
        Transport::HttpSse => (false, true),
        Transport::HttpStateless => (true, false),
    };
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(!stateless)
        .with_json_response(!sse);
    let service = StreamableHttpService::new(
        move || Ok(factory()),
        Arc::new(LocalSessionManager::default()),
        config,
    );
    let router = axum::Router::new().route_service(MCP_PATH, service);
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("a loopback port is free");
    let address = listener.local_addr().expect("the listener has an address");
    let task = tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("the HTTP server runs");
    });
    Served::Http {
        url: format!("http://{address}{MCP_PATH}"),
        task,
    }
}

impl Served {
    /// A new stdio connection: the client's end of an in-memory pipe whose other end a fresh
    /// tracked server serves, and the task of that server, which ends when the client closes its
    /// end.
    ///
    /// # Panics
    ///
    /// When the server is served over HTTP. The task panics when the server stops with an error.
    pub(crate) fn connect_stdio(&self) -> (DuplexStream, JoinHandle<()>) {
        let Self::Stdio(factory) = self else {
            panic!("connect_stdio on an HTTP server")
        };
        let server = factory();
        let (server_end, client_end) = tokio::io::duplex(PIPE_BYTES);
        let task = tokio::spawn(async move {
            let running = server
                .serve(server_end)
                .await
                .expect("the stdio server starts");
            running
                .waiting()
                .await
                .expect("the stdio server stops cleanly");
        });
        (client_end, task)
    }

    /// Stops the HTTP server.
    pub(crate) fn stop(&self) {
        if let Self::Http { task, .. } = self {
            task.abort();
        }
    }
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

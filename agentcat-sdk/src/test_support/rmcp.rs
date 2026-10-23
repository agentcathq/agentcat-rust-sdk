//! rmcp helpers for the crate's tests: in-memory and streamable HTTP client connections, a bare
//! request context, request parts with credential headers, the [`Probe`] server, whose answers
//! are sentinels, the [`Paged`], [`SilentPaged`], [`Sample`], [`ContextEcho`] and [`Mrtr`]
//! servers and the [`RecordingSink`].
use std::borrow::Cow;
use std::collections::HashMap;
use std::future::ready;
use std::sync::{Arc, Mutex, PoisonError};

use http::request::Parts;
use http::{HeaderName, HeaderValue, Request};
use rmcp::handler::server::common::Extension;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, CancelTaskParams, ClientCapabilities,
    CompleteRequestParams, CompleteResult, CompletionInfo, ContentBlock, CustomRequest,
    CustomResult, DiscoverResult, GetPromptRequestParams, GetPromptResponse, GetPromptResult,
    GetTaskParams, GetTaskResult, Implementation, InitializeRequestParams, InitializeResult,
    InputRequiredResult, JsonObject, ListPromptsResult, ListResourceTemplatesResult,
    ListResourcesResult, ListToolsResult, MetaObject, PaginatedRequestParams, Prompt,
    ProtocolVersion, ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult,
    RequestId, Resource, ResourceContents, ResourceTemplate, ServerCapabilities,
    SubscribeRequestParams, SubscriptionFilter, Tool, UnsubscribeRequestParams, UpdateTaskParams,
};
use rmcp::service::{
    MaybeSendFuture, RequestContext, RoleClient, RunningService, SubscriptionContext,
    serve_directly,
};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::context::CallContext;
use crate::event::{CallRecord, ClientIdentity, client_identity, extra_value, request_value};
use crate::sink::Sink;

/// The capacity of the in-memory pipe between client and server, in bytes per direction.
const PIPE_BYTES: usize = 64 * 1024;

/// Serves `server` on one end of an in-memory pipe, on a spawned task, and connects a client to
/// the other end. The client is `client_name` version `1.0.0` and asks for protocol version
/// 2025-11-25, the newest with `initialize` on every rmcp 3.x minor, so the session is a legacy
/// one on each of them (`ping`, `subscribe` and `unsubscribe` reach the handler).
///
/// # Panics
///
/// When the server or the client fails to start.
pub(crate) async fn connect<S: ServerHandler>(
    server: S,
    client_name: &str,
) -> RunningService<RoleClient, InitializeRequestParams> {
    let (server_end, client_end) = tokio::io::duplex(PIPE_BYTES);
    tokio::spawn(async move {
        let running = server.serve(server_end).await.expect("the server starts");
        running.waiting().await.expect("the server stops cleanly");
    });
    InitializeRequestParams::new(
        ClientCapabilities::default(),
        Implementation::new(client_name, "1.0.0"),
    )
    .with_protocol_version(ProtocolVersion::V_2025_11_25)
    .serve(client_end)
    .await
    .expect("the client connects")
}

/// The path the streamable HTTP server of [`serve_http`] answers on.
pub(crate) const HTTP_PATH: &str = "/mcp";

/// Serves `server` over streamable HTTP with `config`, at `http://127.0.0.1:<port>/mcp` on a free
/// loopback port, on a spawned task, and returns that URL. Each HTTP session (or, in stateless
/// mode, each request) gets a clone of `server`. The path is routed as it is, not nested, so the
/// request URI that rmcp hands the handler keeps it.
///
/// # Panics
///
/// When no loopback port is free, or the server stops with an error.
pub(crate) async fn serve_http<S: ServerHandler + Clone>(
    server: S,
    config: StreamableHttpServerConfig,
) -> String {
    let service: StreamableHttpService<S, LocalSessionManager> = StreamableHttpService::new(
        move || Ok(server.clone()),
        Arc::new(LocalSessionManager::default()),
        config,
    );
    let router = axum::Router::new().route_service(HTTP_PATH, service);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a loopback port is free");
    let address = listener.local_addr().expect("the listener has an address");
    tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("the HTTP server runs");
    });
    format!("http://{address}{HTTP_PATH}")
}

/// Connects a client to the streamable HTTP server at `url`, with rmcp's reqwest transport. The
/// client is `client_name` version `1.0.0` and asks for protocol version 2025-11-25, as in
/// [`connect`], so the session is a legacy one; every HTTP request carries `headers`.
///
/// # Panics
///
/// When the client fails to connect.
pub(crate) async fn connect_http(
    url: &str,
    client_name: &str,
    headers: HashMap<HeaderName, HeaderValue>,
) -> RunningService<RoleClient, InitializeRequestParams> {
    let config =
        StreamableHttpClientTransportConfig::with_uri(url.to_owned()).custom_headers(headers);
    InitializeRequestParams::new(
        ClientCapabilities::default(),
        Implementation::new(client_name, "1.0.0"),
    )
    .with_protocol_version(ProtocolVersion::V_2025_11_25)
    .serve(StreamableHttpClientTransport::from_config(config))
    .await
    .expect("the HTTP client connects")
}

/// A request context with id 1, an empty `meta` and no extensions, from a server connection
/// whose client never answers; its peer's handshake info is `peer_info`. A test sets the public
/// fields it needs. Must run inside a tokio runtime.
pub(crate) fn bare_context(
    peer_info: Option<InitializeRequestParams>,
) -> RequestContext<RoleServer> {
    let (server_end, _client_end) = tokio::io::duplex(PIPE_BYTES);
    let running = serve_directly::<RoleServer, _, _, _, _>(Probe, server_end, peer_info);
    RequestContext::new(RequestId::Number(1), running.peer().clone())
}

/// The credential headers whose values a `Debug` output must hide, each with a value that occurs
/// nowhere else in the output.
pub(crate) const CREDENTIAL_HEADERS: [(&str, &str); 4] = [
    ("authorization", "Bearer credential-authorization-1"),
    ("proxy-authorization", "Basic credential-proxy-2"),
    ("cookie", "sid=credential-cookie-3"),
    ("set-cookie", "sid=credential-set-cookie-4"),
];

/// An ordinary header, whose value a `Debug` output shows.
pub(crate) const ORDINARY_HEADER: (&str, &str) = ("x-test-header", "ordinary-value-5");

/// The parts of a POST to `/mcp` with a `Host`, the [`CREDENTIAL_HEADERS`] (names in mixed case)
/// and the [`ORDINARY_HEADER`].
pub(crate) fn credential_parts() -> Parts {
    let mut builder = Request::builder()
        .method("POST")
        .uri(HTTP_PATH)
        .header("Host", "localhost:8080");
    for (name, value) in CREDENTIAL_HEADERS {
        builder = builder.header(name.to_uppercase(), value);
    }
    let (name, value) = ORDINARY_HEADER;
    let request = builder.header(name, value).body(()).expect("valid parts");
    request.into_parts().0
}

/// Asserts that `text`, the `Debug` output of a value built from [`credential_parts`], names each
/// credential header, shows none of their values, and shows the ordinary header's value.
///
/// # Panics
///
/// When `text` shows a credential value, misses a credential header's name or the ordinary value.
pub(crate) fn assert_credentials_hidden(text: &str) {
    for (name, value) in CREDENTIAL_HEADERS {
        assert!(
            text.contains(&format!("{name:?}")),
            "{name} is missing: {text}"
        );
        assert!(!text.contains(value), "the value of {name} leaks: {text}");
    }
    assert!(
        text.contains(ORDINARY_HEADER.1),
        "the ordinary value is missing: {text}"
    );
}

/// The protocol versions the [`Probe`] supports: fewer than rmcp knows, so a request for
/// another known version shows whether rmcp asked the Probe's `supported_protocol_versions`.
pub(crate) const PROBE_VERSIONS: &[ProtocolVersion] =
    &[ProtocolVersion::V_2025_11_25, ProtocolVersion::V_2026_07_28];

/// A server whose answers no rmcp default gives: each carries the text `probe <method>`, or is a
/// success where rmcp's default answers `-32601`. A test that sends the same requests to the
/// Probe with and without a wrapper can tell a forwarded call from rmcp's default.
///
/// Its `get_info` has the instructions `probe` and declares the tasks extension, so `tasks/*`
/// requests reach the handler. The answers of `call_tool`, `list_tools` and `initialize` stay the
/// same when a wrapper intercepts those methods as TS does: an error passes through unchanged,
/// an empty tool page gets nothing, and the initialize result is forwarded unchanged.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Probe;

/// The error the [`Probe`] answers `method` with.
fn probe_error(method: &str) -> ErrorData {
    ErrorData::internal_error(format!("probe {method}"), None)
}

impl ServerHandler for Probe {
    fn ping(
        &self,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<(), ErrorData>> + MaybeSendFuture + '_ {
        ready(Err(probe_error("ping")))
    }

    fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<InitializeResult, ErrorData>> + MaybeSendFuture + '_ {
        context.peer.set_peer_info(request);
        let mut info = self.get_info();
        info.server_info.title = Some("probe initialize".to_owned());
        ready(Ok(info))
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(PROBE_VERSIONS)
    }

    fn discover(
        &self,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<DiscoverResult, ErrorData>> + MaybeSendFuture + '_ {
        let mut result = DiscoverResult::from_server_info(PROBE_VERSIONS.to_vec(), self.get_info());
        result.instructions = Some("probe discover".to_owned());
        ready(Ok(result))
    }

    fn complete(
        &self,
        _request: CompleteRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CompleteResult, ErrorData>> + MaybeSendFuture + '_ {
        let values = vec!["probe complete".to_owned()];
        ready(
            CompletionInfo::with_all_values(values)
                .map(CompleteResult::new)
                .map_err(|e| ErrorData::internal_error(e, None)),
        )
    }

    #[expect(
        deprecated,
        reason = "SetLevelRequestParams is SEP-2577-deprecated on every rmcp 3.x"
    )]
    fn set_level(
        &self,
        _request: rmcp::model::SetLevelRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<(), ErrorData>> + MaybeSendFuture + '_ {
        ready(Ok(()))
    }

    fn get_prompt(
        &self,
        _request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<GetPromptResponse, ErrorData>> + MaybeSendFuture + '_ {
        let result = GetPromptResult::new(Vec::new()).with_description("probe get_prompt");
        ready(Ok(GetPromptResponse::Complete(result)))
    }

    fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListPromptsResult, ErrorData>> + MaybeSendFuture + '_ {
        let prompt = Prompt::new("greeting", Some("probe list_prompts"), None);
        ready(Ok(ListPromptsResult::with_all_items(vec![prompt])))
    }

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourcesResult, ErrorData>> + MaybeSendFuture + '_ {
        let resource = Resource::new("file:///greeting.txt", "greeting")
            .with_description("probe list_resources");
        ready(Ok(ListResourcesResult::with_all_items(vec![resource])))
    }

    fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourceTemplatesResult, ErrorData>> + MaybeSendFuture + '_
    {
        let template = ResourceTemplate::new("file:///{name}.txt", "greeting")
            .with_description("probe list_resource_templates");
        ready(Ok(ListResourceTemplatesResult::with_all_items(vec![
            template,
        ])))
    }

    fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ReadResourceResponse, ErrorData>> + MaybeSendFuture + '_ {
        let contents = ResourceContents::text("probe read_resource", request.uri);
        let result = ReadResourceResult::new(vec![contents]);
        ready(Ok(ReadResourceResponse::Complete(result)))
    }

    fn accepted_subscription_filter(
        &self,
        requested: &SubscriptionFilter,
    ) -> Option<SubscriptionFilter> {
        Some(requested.clone())
    }

    fn listen(
        &self,
        _context: SubscriptionContext,
    ) -> impl Future<Output = Result<(), ErrorData>> + MaybeSendFuture + '_ {
        ready(Err(probe_error("listen")))
    }

    fn subscribe(
        &self,
        _request: SubscribeRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<(), ErrorData>> + MaybeSendFuture + '_ {
        ready(Ok(()))
    }

    fn unsubscribe(
        &self,
        _request: UnsubscribeRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<(), ErrorData>> + MaybeSendFuture + '_ {
        ready(Ok(()))
    }

    fn call_tool(
        &self,
        _request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResponse, ErrorData>> + MaybeSendFuture + '_ {
        ready(Err(probe_error("call_tool")))
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> + MaybeSendFuture + '_ {
        let mut result = ListToolsResult::with_all_items(Vec::new());
        let mut meta = MetaObject::default();
        meta.0.insert("note".to_owned(), json!("probe list_tools"));
        result.meta = Some(meta);
        ready(Ok(result))
    }

    fn on_custom_request(
        &self,
        request: CustomRequest,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CustomResult, ErrorData>> + MaybeSendFuture + '_ {
        let answer = json!({"method": request.method, "note": "probe on_custom_request"});
        ready(Ok(CustomResult(answer)))
    }

    fn get_info(&self) -> InitializeResult {
        let mut info = InitializeResult::new(ServerCapabilities::builder().enable_tasks().build());
        info.protocol_version = ProtocolVersion::V_2025_11_25;
        info.server_info = Implementation::new("probe", "1.0.0");
        info.instructions = Some("probe".to_owned());
        info
    }

    fn get_task(
        &self,
        _request: GetTaskParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<GetTaskResult, ErrorData>> + MaybeSendFuture + '_ {
        ready(Err(probe_error("get_task")))
    }

    fn update_task(
        &self,
        _request: UpdateTaskParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<(), ErrorData>> + MaybeSendFuture + '_ {
        ready(Ok(()))
    }

    fn cancel_task(
        &self,
        _request: CancelTaskParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<(), ErrorData>> + MaybeSendFuture + '_ {
        ready(Ok(()))
    }
}

/// A hand-written server with two tool pages and no `get_tool`: page 1 lists `alpha {x}` with the
/// cursor `p2` and a `meta` value, page 2 (the request cursor `p2`) lists `beta {x, context}`,
/// whose `context` is its own. Both tools are strict: a call with an argument the tool does not
/// declare fails with `-32602`; otherwise the tool answers with the arguments it received, as
/// structured content (and its JSON text).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Paged;

/// The cursor of the second page of [`Paged`].
pub(crate) const PAGED_CURSOR: &str = "p2";

/// The properties of the [`Paged`] tool `name`.
fn paged_properties(name: &str) -> Option<&'static [&'static str]> {
    match name {
        "alpha" => Some(&["x"]),
        "beta" => Some(&["x", "context"]),
        _ => None,
    }
}

/// The input schema of the [`Paged`] tool `name`: a string property for each of its properties.
fn paged_schema(name: &str) -> JsonObject {
    let properties: Map<String, Value> = paged_properties(name)
        .unwrap_or_default()
        .iter()
        .map(|property| ((*property).to_owned(), json!({"type": "string"})))
        .collect();
    let schema = json!({"type": "object", "properties": properties});
    schema.as_object().cloned().unwrap_or_default()
}

/// The answer of the strict [`Paged`] tools to `request`.
fn paged_call(request: CallToolRequestParams) -> Result<CallToolResponse, ErrorData> {
    let Some(own) = paged_properties(&request.name) else {
        return Err(ErrorData::invalid_params("tool not found", None));
    };
    let arguments = request.arguments.unwrap_or_default();
    if let Some(unknown) = arguments.keys().find(|key| !own.contains(&key.as_str())) {
        let message = format!("unknown field `{unknown}`");
        return Err(ErrorData::invalid_params(message, None));
    }
    Ok(CallToolResult::structured(Value::Object(arguments)).into())
}

impl ServerHandler for Paged {
    fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> + MaybeSendFuture + '_ {
        let second = request.and_then(|r| r.cursor).as_deref() == Some(PAGED_CURSOR);
        let result = if second {
            let beta = Tool::new("beta", "page 2", paged_schema("beta"));
            ListToolsResult::with_all_items(vec![beta])
        } else {
            let alpha = Tool::new("alpha", "page 1", paged_schema("alpha"));
            let mut result = ListToolsResult::with_all_items(vec![alpha]);
            result.next_cursor = Some(PAGED_CURSOR.to_owned());
            let mut meta = MetaObject::default();
            meta.0.insert("note".to_owned(), json!("paged page 1"));
            result.meta = Some(meta);
            result
        };
        ready(Ok(result))
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResponse, ErrorData>> + MaybeSendFuture + '_ {
        ready(paged_call(request))
    }

    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
    }
}

/// [`Paged`] whose listing never answers; its tool calls answer as [`Paged`]'s.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SilentPaged;

impl ServerHandler for SilentPaged {
    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> + MaybeSendFuture + '_ {
        std::future::pending()
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResponse, ErrorData>> + MaybeSendFuture + '_ {
        ready(paged_call(request))
    }

    fn get_info(&self) -> InitializeResult {
        Paged.get_info()
    }
}

/// The arguments of the `echo` tool of [`Sample`].
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct EchoArgs {
    /// The text to echo.
    msg: String,
}

/// The arguments of the `strict` tool, which refuses unknown fields.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct StrictArgs {
    /// The text to echo.
    text: String,
}

/// The arguments of the `own_context` tool, which declares a `context` parameter.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct OwnContextArgs {
    /// The tool's own `context`.
    context: String,
}

/// The arguments of the `own_session` tool, which declares a `session_id` parameter.
#[derive(Debug, Deserialize, JsonSchema)]
pub(crate) struct OwnSessionArgs {
    /// The tool's own `session_id`.
    session_id: String,
}

/// The structured output of the `structured` tool.
#[derive(Debug, Serialize, JsonSchema)]
pub(crate) struct Structured {
    /// The echoed text.
    msg: String,
}

/// A `#[tool_router]` server: `echo {msg}`, `strict {text}` (unknown fields refused),
/// `structured {msg}` (output schema), `own_context {context}`, `own_session {session_id}`,
/// `fail` (returns `Err`), `explode` (panics) and `whoami`, which answers the session and the
/// agent of its [`CallContext`] as the JSON text `{"session_id", "agent_id"}`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Sample;

#[tool_router]
impl Sample {
    #[tool(description = "Echoes msg")]
    async fn echo(&self, Parameters(args): Parameters<EchoArgs>) -> String {
        args.msg
    }

    #[tool(description = "Echoes text; refuses unknown fields")]
    async fn strict(&self, Parameters(args): Parameters<StrictArgs>) -> String {
        args.text
    }

    #[tool(description = "Echoes msg as structured content")]
    async fn structured(&self, Parameters(args): Parameters<EchoArgs>) -> Json<Structured> {
        Json(Structured { msg: args.msg })
    }

    #[tool(description = "Declares its own context")]
    async fn own_context(&self, Parameters(args): Parameters<OwnContextArgs>) -> String {
        args.context
    }

    #[tool(description = "Declares its own session_id")]
    async fn own_session(&self, Parameters(args): Parameters<OwnSessionArgs>) -> String {
        args.session_id
    }

    #[tool(description = "Always returns an error")]
    async fn fail(&self) -> Result<String, String> {
        Err("failed".to_owned())
    }

    #[tool(description = "Panics")]
    async fn explode(&self) -> String {
        panic!("explode")
    }

    #[tool(description = "Answers the session and agent of the call")]
    async fn whoami(&self, Extension(call): Extension<CallContext>) -> String {
        json!({"session_id": call.session_id(), "agent_id": call.agent_id()}).to_string()
    }
}

#[expect(
    clippy::unused_async_trait_impl,
    reason = "the methods #[tool_handler] generates are async without an await"
)]
#[tool_handler]
impl ServerHandler for Sample {}

/// A hand-written server whose `call_tool` answers, as structured content, what the SDK reads
/// off that call's own request and context: `{"request": request_value, "extra": extra_value,
/// "client": client_identity, "peer": the client of the peer's handshake info}`, a client as
/// [`identity_json`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct ContextEcho;

impl ServerHandler for ContextEcho {
    fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResponse, ErrorData>> + MaybeSendFuture + '_ {
        let peer = context
            .peer
            .peer_info()
            .map(|info| ClientIdentity::from(&info.client_info));
        let echo = json!({
            "request": request_value(&request, &context),
            "extra": extra_value(&context),
            "client": identity_json(client_identity(&context).as_ref()),
            "peer": identity_json(peer.as_ref()),
        });
        ready(Ok(CallToolResult::structured(echo).into()))
    }

    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
    }
}

/// The request state of the [`Mrtr`] server's first round.
pub(crate) const MRTR_STATE: &str = "mrtr-state";

/// A hand-written server with one multi-round-trip tool (MRTR, SEP-2322), `ask {}`: a call
/// without `input_responses` answers an `InputRequiredResult` with the request state
/// [`MRTR_STATE`]; a call with them (the continuation round) answers the text `answered`. rmcp
/// lets an `InputRequiredResult` reach only a request of protocol version 2026-07-28 or newer.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Mrtr;

impl ServerHandler for Mrtr {
    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> + MaybeSendFuture + '_ {
        let schema = json!({"type": "object"}).as_object().cloned();
        let ask = Tool::new("ask", "Asks the client", schema.unwrap_or_default());
        ready(Ok(ListToolsResult::with_all_items(vec![ask])))
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResponse, ErrorData>> + MaybeSendFuture + '_ {
        let response = if request.input_responses.is_some() {
            CallToolResult::success(vec![ContentBlock::text("answered")]).into()
        } else {
            InputRequiredResult::from_request_state(MRTR_STATE).into()
        };
        ready(Ok(response))
    }

    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
    }
}

/// `identity` as JSON: `{"name"?, "version"?}` with the fields it has, or `null` for none.
pub(crate) fn identity_json(identity: Option<&ClientIdentity>) -> Value {
    let Some(identity) = identity else {
        return Value::Null;
    };
    let mut object = Map::new();
    if let Some(name) = &identity.name {
        object.insert("name".to_owned(), json!(name));
    }
    if let Some(version) = &identity.version {
        object.insert("version".to_owned(), json!(version));
    }
    Value::Object(object)
}

/// A [`Sink`] that keeps every log line and every call record.
#[derive(Debug, Default)]
pub(crate) struct RecordingSink {
    logs: Mutex<Vec<String>>,
    records: Mutex<Vec<CallRecord>>,
}

impl RecordingSink {
    /// The log lines so far, oldest first.
    pub(crate) fn logs(&self) -> Vec<String> {
        self.logs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The call records so far, oldest first; the sink keeps none of them.
    pub(crate) fn take_records(&self) -> Vec<CallRecord> {
        std::mem::take(&mut *self.records.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

impl Sink for RecordingSink {
    fn log(&self, message: String) {
        self.logs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message);
    }

    fn record(&self, record: CallRecord) {
        self.records
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(record);
    }
}

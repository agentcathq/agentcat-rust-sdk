//! The tool-call pipeline over in-memory connections: the schema of the called tool and
//! the stripped arguments, foreign tools, the mint-back and its
//! mirror, `get_more_tools`, tracing off,
//! errors, MRTR, the degraded path, the hooks' snapshot, the
//! [`CallContext`], the record's fields, concurrent calls and cancelled
//! calls. The texts the agent reads come from the functions the fixtures pin
//! (`build_mint_back_text`, `report_missing`, ...); the log lines are the texts the call path
//! writes for a rebuild, a degraded call and a failed lookup.
use std::future::ready;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt;

use rmcp::model::{
    CallToolRequest, CallToolRequestParams, CallToolResponse, CallToolResult, ClientCapabilities,
    ClientRequest, Implementation, InitializeRequestParams, InitializeResult, InputResponses,
    ListToolsResult, PaginatedRequestParams, ProtocolVersion, RequestMetaObject,
    ServerCapabilities, ServerResult, Tool,
};
use rmcp::service::{MaybeSendFuture, RequestContext, RoleClient, RunningService, ServiceError};
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde_json::{Value, json};

use super::RecordGuard;
use crate::call::{error_data_error, report_missing, result_error};
use crate::client::AgentCat;
use crate::config::Options;
use crate::constants::{
    AGENT_ID_PARAM, DEFAULT_CONTEXT_PARAMETER_DESCRIPTION, GET_MORE_TOOLS_NAME,
    GET_MORE_TOOLS_RESULT_TEXT, MCP_SESSION_KEY, SESSION_ID_PARAM_PATTERN,
    get_more_tools_descriptor,
};
use crate::context::CallContext;
use crate::event::CallRecord;
use crate::handles::{
    HandleResolution, SessionIdFailure, SessionSource, build_mint_back_text,
    build_structured_mint_back,
};
use crate::hooks::snapshots_built;
use crate::inject::{InjectConfig, InjectState, build_injected_list};
use crate::test_support::json::object;
use crate::test_support::rmcp::{
    ContextEcho, MRTR_STATE, Mrtr, PAGED_CURSOR, Paged, Probe, RecordingSink, Sample, SilentPaged,
    bare_context, connect,
};
use crate::tokens::{estimate_input_tokens, estimate_output_tokens};

/// A connected test client.
type Client = RunningService<RoleClient, InitializeRequestParams>;

/// The log line of a rebuild that worked.
const REBUILT: &str =
    "Rebuilt injection registries on demand (tools/call before tools/list on this instance)";

/// The log line of a rebuild that timed out.
const REBUILD_TIMED_OUT: &str =
    "Warning: registry rebuild-on-demand failed - Error: tools/list did not respond within 5000ms";

/// The tag of the session source.
const SESSION_SOURCE_TAG: &str = "agentcat_session_id_source";

/// A handle with `options` whose log lines and records go to a [`RecordingSink`].
fn recording(options: Options) -> (AgentCat, Arc<RecordingSink>) {
    let sink = Arc::new(RecordingSink::default());
    let agentcat = AgentCat::with_sink("", options, sink.clone());
    (agentcat, sink)
}

/// The defaults with agent tracking on, so tools get `session_id`, `agent_id` and `context`.
fn tracking() -> Options {
    Options::default().enable_agent_tracking(true)
}

/// The request for the tool `name` with the arguments `arguments`.
fn params(name: &str, arguments: Value) -> CallToolRequestParams {
    CallToolRequestParams::new(name.to_owned()).with_arguments(object(arguments))
}

/// Calls the tool `name` with `arguments`; the call must reach a result.
async fn call(client: &Client, name: &str, arguments: Value) -> CallToolResult {
    client
        .call_tool(params(name, arguments))
        .await
        .unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// Calls the tool `name` with `arguments`; the call must fail with a JSON-RPC error.
async fn call_error(client: &Client, name: &str, arguments: Value) -> ErrorData {
    match client.call_tool(params(name, arguments)).await {
        Err(ServiceError::McpError(error)) => error,
        other => panic!("{name}: expected a JSON-RPC error, got {other:?}"),
    }
}

/// The texts of the text blocks of `result`, in order.
fn texts(result: &CallToolResult) -> Vec<&str> {
    result
        .content
        .iter()
        .filter_map(|block| block.as_text().map(|text| text.text.as_str()))
        .collect()
}

/// The one record `sink` holds.
fn only_record(sink: &RecordingSink) -> CallRecord {
    let mut records = sink.take_records();
    assert_eq!(records.len(), 1, "one record: {records:?}");
    records.remove(0)
}

/// The keys of `object`, in order.
fn keys(object: &serde_json::Map<String, Value>) -> Vec<&str> {
    object.keys().map(String::as_str).collect()
}

/// The mint-back text of `record`'s handles.
fn mint_back(record: &CallRecord) -> String {
    build_mint_back_text(&record.resolution).expect("the call gets a mint-back text")
}

/// The arguments of a call to a tool whose own arguments are `own`, with the three injected
/// parameters added the way an agent sends them on its first call.
fn with_handles(own: Value) -> Value {
    let mut arguments = object(own);
    arguments.insert("session_id".to_owned(), json!("start"));
    arguments.insert("agent_id".to_owned(), json!("opus|test|a1b2c"));
    arguments.insert(
        "context".to_owned(),
        json!("Checking that the tool gets only its own arguments."),
    );
    Value::Object(arguments)
}

/// A session ID for tests: a well-formed session ID, `ses_` and the letters and digits
/// of `label` padded with `0` to 27.
fn sid(label: &str) -> String {
    let body: String = label
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .chain(std::iter::repeat('0'))
        .take(27)
        .collect();
    format!("ses_{body}")
}

/// Whether `result` starts with a mint-back block: a first text block that
/// begins `[session_id`.
fn has_mint_back(result: &CallToolResult) -> bool {
    texts(result)
        .first()
        .is_some_and(|text| text.starts_with("[session_id"))
}

/// The arguments a [`Paged`] tool answered with: its structured content without the mirror.
fn paged_arguments(result: &CallToolResult) -> Value {
    let mut structured = result
        .structured_content
        .clone()
        .expect("Paged answers with structured content");
    if let Value::Object(object) = &mut structured {
        object.shift_remove(MCP_SESSION_KEY);
    }
    structured
}

// --- the listing the calls build on ------------------------------------------------------------

/// The tool `name` of `tools`, as JSON.
fn listed(tools: &[Tool], name: &str) -> Value {
    let tool = tools
        .iter()
        .find(|tool| tool.name == name)
        .unwrap_or_else(|| panic!("{name} is listed"));
    serde_json::to_value(tool).unwrap()
}

/// The listings of the end-to-end suites (`v2/lowlevel-handles`, `v2/agent-tracking`,
/// `v2/get-more-tools`, `context-preservation`): with agent tracking on, a tool gets `session_id`,
/// `agent_id` and `context` after its own parameters, all required, and `context` has the default
/// description unless the tool declares its own; `get_more_tools` gets both handles, a required
/// `context` and its annotations, and a plain-object output schema gets `mcp_session`. With agent
/// tracking off (the default), no tool gets `agent_id`; with tracing off, no tool gets a handle
/// but `get_more_tools` is listed; with report-missing off, the handles stay and `get_more_tools`
/// is gone (`handles-integration`).
#[tokio::test]
async fn listing_follows_the_handle_options() {
    let plain = connect(Sample, "plain").await;
    let own = plain.list_all_tools().await.expect("the listing works");
    let own_context = listed(&own, "own_context")["inputSchema"]["properties"]["context"].clone();

    let (agentcat, _sink) = recording(tracking());
    let client = connect(agentcat.track(Sample), "tracking").await;
    let tools = client.list_all_tools().await.expect("the listing works");
    let echo = listed(&tools, "echo");
    let schema = &echo["inputSchema"];
    let all = ["msg", "session_id", "agent_id", "context"];
    assert_eq!(keys(schema["properties"].as_object().unwrap()), all);
    assert_eq!(schema["required"], json!(all));
    assert_eq!(
        schema["properties"]["context"]["description"],
        DEFAULT_CONTEXT_PARAMETER_DESCRIPTION
    );
    let pattern = &schema["properties"]["session_id"]["pattern"];
    assert_eq!(pattern, SESSION_ID_PARAM_PATTERN);
    let structured = listed(&tools, "structured");
    let mirror = &structured["outputSchema"]["properties"][MCP_SESSION_KEY];
    assert_eq!(mirror["type"], "object");
    let mirrored = mirror["properties"].as_object().unwrap();
    assert_eq!(keys(mirrored), ["session_id", "agent_id", "status"]);

    let mine = listed(&tools, "own_context");
    let context = &mine["inputSchema"]["properties"]["context"];
    assert_eq!(context, &own_context, "the tool's own context stays");
    assert_ne!(
        context["description"],
        DEFAULT_CONTEXT_PARAMETER_DESCRIPTION
    );

    let more = listed(&tools, GET_MORE_TOOLS_NAME);
    let descriptor = get_more_tools_descriptor();
    assert_eq!(more["description"], descriptor["description"]);
    let required = more["inputSchema"]["required"].as_array().unwrap();
    for name in ["context", "session_id", "agent_id"] {
        assert!(required.contains(&json!(name)), "{name} is required");
    }
    assert_eq!(more["annotations"], descriptor["annotations"]);

    let (agentcat, _sink) = recording(Options::default());
    let client = connect(agentcat.track(Sample), "defaults").await;
    for tool in client.list_all_tools().await.expect("the listing works") {
        let properties = tool.input_schema.get("properties");
        let agent_id = properties.and_then(|p| p.get("agent_id"));
        assert!(agent_id.is_none(), "{} has no agent_id", tool.name);
    }

    let (agentcat, _sink) = recording(tracking().enable_tracing(false));
    let client = connect(agentcat.track(Sample), "tracing off").await;
    let tools = client.list_all_tools().await.expect("the listing works");
    let properties = &listed(&tools, "echo")["inputSchema"]["properties"];
    assert_eq!(keys(properties.as_object().unwrap()), ["msg", "context"]);
    listed(&tools, GET_MORE_TOOLS_NAME);

    let (agentcat, _sink) = recording(tracking().enable_report_missing(false));
    let client = connect(agentcat.track(Sample), "report-missing off").await;
    let tools = client.list_all_tools().await.expect("the listing works");
    assert!(tools.iter().all(|tool| tool.name != GET_MORE_TOOLS_NAME));
    let schema = &listed(&tools, "echo")["inputSchema"];
    assert_eq!(keys(schema["properties"].as_object().unwrap()), all);
    assert_eq!(schema["required"], json!(all));
}

// --- the schema of the called tool and the stripped arguments ---------------------------------

/// After a listing, a `deny_unknown_fields` tool called with `session_id`, `agent_id`
/// and `context` receives only its own field and succeeds.
#[tokio::test]
async fn strict_tool_gets_stripped_arguments_after_listing() {
    let (agentcat, sink) = recording(tracking());
    let client = connect(agentcat.track(Sample), "strict").await;
    client.list_all_tools().await.expect("the listing works");
    let result = call(&client, "strict", with_handles(json!({"text": "hi"}))).await;
    let record = only_record(&sink);
    assert_ne!(result.is_error, Some(true), "{result:?}");
    assert_eq!(texts(&result), [mint_back(&record).as_str(), "hi"]);
    assert!(!record.is_error);
}

/// On a fresh handle (no listing), the schema comes from `inner.get_tool`, so the strict tool
/// succeeds without a rebuild.
#[tokio::test]
async fn strict_tool_gets_stripped_arguments_before_listing() {
    let (agentcat, sink) = recording(tracking());
    let client = connect(agentcat.track(Sample), "strict").await;
    let result = call(&client, "strict", with_handles(json!({"text": "hi"}))).await;
    let record = only_record(&sink);
    assert_eq!(texts(&result), [mint_back(&record).as_str(), "hi"]);
    assert!(
        !sink.logs().iter().any(|line| line == REBUILT),
        "no rebuild"
    );
    let listed = agentcat
        .inner()
        .with_registry::<Sample, _>(|registry| registry.listed());
    assert!(!listed, "a get_tool lookup records no listing");
}

/// The log lines of the tool-list pipeline over every page of [`Paged`], as a listing of each
/// page on a fresh handle with `options` writes them.
async fn paged_pipeline_logs(options: &Options) -> Vec<String> {
    let plain = connect(Paged, "plain").await;
    let config = InjectConfig::from_options(options);
    let mut state = InjectState::default();
    let mut logs = Vec::new();
    for cursor in [None, Some(PAGED_CURSOR)] {
        let page = PaginatedRequestParams::default().with_cursor(cursor.map(str::to_owned));
        let tools = plain
            .list_tools(Some(page))
            .await
            .expect("the page lists")
            .tools;
        logs.extend(build_injected_list(&config, tools, &mut state).logs);
    }
    logs
}

/// A hand-written server without `get_tool` and no listing yet: the first call
/// rebuilds the registry from every page (the pipeline's lines, then the rebuild line, once), and
/// the strict tool gets its own field only. The tool has no output schema, so the rebuilt registry
/// closes the mirror gate.
#[tokio::test]
async fn strict_tool_without_get_tool_rebuilds_registry() {
    let mut expected = paged_pipeline_logs(&tracking()).await;
    expected.push(REBUILT.to_owned());

    let (agentcat, sink) = recording(tracking());
    let client = connect(agentcat.track(Paged), "paged").await;
    let result = call(&client, "alpha", with_handles(json!({"x": "1"}))).await;
    assert_eq!(
        result.structured_content,
        Some(json!({"x": "1"})),
        "no mirror"
    );
    assert_eq!(sink.logs(), expected);

    let again = call(&client, "beta", with_handles(json!({"x": "2"}))).await;
    let mut own = with_handles(json!({"x": "2"}));
    own.as_object_mut()
        .unwrap()
        .retain(|key, _| key == "x" || key == "context");
    assert_eq!(
        paged_arguments(&again),
        own,
        "page 2 was rebuilt too; beta's context is its own"
    );
    assert_eq!(sink.logs(), expected, "the registry stays rebuilt");
    assert_eq!(sink.take_records().len(), 2);
}

/// Rebuild on demand, as the per-request instances of an HTTP server need it: a
/// fresh handle on a server without `get_tool` gets a call before any listing, with the defaults,
/// no `session_id` and the context `rebuild on demand`. The registry is rebuilt, the call mints a
/// `ses_` session and answers the mint-back first, the record keeps that session, the tool name
/// and the intent, and the tool gets its own argument only (the rebuild restored the strip). A
/// second fresh handle, as the next per-request instance, rebuilds too and keeps that session with
/// no mint-back (a call that arrives before any listing rebuilds the registries on demand).
#[tokio::test]
async fn rebuild_on_demand_mints_and_records_the_call() {
    let (agentcat, sink) = recording(Options::default());
    let client = connect(agentcat.track(Paged), "cold").await;
    let arguments = json!({"x": "cold start", "context": "rebuild on demand"});
    let result = call(&client, "alpha", arguments).await;
    assert!(
        sink.logs().iter().any(|line| line == REBUILT),
        "{:?}",
        sink.logs()
    );
    let record = only_record(&sink);
    assert_eq!(record.resolution.session_source, SessionSource::Minted);
    let session = record.resolution.session_id.clone();
    assert!(session.starts_with("ses_"), "{session}");
    let block = mint_back(&record);
    assert!(block.contains(&format!("session_id: {session}")), "{block}");
    assert_eq!(texts(&result).first(), Some(&block.as_str()));
    assert_eq!(record.resource_name, "alpha");
    assert_eq!(record.user_intent, Some(json!("rebuild on demand")));
    assert_eq!(paged_arguments(&result), json!({"x": "cold start"}));

    let (agentcat, sink) = recording(Options::default());
    let client = connect(agentcat.track(Paged), "next instance").await;
    let arguments = json!({"x": "warm", "session_id": session, "context": "rebuild on demand"});
    let result = call(&client, "alpha", arguments).await;
    assert!(sink.logs().iter().any(|line| line == REBUILT), "rebuilt");
    let record = only_record(&sink);
    assert_eq!(record.resolution.session_source, SessionSource::Supplied);
    assert_eq!(record.resolution.session_id, session);
    assert_eq!(texts(&result), [json!({"x": "warm"}).to_string()]);
    assert_eq!(paged_arguments(&result), json!({"x": "warm"}));
}

/// A listing that never answers bounds the call at 5 s; the call goes on
/// with the heuristic strip, and with no listing ever recorded the mirror gate is open.
#[tokio::test]
async fn rebuild_times_out_after_5s() {
    let (agentcat, sink) = recording(tracking());
    let client = connect(agentcat.track(SilentPaged), "silent").await;
    tokio::time::pause();
    let started = tokio::time::Instant::now();
    let result = call(&client, "alpha", with_handles(json!({"x": "1"}))).await;
    assert!(started.elapsed() >= Duration::from_secs(5));
    assert_eq!(sink.logs(), [REBUILD_TIMED_OUT]);
    let record = only_record(&sink);
    assert!(record.duration_ms >= 5000, "{}", record.duration_ms);
    assert_eq!(
        paged_arguments(&result),
        json!({"x": "1"}),
        "heuristic strip"
    );
    let mint = build_structured_mint_back(&record.resolution).expect("minted");
    let mirrored = json!({MCP_SESSION_KEY: mint, "x": "1"});
    assert_eq!(
        result.structured_content,
        Some(mirrored),
        "the mirror goes first"
    );
}

/// The text the `echo` tool of [`FailingList`] answers.
const FAILING_LIST_ANSWER: &str = "ok";

/// A hand-written server whose listing always fails with `list boom` and whose every tool call
/// answers [`FAILING_LIST_ANSWER`].
#[derive(Debug, Clone, Copy)]
struct FailingList;

impl ServerHandler for FailingList {
    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> + MaybeSendFuture + '_ {
        ready(Err(ErrorData::internal_error("list boom", None)))
    }

    fn call_tool(
        &self,
        _request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResponse, ErrorData>> + MaybeSendFuture + '_ {
        let text = rmcp::model::ContentBlock::text(FAILING_LIST_ANSWER);
        ready(Ok(CallToolResult::success(vec![text]).into()))
    }

    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
    }
}

/// A listing that fails every time does not stop the calls: each call tries the rebuild
/// again (no listing was ever recorded), logs the failure, and still reaches the handler.
#[tokio::test]
async fn rebuild_retries_after_a_failed_listing() {
    let (agentcat, sink) = recording(Options::default());
    let client = connect(agentcat.track(FailingList), "failing").await;
    for _ in 0..2 {
        let result = call(&client, "echo", json!({"msg": "hi"})).await;
        assert!(texts(&result).contains(&FAILING_LIST_ANSWER), "{result:?}");
    }
    let failed = "Warning: registry rebuild-on-demand failed - Error: list boom";
    assert_eq!(sink.logs(), [failed, failed]);
    assert_eq!(sink.take_records().len(), 2);
}

/// A composed-input `union_tool`: its `oneOf` input schema gets
/// no injected parameters.
fn union_tool() -> Tool {
    let schema = object(json!({
        "type": "object",
        "oneOf": [
            {
                "properties": {"kind": {"const": "a"}, "a": {"type": "string"}},
                "required": ["kind", "a"]
            },
            {
                "properties": {"kind": {"const": "b"}, "b": {"type": "number"}},
                "required": ["kind", "b"]
            }
        ]
    }));
    Tool::new("union_tool", "takes one of two shapes", schema)
}

/// A hand-written server that lists [`union_tool`] and answers a call with its arguments as
/// structured content. `Composed(true)` also answers `get_tool` with the tool; `Composed(false)`
/// answers `None`, as rmcp's default `get_tool` does.
#[derive(Debug, Clone, Copy)]
struct Composed(bool);

impl ServerHandler for Composed {
    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> + MaybeSendFuture + '_ {
        ready(Ok(ListToolsResult::with_all_items(vec![union_tool()])))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        (self.0 && name == "union_tool").then(union_tool)
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResponse, ErrorData>> + MaybeSendFuture + '_ {
        let arguments = Value::Object(request.arguments.unwrap_or_default());
        ready(Ok(CallToolResult::structured(arguments).into()))
    }

    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
    }
}

/// A composed-input tool gets no injected parameters, and every listed tool still
/// gets a registry entry, an empty one here (the strip
/// fallback applies only to tools never seen in any listing; call.json case
/// `strip:entry:empty-list`). So the handler receives the arguments as sent, whether the SDK
/// learns the tool from `get_tool`, from a listing, or from a rebuild.
#[tokio::test]
async fn composed_tool_gets_the_same_arguments_on_every_path() {
    let sent = with_handles(json!({"kind": "a", "a": "x"}));
    let paths = [
        ("get_tool", true, false),
        ("registry", false, true),
        ("rebuild", false, false),
    ];
    for (path, answers_get_tool, lists_first) in paths {
        let (agentcat, _sink) = recording(tracking());
        let client = connect(agentcat.track(Composed(answers_get_tool)), path).await;
        if lists_first {
            client.list_all_tools().await.expect("the listing works");
        }
        let result = call(&client, "union_tool", sent.clone()).await;
        assert_eq!(result.structured_content, Some(sent.clone()), "{path}");
    }
}

/// Composed-schema tests: `union_tool`'s `oneOf`
/// input schema gets no injected parameter (none under `properties`, both `oneOf` branches kept),
/// yet nobody declared `session_id`, so the name is the SDK's: a call without one mints a session
/// and answers the mint-back first, tagged `minted`; a call that echoes a well-formed session is
/// honored, `supplied` and not `foreign`, with no mint-back.
#[tokio::test]
async fn composed_tool_mints_and_honors_an_issued_session() {
    let (agentcat, sink) = recording(Options::default());
    let client = connect(agentcat.track(Composed(false)), "composed").await;
    let tools = client.list_all_tools().await.expect("the listing works");
    let schema = &listed(&tools, "union_tool")["inputSchema"];
    let properties = schema.get("properties");
    assert!(
        properties.is_none_or(|p| p.get("session_id").is_none()),
        "{schema}"
    );
    assert_eq!(
        schema["oneOf"].as_array().map(Vec::len),
        Some(2),
        "{schema}"
    );

    let result = call(&client, "union_tool", json!({"kind": "a", "a": "x"})).await;
    let record = only_record(&sink);
    assert_eq!(record.resolution.session_source, SessionSource::Minted);
    let block = mint_back(&record);
    assert!(block.starts_with("[session_id issued"), "{block}");
    assert_eq!(texts(&result).first(), Some(&block.as_str()));
    let issued = &record.resolution.session_id;
    assert!(block.contains(&format!("session_id: {issued}")), "{block}");
    assert_eq!(record.tags[SESSION_SOURCE_TAG], "minted");

    let echoed = sid("union");
    let arguments = json!({"kind": "b", "b": 2, "session_id": echoed});
    let result = call(&client, "union_tool", arguments).await;
    assert!(!has_mint_back(&result), "{result:?}");
    let record = only_record(&sink);
    assert_eq!(record.resolution.session_source, SessionSource::Supplied);
    assert_eq!(record.resolution.session_id, echoed);
    assert_eq!(record.tags[SESSION_SOURCE_TAG], "supplied");
}

/// A tool on page 2 that declares its own `context` keeps it after page 1 is listed
/// again, because the registry merges pages; the heuristic strip would remove it.
#[tokio::test]
async fn page_two_tool_keeps_own_context() {
    let (agentcat, sink) = recording(tracking());
    let client = connect(agentcat.track(Paged), "paged").await;
    for cursor in [Some(PAGED_CURSOR), None] {
        let page = PaginatedRequestParams::default().with_cursor(cursor.map(str::to_owned));
        client.list_tools(Some(page)).await.expect("the page lists");
    }
    let mut arguments = with_handles(json!({"x": "1"}));
    arguments["context"] = json!("The tool's own context value.");
    let result = call(&client, "beta", arguments).await;
    let own = json!({"x": "1", "context": "The tool's own context value."});
    assert_eq!(paged_arguments(&result), own);
    let record = only_record(&sink);
    assert_eq!(
        record.user_intent,
        Some(json!("The tool's own context value.")),
        "the tool's own context is kept"
    );
    assert!(!sink.logs().iter().any(|line| line == REBUILT));
}

// --- tools that declare their own parameters ---------------------------------------------------

/// A tool that declares its own `session_id` gets the value untouched, even `start`;
/// no mint-back text; the session source is `foreign`. The same before a listing (its `get_tool`
/// schema) and after one (the registry's declared set).
#[tokio::test]
async fn own_session_param_is_foreign() {
    let (agentcat, sink) = recording(tracking());
    let client = connect(agentcat.track(Sample), "foreign").await;
    for listed in [false, true] {
        if listed {
            client.list_all_tools().await.expect("the listing works");
        }
        let arguments =
            json!({"session_id": "start", "context": "Uses its own session parameter."});
        let result = call(&client, "own_session", arguments).await;
        assert_eq!(texts(&result), ["start"], "listed: {listed}");
        let record = only_record(&sink);
        assert_eq!(record.resolution.session_source, SessionSource::Foreign);
        assert_eq!(record.resolution.session_id, "");
        assert_eq!(
            record.tags[SESSION_SOURCE_TAG],
            SessionSource::Foreign.as_str()
        );
    }
}

/// A tool that declares its own `context` receives it untouched, and it is still recorded as
/// the intent.
#[tokio::test]
async fn own_context_reaches_handler() {
    let (agentcat, sink) = recording(Options::default());
    let client = connect(agentcat.track(Sample), "own-context").await;
    let arguments = json!({"context": "The tool's own context.", "session_id": "start"});
    let result = call(&client, "own_context", arguments).await;
    let record = only_record(&sink);
    assert_eq!(
        texts(&result),
        [mint_back(&record).as_str(), "The tool's own context."]
    );
    assert_eq!(record.user_intent, Some(json!("The tool's own context.")));
}

// --- the mint-back and its mirror -----------------------------------------------------------

/// `start` mints and an unknown value is unrecognized, each with its text first; a
/// supplied ID gets no text.
#[tokio::test]
async fn mint_back_for_minted_and_invalid_only() {
    let (agentcat, sink) = recording(Options::default());
    let client = connect(agentcat.track(Sample), "mint-back").await;

    let minted = call(&client, "echo", json!({"msg": "hi", "session_id": "start"})).await;
    let record = only_record(&sink);
    assert_eq!(record.resolution.session_source, SessionSource::Minted);
    assert_eq!(texts(&minted), [mint_back(&record).as_str(), "hi"]);
    let session = record.resolution.session_id;

    let invalid = call(
        &client,
        "echo",
        json!({"msg": "hi", "session_id": "ses_bogus"}),
    )
    .await;
    let record = only_record(&sink);
    assert_eq!(record.resolution.session_source, SessionSource::Invalid);
    assert_eq!(texts(&invalid), [mint_back(&record).as_str(), "hi"]);

    let supplied = call(&client, "echo", json!({"msg": "hi", "session_id": session})).await;
    let record = only_record(&sink);
    assert_eq!(record.resolution.session_source, SessionSource::Supplied);
    assert_eq!(record.resolution.session_id, session);
    assert_eq!(texts(&supplied), ["hi"]);
}

/// Agent tracking is off by default: an omitted option resolves no agent ID
/// (`enable_agent_tracking` defaults to `false`): with the default options, a call
/// that sends an `agent_id` still mints a session and resolves no agent: no agent ID on the
/// record, no agent tags, and a mint-back without an `agent_id` line.
#[tokio::test]
async fn default_options_resolve_no_agent() {
    let (agentcat, sink) = recording(Options::default());
    let client = connect(agentcat.track(Sample), "defaults").await;
    let arguments = json!({"msg": "hi", "agent_id": "agt_ignored"});
    let result = call(&client, "echo", arguments).await;
    let record = only_record(&sink);
    assert_eq!(record.resolution.session_source, SessionSource::Minted);
    assert_eq!(record.resolution.agent_id, None);
    assert!(
        !record.tags.contains_key("agentcat_agent_id"),
        "{:?}",
        record.tags
    );
    assert!(!record.tags.contains_key("agentcat_agent_id_source"));
    let block = mint_back(&record);
    assert!(!block.contains("agent_id"), "{block}");
    assert_eq!(texts(&result).first(), Some(&block.as_str()));
}

/// Handles that the SDK injected are stripped before the original handler and kept raw on the event,
/// echoed handles are reused verbatim with no mint-back, and the callback receives
/// stripped args with supplied handles reused silently.
/// With agent tracking on, a call that echoes a well-formed `session_id`, an `agent_id` and
/// a `context` reaches the tool with its own argument only and gets no mint-back, before a listing
/// (the rebuild) and after one. The record keeps the session and the agent verbatim, tagged
/// `supplied`, the context as the intent, and the arguments as sent in its request.
#[tokio::test]
async fn supplied_handles_are_stripped_for_the_tool_and_kept_on_the_record() {
    let session = sid("fixed");
    let agent = "opus-4.80-1m|claude-code|k3n9x";
    let sent = json!({"x": "hi", "session_id": session, "agent_id": agent, "context": "why"});
    for listed in [false, true] {
        let (agentcat, sink) = recording(tracking());
        let client = connect(agentcat.track(Paged), "supplied").await;
        if listed {
            client.list_all_tools().await.expect("the listing works");
        }
        let result = call(&client, "alpha", sent.clone()).await;
        assert_eq!(
            paged_arguments(&result),
            json!({"x": "hi"}),
            "listed: {listed}"
        );
        assert!(!has_mint_back(&result), "listed: {listed}: {result:?}");
        let record = only_record(&sink);
        assert_eq!(record.resolution.session_source, SessionSource::Supplied);
        assert_eq!(record.resolution.session_id, session);
        assert_eq!(record.resolution.agent_id.as_deref(), Some(agent));
        assert_eq!(record.tags[SESSION_SOURCE_TAG], "supplied");
        assert_eq!(record.tags["agentcat_agent_id"], agent);
        assert_eq!(record.tags["agentcat_agent_id_source"], "supplied");
        assert_eq!(record.user_intent, Some(json!("why")));
        assert_eq!(
            record.request["params"]["arguments"], sent,
            "listed: {listed}"
        );
    }
}

/// A hand-written server without `get_tool` whose tools answer structured content: `plain` (an
/// object output schema), `composed` (a `oneOf` output schema, skipped by the pipeline), `owned`
/// (an object output schema; its value has its own `mcp_session`) and `bare` (no output schema).
#[derive(Debug, Clone, Copy)]
struct Outputs;

/// The tool `name` of [`Outputs`] with the output schema `output`.
fn outputs_tool(name: &'static str, output: Option<Value>) -> Tool {
    let tool = Tool::new(
        name,
        "answers structured content",
        object(json!({"type": "object"})),
    );
    match output {
        Some(schema) => tool.with_raw_output_schema(Arc::new(object(schema))),
        None => tool,
    }
}

impl ServerHandler for Outputs {
    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> + MaybeSendFuture + '_ {
        let plain = json!({"type": "object", "properties": {"msg": {"type": "string"}}});
        let composed = json!({"type": "object", "oneOf": [{"properties": {"msg": {}}}]});
        let tools = vec![
            outputs_tool("plain", Some(plain.clone())),
            outputs_tool("composed", Some(composed)),
            outputs_tool("owned", Some(plain)),
            outputs_tool("bare", None),
        ];
        ready(Ok(ListToolsResult::with_all_items(tools)))
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResponse, ErrorData>> + MaybeSendFuture + '_ {
        let structured = if request.name == "owned" {
            json!({MCP_SESSION_KEY: "the customer's", "msg": "hi"})
        } else {
            json!({"msg": "hi"})
        };
        ready(Ok(CallToolResult::structured(structured).into()))
    }

    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
    }
}

/// The mirror goes first into the structured content of a tool whose output
/// schema got `mcp_session` (a hand-written listing, and the macro's `structured` through
/// `get_tool`); a tool whose output schema was skipped or that has none gets no mirror, but still
/// the mint-back text first; a customer's own `mcp_session` stays.
#[tokio::test]
async fn mirror_gate_follows_registry() {
    let (agentcat, sink) = recording(Options::default());
    let client = connect(agentcat.track(Outputs), "outputs").await;
    client.list_all_tools().await.expect("the listing works");
    let start = || json!({"session_id": "start"});

    let plain = call(&client, "plain", start()).await;
    let record = only_record(&sink);
    let mint = build_structured_mint_back(&record.resolution).expect("minted");
    let mirrored = json!({MCP_SESSION_KEY: mint, "msg": "hi"});
    assert_eq!(
        serde_json::to_string(&plain.structured_content).unwrap(),
        serde_json::to_string(&mirrored).unwrap(),
        "mcp_session first"
    );
    for name in ["composed", "bare"] {
        let result = call(&client, name, start()).await;
        assert_eq!(
            result.structured_content,
            Some(json!({"msg": "hi"})),
            "{name}"
        );
        let record = only_record(&sink);
        assert_eq!(
            texts(&result)[0],
            mint_back(&record),
            "{name}: the text block"
        );
    }
    let owned = call(&client, "owned", start()).await;
    let own = json!({MCP_SESSION_KEY: "the customer's", "msg": "hi"});
    assert_eq!(owned.structured_content, Some(own));

    let (agentcat, sink) = recording(Options::default());
    let client = connect(agentcat.track(Sample), "structured").await;
    let result = call(
        &client,
        "structured",
        json!({"msg": "hi", "session_id": "start"}),
    )
    .await;
    let record = only_record(&sink);
    let mint = build_structured_mint_back(&record.resolution).expect("minted");
    let mirrored = json!({MCP_SESSION_KEY: mint, "msg": "hi"});
    assert_eq!(
        serde_json::to_string(&result.structured_content).unwrap(),
        serde_json::to_string(&mirrored).unwrap()
    );
}

/// The record keeps the customer's result: the same content and structured content as the
/// handler alone answers, without the mint-back text or the mirror.
#[tokio::test]
async fn record_keeps_the_original_result() {
    let plain = connect(Sample, "plain").await;
    let expected = call(&plain, "structured", json!({"msg": "hi"})).await;
    let expected = serde_json::to_value(&expected).unwrap();

    let (agentcat, sink) = recording(Options::default());
    let client = connect(agentcat.track(Sample), "tracked").await;
    let arguments = json!({"msg": "hi", "session_id": "start", "context": "Keeps the result."});
    let decorated = call(&client, "structured", arguments).await;
    let record = only_record(&sink);
    assert_eq!(texts(&decorated).len(), 2, "the wire has the mint-back");
    let response = record.response.expect("a result is recorded");
    for key in ["content", "structuredContent", "isError"] {
        assert_eq!(response.get(key), expected.get(key), "{key}");
    }
    assert_eq!(
        record.output_tokens,
        estimate_output_tokens(Some(&response)),
        "the original result is the one counted"
    );
}

// --- get_more_tools -------------------------------------------------------------------------

/// `get_more_tools` is answered by the SDK with the canned text after the mint-back, logged
/// with the context length, and recorded with its session, the arguments as sent, the context as
/// the intent and the canned result as the response.
#[tokio::test]
async fn get_more_tools_answered_and_recorded() {
    let (agentcat, sink) = recording(Options::default());
    let client = connect(agentcat.track(Sample), "more").await;
    client.list_all_tools().await.expect("the listing works");
    let before = sink.logs().len();
    let context = json!("Needs a tool that books meeting rooms.");
    let arguments = json!({"context": context, "session_id": "start"});
    let result = call(&client, GET_MORE_TOOLS_NAME, arguments.clone()).await;
    let (answer, line) = report_missing(Some(&context));
    let record = only_record(&sink);
    assert_eq!(
        texts(&result),
        [mint_back(&record).as_str(), GET_MORE_TOOLS_RESULT_TEXT]
    );
    assert_eq!(record.resource_name, GET_MORE_TOOLS_NAME);
    assert_eq!(record.request["params"]["name"], GET_MORE_TOOLS_NAME);
    assert_eq!(record.request["params"]["arguments"], arguments);
    assert!(record.resolution.session_id.starts_with("ses_"), "minted");
    assert_eq!(record.user_intent, Some(context));
    assert_eq!(
        record.response,
        Some(serde_json::to_value(&answer).unwrap())
    );
    assert!(!record.is_error);
    assert_eq!(sink.logs()[before..], [line]);
}

/// Report-missing keeps task continuity: when `report_missing` is called between other
/// tools, and when several missing tools are reported in the same task, an
/// agent that echoes one well-formed `session_id` on every call, `get_more_tools` included, gets
/// no mint-back, and every record keeps that session, `supplied`, in the order of the calls, each
/// with its own tool and context.
#[tokio::test]
async fn supplied_session_holds_across_get_more_tools_calls() {
    let session = sid("report_missing_continuity");
    let (agentcat, sink) = recording(Options::default());
    let client = connect(agentcat.track(Sample), "continuity").await;
    let calls = [
        ("echo", "Adding first todo to test task continuity"),
        (GET_MORE_TOOLS_NAME, "Importing data from spreadsheets"),
        (GET_MORE_TOOLS_NAME, "Need to validate imported data"),
        (GET_MORE_TOOLS_NAME, "Process large datasets"),
        ("echo", "Listing todos after reporting missing tool"),
    ];
    for (name, context) in calls {
        let mut arguments = json!({"context": context, "session_id": session});
        if name == "echo" {
            arguments["msg"] = json!("First todo");
        }
        let result = call(&client, name, arguments).await;
        assert!(!has_mint_back(&result), "{name}: {result:?}");
    }
    let records = sink.take_records();
    assert_eq!(records.len(), calls.len());
    for (record, (name, context)) in records.iter().zip(calls) {
        assert_eq!(record.resource_name, name);
        assert_eq!(record.resolution.session_source, SessionSource::Supplied);
        assert_eq!(record.resolution.session_id, session, "{context}");
        assert_eq!(record.user_intent, Some(json!(context)));
    }
}

/// Report-missing tracks `get_more_tools` across different sessions:
/// two tracked servers, each with its own connection, each answering a `get_more_tools` call that
/// sends no `session_id`, record two different minted sessions, each with its own context.
#[tokio::test]
async fn get_more_tools_mints_a_session_per_connection() {
    let mut sessions = Vec::new();
    let contexts = [
        ("first", "Need OAuth integration tool"),
        ("second", "Need OAuth2 authentication tool"),
    ];
    for (name, context) in contexts {
        let (agentcat, sink) = recording(Options::default());
        let client = connect(agentcat.track(Sample), name).await;
        let arguments = json!({"context": context});
        call(&client, GET_MORE_TOOLS_NAME, arguments.clone()).await;
        let record = only_record(&sink);
        assert_eq!(record.resource_name, GET_MORE_TOOLS_NAME);
        assert_eq!(record.resolution.session_source, SessionSource::Minted);
        assert!(record.resolution.session_id.starts_with("ses_"), "{name}");
        assert_eq!(record.request["params"]["arguments"], arguments);
        assert_eq!(record.user_intent, Some(json!(context)));
        sessions.push(record.resolution.session_id);
    }
    assert_ne!(sessions[0], sessions[1]);
}

/// The text the customer's own `get_more_tools` of [`OwnMore`] answers.
const CUSTOMER_MORE: &str = "the customer's get_more_tools";

/// A hand-written server that lists and answers its own `get_more_tools`.
#[derive(Debug, Clone, Copy)]
struct OwnMore;

impl ServerHandler for OwnMore {
    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> + MaybeSendFuture + '_ {
        let schema =
            object(json!({"type": "object", "properties": {"context": {"type": "string"}}}));
        let tool = Tool::new(GET_MORE_TOOLS_NAME, "the customer's own", schema);
        ready(Ok(ListToolsResult::with_all_items(vec![tool])))
    }

    fn call_tool(
        &self,
        _request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResponse, ErrorData>> + MaybeSendFuture + '_ {
        let text = rmcp::model::ContentBlock::text(CUSTOMER_MORE);
        ready(Ok(CallToolResult::success(vec![text]).into()))
    }

    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
    }
}

/// With report-missing on, the SDK answers a customer's own `get_more_tools`; with it off,
/// the customer's tool runs: traced, with tracing off, and on the degraded path. Client and server
/// share this current-thread runtime, so the session ID switch reaches the server.
#[tokio::test]
async fn customer_get_more_tools_is_shadowed() {
    let arguments = || json!({"context": "Looking for more tools.", "session_id": "start"});
    let (agentcat, _sink) = recording(Options::default());
    let client = connect(agentcat.track(OwnMore), "shadowed").await;
    client.list_all_tools().await.expect("the listing works");
    let result = call(&client, GET_MORE_TOOLS_NAME, arguments()).await;
    assert_eq!(texts(&result).last(), Some(&GET_MORE_TOOLS_RESULT_TEXT));

    let off = Options::default().enable_report_missing(false);
    let (agentcat, sink) = recording(off);
    let client = connect(agentcat.track(OwnMore), "reachable").await;
    let result = call(&client, GET_MORE_TOOLS_NAME, arguments()).await;
    assert_eq!(texts(&result).last(), Some(&CUSTOMER_MORE));
    let record = only_record(&sink);
    assert_eq!(
        record.user_intent, None,
        "no intent for the name get_more_tools"
    );

    let untraced = Options::default()
        .enable_report_missing(false)
        .enable_tracing(false);
    let (agentcat, sink) = recording(untraced);
    let client = connect(agentcat.track(OwnMore), "untraced").await;
    let result = call(&client, GET_MORE_TOOLS_NAME, arguments()).await;
    assert_eq!(texts(&result), [CUSTOMER_MORE], "tracing off");
    assert!(sink.take_records().is_empty());

    let off = Options::default().enable_report_missing(false);
    let (agentcat, sink) = recording(off);
    let client = connect(agentcat.track(OwnMore), "degraded").await;
    let _failure = SessionIdFailure::start();
    let result = call(&client, GET_MORE_TOOLS_NAME, arguments()).await;
    assert_eq!(texts(&result), [CUSTOMER_MORE], "degraded path");
    assert!(sink.take_records().is_empty());
}

// --- tracing off, errors, MRTR, the degraded path ---------------------------------------------

/// The arguments that [`ContextEcho`] received, from its answer.
fn echoed_arguments(result: &CallToolResult) -> Value {
    let echo = result
        .structured_content
        .as_ref()
        .expect("ContextEcho answers structured content");
    echo["request"]["params"]["arguments"].clone()
}

/// With tracing off, the SDK still strips what it injected before the handler
/// runs. A `deny_unknown_fields` tool receives its own field only, after a listing (the
/// registry) and before one (`get_tool`), and gets no mint-back; a tool the SDK has no schema
/// for (a handler without `get_tool` whose listing is empty) gets the heuristic strip of
/// `session_id`, `agent_id` and `context`.
#[tokio::test]
async fn tracing_off_strips_injected_arguments() {
    let arguments = json!({"text": "hi", "context": "Tracing is off; the SDK still strips this."});
    for listed in [true, false] {
        let (agentcat, sink) = recording(Options::default().enable_tracing(false));
        let client = connect(agentcat.track(Sample), "strict").await;
        if listed {
            client.list_all_tools().await.expect("the listing works");
        }
        let result = call(&client, "strict", arguments.clone()).await;
        assert_ne!(result.is_error, Some(true), "listed: {listed}: {result:?}");
        assert_eq!(texts(&result), ["hi"], "listed: {listed}: no mint-back");
        assert!(sink.take_records().is_empty());
    }

    let (agentcat, sink) = recording(Options::default().enable_tracing(false));
    let client = connect(agentcat.track(ContextEcho), "heuristic").await;
    let result = call(&client, "echo", with_handles(json!({"msg": "hi"}))).await;
    assert_eq!(echoed_arguments(&result), json!({"msg": "hi"}));
    assert!(
        result
            .structured_content
            .unwrap()
            .get(MCP_SESSION_KEY)
            .is_none()
    );
    assert!(sink.take_records().is_empty());
}

/// With tracing off, a tool that declares its own `context` receives it, before
/// any listing (its `get_tool` schema) and after one (the registry); the heuristic strip would remove
/// it.
#[tokio::test]
async fn tracing_off_keeps_own_context() {
    let (agentcat, sink) = recording(Options::default().enable_tracing(false));
    let client = connect(agentcat.track(Sample), "own-context").await;
    for listed in [false, true] {
        if listed {
            client.list_all_tools().await.expect("the listing works");
        }
        let arguments = json!({"context": "The tool's own context."});
        let result = call(&client, "own_context", arguments).await;
        assert_eq!(
            texts(&result),
            ["The tool's own context."],
            "listed: {listed}"
        );
    }
    assert!(
        !sink.logs().iter().any(|line| line == REBUILT),
        "get_tool knew the tool"
    );
    assert!(sink.take_records().is_empty());
}

/// With tracing off nothing is recorded: not a result, an `is_error` result, a
/// JSON-RPC error or a panic; `get_more_tools` is still answered (and logged), with no mint-back.
/// After a listing, the only log line is the intercept's.
#[tokio::test]
async fn tracing_off_records_nothing() {
    let (agentcat, sink) = recording(Options::default().enable_tracing(false));
    let client = connect(agentcat.track(Sample), "off").await;
    client.list_all_tools().await.expect("the listing works");
    let listed = sink.logs().len();
    call(&client, "echo", json!({"msg": "hi", "session_id": "start"})).await;
    let failed = call(&client, "fail", json!({})).await;
    assert_eq!(failed.is_error, Some(true));
    call_error(&client, "nope", json!({})).await;

    let context = json!("Looking for more tools.");
    let arguments = json!({"context": context, "session_id": "start"});
    let result = call(&client, GET_MORE_TOOLS_NAME, arguments).await;
    assert_eq!(texts(&result), [GET_MORE_TOOLS_RESULT_TEXT], "no mint-back");
    assert_eq!(sink.logs()[listed..], [report_missing(Some(&context)).1]);

    let tracked = agentcat.track(Sample);
    let exploded =
        AssertUnwindSafe(tracked.call_tool(params("explode", json!({})), bare_context(None)))
            .catch_unwind()
            .await;
    assert!(exploded.is_err(), "the panic reaches the caller");
    assert!(sink.take_records().is_empty(), "nothing is recorded");
}

/// With tracing off, a call to a handler without `get_tool` that never
/// listed still rebuilds the registry to learn the schema, but writes none of the rebuild's
/// log lines: not the pipeline's, not the rebuild line, not a failed rebuild's. `alpha` loses the
/// injected `context`; `beta`, rebuilt from page 2, keeps its own, which the heuristic strip would drop.
#[tokio::test]
async fn tracing_off_rebuild_logs_nothing() {
    let (agentcat, sink) = recording(Options::default().enable_tracing(false));
    let client = connect(agentcat.track(Paged), "paged-off").await;
    let arguments = json!({"x": "1", "context": "Tracing is off."});
    let alpha = call(&client, "alpha", arguments.clone()).await;
    assert_eq!(
        paged_arguments(&alpha),
        json!({"x": "1"}),
        "the injected context"
    );
    let beta = call(&client, "beta", arguments.clone()).await;
    assert_eq!(paged_arguments(&beta), arguments, "beta's own context");
    let listed = agentcat
        .inner()
        .with_registry::<Paged, _>(|registry| registry.listed());
    assert!(listed, "the rebuild recorded the listing");
    assert!(sink.logs().is_empty(), "{:?}", sink.logs());

    let (agentcat, sink) = recording(Options::default().enable_tracing(false));
    let client = connect(agentcat.track(FailingList), "failing-off").await;
    let result = call(&client, "echo", json!({"msg": "hi"})).await;
    assert!(texts(&result).contains(&FAILING_LIST_ANSWER), "{result:?}");
    assert!(sink.logs().is_empty(), "{:?}", sink.logs());
    assert!(sink.take_records().is_empty());
}

/// The degraded path (no session ID could be issued) keeps the raw arguments, also
/// after a listing: the handler receives `session_id`, `agent_id` and `context` as sent, and
/// nothing is recorded.
#[tokio::test]
async fn degraded_path_keeps_raw_arguments() {
    let (agentcat, sink) = recording(tracking());
    let client = connect(agentcat.track(ContextEcho), "degraded").await;
    client.list_all_tools().await.expect("the listing works");
    let _failure = SessionIdFailure::start();
    let raw = with_handles(json!({"msg": "hi"}));
    let result = call(&client, "echo", raw.clone()).await;
    assert_eq!(echoed_arguments(&result), raw);
    assert!(sink.take_records().is_empty());
}

/// The call path builds the hooks' [`ToolCall`] snapshot only when
/// a hook starts: never without hooks, once for a call with a hook; in hook mode the session hook
/// alone counts.
///
/// [`ToolCall`]: crate::ToolCall
#[tokio::test]
async fn snapshot_built_only_with_hooks() {
    let cases = [
        ("no hooks", Options::default(), 0),
        (
            "event_tags",
            Options::default().event_tags(|_call| ready(Ok(None))),
            1,
        ),
        (
            "hook mode",
            Options::default().resolve_session_id(|_call| ready(Ok(None))),
            1,
        ),
        (
            "every hook",
            Options::default()
                .resolve_session_id(|_call| ready(Ok(None)))
                .identify(|_call| ready(Ok(None)))
                .event_tags(|_call| ready(Ok(None)))
                .event_properties(|_call| ready(Ok(None))),
            1,
        ),
    ];
    for (label, options, expected) in cases {
        let (agentcat, sink) = recording(options);
        let client = connect(agentcat.track(Sample), label).await;
        let before = snapshots_built();
        call(&client, "echo", json!({"msg": "hi"})).await;
        assert_eq!(snapshots_built() - before, expected, "{label}");
        assert_eq!(sink.take_records().len(), 1, "{label}");
    }
}

/// An `Err(ErrorData)` reaches the client exactly as without the SDK; the
/// record has the error with the JSON-RPC code name as its type, and no response.
#[tokio::test]
async fn error_data_returned_unchanged() {
    let plain = connect(Probe, "plain").await;
    let expected = call_error(&plain, "greet", json!({"session_id": "start"})).await;

    let (agentcat, sink) = recording(Options::default());
    let client = connect(agentcat.track(Probe), "tracked").await;
    let actual = call_error(&client, "greet", json!({"session_id": "start"})).await;
    assert_eq!(
        serde_json::to_value(&actual).unwrap(),
        serde_json::to_value(&expected).unwrap()
    );
    let record = only_record(&sink);
    assert!(record.is_error);
    assert_eq!(record.error, Some(error_data_error(&expected)));
    assert_eq!(
        record.error.unwrap()["type"],
        "InternalError",
        "the code name"
    );
    assert_eq!(record.response, None);
    assert_eq!(record.output_tokens, None);
}

/// Rmcp's error for an unknown tool reaches the client unchanged (no mint-back is
/// possible) and is recorded.
#[tokio::test]
async fn unknown_tool_error_unchanged() {
    let plain = connect(Sample, "plain").await;
    let expected = call_error(&plain, "nope", json!({"session_id": "start"})).await;

    let (agentcat, sink) = recording(Options::default());
    let client = connect(agentcat.track(Sample), "tracked").await;
    let actual = call_error(&client, "nope", json!({"session_id": "start"})).await;
    assert_eq!(
        serde_json::to_value(&actual).unwrap(),
        serde_json::to_value(&expected).unwrap()
    );
    let record = only_record(&sink);
    assert_eq!(record.resource_name, "nope");
    assert_eq!(
        record.error.unwrap()["type"],
        "InvalidParams",
        "the code name"
    );
}

/// An `is_error` result gets the mint-back text first (the session part only,
/// since no `agent_id` was sent); the record has the result's error, computed on the customer's
/// result.
#[tokio::test]
async fn is_error_result_gets_mint_back() {
    let plain = connect(Sample, "plain").await;
    let original = call(&plain, "fail", json!({})).await;

    let (agentcat, sink) = recording(tracking());
    let client = connect(agentcat.track(Sample), "tracked").await;
    let result = call(&client, "fail", json!({"session_id": "start"})).await;
    let record = only_record(&sink);
    assert_eq!(result.is_error, Some(true));
    assert_eq!(texts(&result), [mint_back(&record).as_str(), "failed"]);
    assert!(
        !texts(&result)[0].contains(AGENT_ID_PARAM),
        "no agent_id was sent"
    );
    assert!(record.is_error);
    assert_eq!(record.error, Some(result_error(&original)));
    let response = record.response.expect("a result is recorded");
    assert_eq!(
        response["content"],
        serde_json::to_value(&original.content).unwrap()
    );
}

/// A `tools/call` request for `params` that names the protocol version 2026-07-28 in its `_meta`,
/// the version from which rmcp lets an `InputRequiredResult` reach the client.
fn modern_call(params: CallToolRequestParams) -> ClientRequest {
    let mut request = CallToolRequest::new(params);
    request
        .extensions
        .insert(RequestMetaObject::with_client_context(
            ProtocolVersion::V_2026_07_28,
            Implementation::new("mrtr-client", "1.0.0"),
            ClientCapabilities::default(),
        ));
    ClientRequest::CallToolRequest(request)
}

/// Sends `request` and returns the server's result.
async fn send(client: &Client, request: ClientRequest) -> ServerResult {
    client
        .send_request(request)
        .await
        .expect("the request is answered")
}

/// An `InputRequiredResult` reaches the client as the handler answered it (no mint-back),
/// and the record is tagged `agentcat_mrtr: input_required`, after the other tags.
#[tokio::test]
async fn input_required_is_tagged_not_decorated() {
    let ask = || params("ask", json!({"session_id": "start"}));
    let plain = connect(Mrtr, "plain").await;
    let expected = serde_json::to_value(send(&plain, modern_call(ask())).await).unwrap();
    assert_eq!(expected["requestState"], MRTR_STATE);

    let (agentcat, sink) = recording(Options::default());
    let client = connect(agentcat.track(Mrtr), "tracked").await;
    let actual = serde_json::to_value(send(&client, modern_call(ask())).await).unwrap();
    assert_eq!(actual, expected);
    let record = only_record(&sink);
    assert_eq!(
        keys(&record.tags),
        [
            SESSION_SOURCE_TAG,
            "agentcat_protocol_version",
            "agentcat_mrtr"
        ]
    );
    assert_eq!(record.tags["agentcat_mrtr"], "input_required");
    assert_eq!(record.response, Some(expected));
    assert!(!record.is_error);
}

/// A call with input responses is tagged `agentcat_mrtr: continuation`, first among
/// the tags, and its complete result gets the mint-back.
#[tokio::test]
async fn continuation_is_tagged() {
    let mut responses = InputResponses::new();
    responses.insert("question".to_owned(), json!({"action": "accept"}));
    let request = params("ask", json!({"session_id": "start"})).with_input_responses(responses);
    let (agentcat, sink) = recording(Options::default());
    let client = connect(agentcat.track(Mrtr), "tracked").await;
    let ServerResult::CallToolResult(result) = send(&client, modern_call(request)).await else {
        panic!("the continuation completes");
    };
    let record = only_record(&sink);
    assert_eq!(texts(&result), [mint_back(&record).as_str(), "answered"]);
    assert_eq!(
        keys(&record.tags),
        [
            "agentcat_mrtr",
            SESSION_SOURCE_TAG,
            "agentcat_protocol_version"
        ]
    );
    assert_eq!(record.tags["agentcat_mrtr"], "continuation");
}

/// When no session ID can be minted, the call takes the degraded path:
/// a log line, the raw arguments to the handler (so the strict tool refuses them), no record;
/// `get_more_tools` is still answered. Client and server share this current-thread runtime, so
/// the switch reaches the server.
#[tokio::test]
async fn session_id_failure_takes_degraded_path() {
    let (agentcat, sink) = recording(Options::default());
    let client = connect(agentcat.track(Sample), "degraded").await;
    client.list_all_tools().await.expect("the listing works");
    let before = sink.logs().len();
    let _failure = SessionIdFailure::start();

    let result = call(
        &client,
        "strict",
        json!({"text": "hi", "session_id": "start"}),
    )
    .await;
    assert_eq!(
        result.is_error,
        Some(true),
        "the raw session_id reached the tool"
    );
    assert!(
        texts(&result)[0].contains("unknown field `session_id`"),
        "{result:?}"
    );

    let context = json!("Looking for more tools.");
    let more = call(&client, GET_MORE_TOOLS_NAME, json!({"context": context})).await;
    assert_eq!(texts(&more), [GET_MORE_TOOLS_RESULT_TEXT]);

    let failed = |name: &str| {
        format!(
            "Warning: AgentCat tracing failed for tool {name}, falling back to original handler - Error: could not generate a session ID"
        )
    };
    let expected = [
        failed("strict"),
        failed(GET_MORE_TOOLS_NAME),
        report_missing(Some(&context)).1,
    ];
    assert_eq!(sink.logs()[before..], expected);
    assert!(sink.take_records().is_empty());
}

// --- the call context, the record, concurrency ------------------------------------------------

/// The `{"session_id", "agent_id"}` that `whoami` answered (after the mint-back, when any).
fn whoami(result: &CallToolResult) -> Value {
    let text = texts(result)
        .last()
        .copied()
        .expect("whoami answers a text");
    serde_json::from_str(text).expect("whoami answers JSON")
}

/// A tool reads its call's session and agent from the [`CallContext`]: the minted ID,
/// a supplied ID, and no session for an unrecognized value.
#[tokio::test]
async fn call_context_reaches_tool() {
    let (agentcat, sink) = recording(tracking());
    let client = connect(agentcat.track(Sample), "whoami").await;
    let agent = "opus|whoami|x1y2z";

    let minted = call(
        &client,
        "whoami",
        json!({"session_id": "start", "agent_id": agent}),
    )
    .await;
    let session = only_record(&sink).resolution.session_id;
    assert_eq!(
        whoami(&minted),
        json!({"session_id": session, "agent_id": agent})
    );

    let supplied = call(&client, "whoami", json!({"session_id": session})).await;
    sink.take_records();
    assert_eq!(
        whoami(&supplied),
        json!({"session_id": session, "agent_id": null})
    );

    let invalid = call(
        &client,
        "whoami",
        json!({"session_id": "nope", "agent_id": agent}),
    )
    .await;
    sink.take_records();
    assert_eq!(
        whoami(&invalid),
        json!({"session_id": null, "agent_id": agent})
    );
}

/// A call the SDK does not trace (tracing off, or the degraded path
/// when no session ID can be issued) still reaches the tool with a [`CallContext`], one with no
/// session and no agent.
#[tokio::test]
async fn call_context_reaches_untraced_tool() {
    let none = json!({"session_id": null, "agent_id": null});
    let arguments = || json!({"session_id": "start", "agent_id": "opus|whoami|x1y2z"});

    let (agentcat, sink) = recording(tracking().enable_tracing(false));
    let client = connect(agentcat.track(Sample), "tracing off").await;
    let result = call(&client, "whoami", arguments()).await;
    assert_eq!(whoami(&result), none, "tracing off");
    assert!(sink.take_records().is_empty());

    let (agentcat, sink) = recording(tracking());
    let client = connect(agentcat.track(Sample), "degraded").await;
    let _failure = SessionIdFailure::start();
    let result = call(&client, "whoami", arguments()).await;
    assert_eq!(whoami(&result), none, "degraded path");
    assert!(sink.take_records().is_empty());
}

/// The resource name (`Unknown Tool` for an empty name), the intent rules,
/// the order of the tags, the token estimates, and the server and client of the record.
#[tokio::test]
async fn record_fields_hold_the_resource_name_intent_tags_and_tokens() {
    let (agentcat, sink) = recording(tracking());
    let client = connect(agentcat.track(Sample), "fields").await;
    let raw =
        json!({"msg": "hi", "session_id": "start", "agent_id": "opus|fields|q9", "context": "why"});
    let mut request = params("echo", raw.clone());
    let mut meta = RequestMetaObject::new();
    meta.set_protocol_version(ProtocolVersion::V_2025_11_25);
    request.meta = Some(meta);
    client.call_tool(request).await.expect("the call works");
    let record = only_record(&sink);
    assert_eq!(record.resource_name, "echo");
    assert_eq!(record.user_intent, Some(json!("why")));
    assert_eq!(
        keys(&record.tags),
        [
            SESSION_SOURCE_TAG,
            "agentcat_agent_id",
            "agentcat_agent_id_source",
            "agentcat_protocol_version"
        ]
    );
    assert_eq!(record.tags["agentcat_protocol_version"], "2025-11-25");
    assert_eq!(record.input_tokens, estimate_input_tokens(Some(&raw)));
    assert!(record.input_tokens.is_some());
    assert_eq!(
        record.output_tokens,
        estimate_output_tokens(record.response.as_ref())
    );
    assert!(record.output_tokens.is_some());
    let server = Sample.get_info().server_info;
    assert_eq!(record.server_name, Some(server.name));
    assert_eq!(record.server_version, Some(server.version));
    let client_name = record.client.and_then(|client| client.name);
    assert_eq!(client_name.as_deref(), Some("fields"));
    assert_eq!(record.request["params"]["arguments"], raw);

    call_error(&client, "", json!({})).await;
    assert_eq!(only_record(&sink).resource_name, "Unknown Tool");

    call(&client, "echo", json!({"msg": "hi", "context": 42})).await;
    assert_eq!(only_record(&sink).user_intent, Some(json!(42)), "any type");
    call(&client, "echo", json!({"msg": "hi"})).await;
    assert_eq!(only_record(&sink).user_intent, None, "no context argument");

    let (agentcat, sink) = recording(Options::default().enable_tool_call_context(false));
    let client = connect(agentcat.track(Sample), "no-context").await;
    call(&client, "echo", json!({"msg": "hi", "context": "why"})).await;
    assert_eq!(only_record(&sink).user_intent, None, "context off");
    call(&client, GET_MORE_TOOLS_NAME, json!({"context": "why"})).await;
    assert_eq!(
        only_record(&sink).user_intent,
        Some(json!("why")),
        "get_more_tools records its context regardless"
    );
}

/// End to end, with a custom `context` description,
/// every listed tool shows it, and each call records its own `context` as the intent.
#[tokio::test]
async fn custom_description_listing_and_intents() {
    let description = "Explain your intent for this operation";
    let options = Options::default().custom_context_description(description);
    let (agentcat, sink) = recording(options);
    let client = connect(agentcat.track(Sample), "custom").await;
    let tools = client.list_all_tools().await.expect("the listing works");
    for name in ["echo", "strict", "structured"] {
        let context = &listed(&tools, name)["inputSchema"]["properties"]["context"];
        assert_eq!(context["description"], description, "{name}");
    }
    let calls = [
        (
            "echo",
            json!({"msg": "a", "context": "Setting up my first task"}),
        ),
        (
            "strict",
            json!({"text": "b", "context": "Finishing the first task"}),
        ),
        (
            "structured",
            json!({"msg": "c", "context": "Checking my progress"}),
        ),
    ];
    for (name, arguments) in calls {
        let intent = arguments["context"].clone();
        let result = call(&client, name, arguments).await;
        assert_ne!(result.is_error, Some(true), "{name}: {result:?}");
        let record = only_record(&sink);
        assert_eq!(record.resource_name, name);
        assert_eq!(record.user_intent, Some(intent), "{name}");
    }
}

/// Concurrent calls, each with its own agent, each get their own minted session, in the
/// mint-back, in the call context and in the record.
#[tokio::test]
async fn concurrent_calls_are_independent() {
    let (agentcat, sink) = recording(tracking());
    let client = connect(agentcat.track(Sample), "concurrent").await;
    let calls = (0..10).map(|n| {
        let client = &client;
        async move {
            let agent = format!("opus|concurrent|{n}");
            let arguments = json!({"session_id": "start", "agent_id": agent});
            (agent, call(client, "whoami", arguments).await)
        }
    });
    let results = futures::future::join_all(calls).await;

    let mut sessions = std::collections::BTreeSet::new();
    for (agent, result) in &results {
        let echo = whoami(result);
        assert_eq!(echo["agent_id"], json!(agent));
        let session = echo["session_id"].as_str().expect("minted").to_owned();
        let resolution = HandleResolution {
            session_id: session.clone(),
            session_source: SessionSource::Minted,
            hook_mode: false,
            agent_id: Some(agent.clone()),
        };
        assert_eq!(texts(result)[0], build_mint_back_text(&resolution).unwrap());
        sessions.insert(session);
    }
    assert_eq!(sessions.len(), 10, "ten distinct sessions");

    let records = sink.take_records();
    assert_eq!(records.len(), 10);
    for record in records {
        let agent = record
            .resolution
            .agent_id
            .clone()
            .expect("each call names its agent");
        let (_, result) = results
            .iter()
            .find(|(sent, _)| *sent == agent)
            .expect("a call sent this agent");
        assert_eq!(
            whoami(result)["session_id"],
            json!(record.resolution.session_id)
        );
    }
}

// --- cancelled calls --------------------------------------------------------------

/// How long the tool of [`Slow`] takes.
const SLOW_TOOL: Duration = Duration::from_secs(60);

/// A hand-written server whose every tool call answers after [`SLOW_TOOL`].
#[derive(Debug, Clone, Copy)]
struct Slow;

impl ServerHandler for Slow {
    #[expect(
        clippy::manual_async_fn,
        reason = "the signature of rmcp's trait method: a returned future with MaybeSendFuture"
    )]
    fn call_tool(
        &self,
        _request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResponse, ErrorData>> + MaybeSendFuture + '_ {
        async {
            tokio::time::sleep(SLOW_TOOL).await;
            Ok(CallToolResult::success(vec![]).into())
        }
    }

    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
    }
}

/// A call future dropped before the tool answers (here by the customer's timeout)
/// publishes its record with no response, `is_error`, the cancelled error in the fixed key order,
/// and the duration until the drop.
#[tokio::test]
async fn cancelled_call_publishes_record() {
    tokio::time::pause();
    let (agentcat, sink) = recording(Options::default());
    let tracked = agentcat.track(Slow);
    let arguments = json!({"session_id": "start", "context": "Takes too long."});
    let call = tracked.call_tool(params("slow", arguments), bare_context(None));
    let outcome = tokio::time::timeout(Duration::from_secs(1), call).await;
    assert!(outcome.is_err(), "the timeout drops the call");

    let record = only_record(&sink);
    assert_eq!(record.resource_name, "slow");
    assert!(record.is_error);
    assert_eq!(record.response, None);
    assert_eq!(record.output_tokens, None);
    assert_eq!(record.user_intent, Some(json!("Takes too long.")));
    let error = Value::Object(record.error.expect("the cancelled error"));
    assert_eq!(
        serde_json::to_string(&error).unwrap(),
        r#"{"message":"tool call cancelled before it completed","type":"cancelled","platform":"rust"}"#
    );
    assert!(
        (1000..SLOW_TOOL.as_millis()).contains(&u128::from(record.duration_ms)),
        "{}",
        record.duration_ms
    );
}

/// A call that completes goes to the sink exactly once, never as cancelled: a result, an
/// `is_error` result, a JSON-RPC error, the `get_more_tools` intercept and a panic.
#[tokio::test]
async fn completed_call_is_recorded_once() {
    let (agentcat, sink) = recording(Options::default());
    let tracked = agentcat.track(Sample);
    let cases = [
        ("echo", json!({"msg": "hi"}), None),
        ("fail", json!({}), None),
        ("nope", json!({}), Some("InvalidParams")),
        (GET_MORE_TOOLS_NAME, json!({"context": "why"}), None),
        ("explode", json!({}), Some("panic")),
    ];
    for (name, arguments, error_type) in cases {
        let call = tracked.call_tool(params(name, arguments), bare_context(None));
        let _outcome = AssertUnwindSafe(call).catch_unwind().await;
        let record = only_record(&sink);
        let recorded_type = record.error.as_ref().and_then(|error| error.get("type"));
        assert_eq!(
            recorded_type,
            error_type.map(Value::from).as_ref(),
            "{name}"
        );
    }
}

/// The guard's log-not-send contract (`RecordGuard::discard`): a discarded record goes to the
/// sink neither as it is nor as cancelled by the guard's drop, and the log line it is given does.
/// The line is the test's own; this does not cover the warning text that `finish` builds.
#[tokio::test]
async fn discarded_record_is_logged_not_sent() {
    let (agentcat, sink) = recording(Options::default());
    let tracked = agentcat.track(Sample);
    tracked
        .call_tool(params("echo", json!({"msg": "hi"})), bare_context(None))
        .await
        .unwrap();
    let record = only_record(&sink);
    let line = "Warning: AgentCat post-handler processing failed for tool echo; returning the customer's result untouched - test".to_owned();
    RecordGuard::new(record, sink.as_ref(), tokio::time::Instant::now()).discard(line.clone());
    assert!(
        sink.take_records().is_empty(),
        "a discarded record is not sent"
    );
    assert_eq!(sink.logs().last(), Some(&line));
}

/// [`CallContext`] reads: `None` for an empty session.
#[test]
fn call_context_reads_none_for_an_empty_session() {
    let agentcat = AgentCat::new("", Options::default());
    let context = CallContext::new(String::new(), None, agentcat.clone());
    assert_eq!(context.session_id(), None);
    assert_eq!(context.agent_id(), None);
    let context = CallContext::new("ses_1".to_owned(), Some("a".to_owned()), agentcat);
    assert_eq!(context.session_id(), Some("ses_1"));
    assert_eq!(context.agent_id(), Some("a"));
    let _: &AgentCat = context.agentcat();
}

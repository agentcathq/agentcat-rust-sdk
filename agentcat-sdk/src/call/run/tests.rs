//! The tool-call pipeline (B-022) over in-memory connections: the schema of the called tool and
//! the stripped arguments (C03, B-050, B-051, RD-12), foreign tools (B-060), the mint-back and its
//! mirror (B-056 to B-059, B-069), `get_more_tools` (B-028, B-046), tracing off (R27, RD-29),
//! errors (RD-05, D16, C24), MRTR (B-025), the degraded path (K-05), the hooks' snapshot, the
//! [`CallContext`], the record's fields (B-077, B-083, B-096), concurrent calls and cancelled
//! calls (C41, RD-34). The texts the agent reads come from the functions the TS vectors pin
//! (`build_mint_back_text`, `report_missing`, ...); the log lines are the TS texts of
//! `src/engine/callWrap.ts`.
use std::future::ready;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt;

use rmcp::model::{
    CallToolRequest, CallToolRequestParams, CallToolResponse, CallToolResult, ClientCapabilities,
    ClientRequest, Implementation, InitializeRequestParams, InitializeResult, InputResponses,
    JsonObject, ListToolsResult, PaginatedRequestParams, ProtocolVersion, RequestMetaObject,
    ServerCapabilities, ServerResult, Tool,
};
use rmcp::service::{MaybeSendFuture, RequestContext, RoleClient, RunningService, ServiceError};
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde_json::{Value, json};

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
use crate::test_support::rmcp::{
    ContextEcho, MRTR_STATE, Mrtr, PAGED_CURSOR, Paged, Probe, RecordingSink, Sample, SilentPaged,
    bare_context, connect,
};
use crate::tokens::{estimate_input_tokens, estimate_output_tokens};

/// A connected test client.
type Client = RunningService<RoleClient, InitializeRequestParams>;

/// The log line of a rebuild that worked (`callWrap.ts:121`).
const REBUILT: &str =
    "Rebuilt injection registries on demand (tools/call before tools/list on this instance)";

/// The log line of a rebuild that timed out (`callWrap.ts:108,124`).
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

/// The JSON object `value`.
fn object(value: Value) -> JsonObject {
    match value {
        Value::Object(object) => object,
        other => panic!("{other} is not an object"),
    }
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

/// The listings of the TS end-to-end suites (`v2/lowlevel-handles`, `v2/agent-tracking`,
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

// --- the schema of the called tool and the stripped arguments (Review Focus 1) ---------------

/// C03, B-050: after a listing, a `deny_unknown_fields` tool called with `session_id`, `agent_id`
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

/// C03: on a fresh handle (no listing), the schema comes from `inner.get_tool`, so the strict tool
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

/// C03, B-027, RD-12: a hand-written server without `get_tool` and no listing yet: the first call
/// rebuilds the registry from every page (the pipeline's lines, then the rebuild line, once), and
/// the strict tool gets its own field only. The tool has no output schema, so the rebuilt registry
/// closes the mirror gate (B-069).
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

/// B-027, B-051, B-069: a listing that never answers bounds the call at 5 s; the call goes on
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
    assert_eq!(result.structured_content, Some(mirrored), "B-069");
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

/// B-027: a listing that fails every time does not stop the calls: each call tries the rebuild
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

/// TS's composed-input `union_tool` (`handles-integration.test.ts`): its `oneOf` input schema gets
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

/// B-050 (Task 7 fix round 1): a composed-input tool gets no injected parameters, and TS still
/// gives every listed tool a registry entry, an empty one here (`listWrap.ts`: "the strip
/// fallback must apply only to tools never seen in any listing"; call.json case
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

/// RD-12, K-01: a tool on page 2 that declares its own `context` keeps it after page 1 is listed
/// again, because the registry merges pages; TS's heuristic strip would remove it.
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
        "K-08"
    );
    assert!(!sink.logs().iter().any(|line| line == REBUILT));
}

// --- tools that declare their own parameters (Review Focus 2) ---------------------------------

/// B-060, C19: a tool that declares its own `session_id` gets the value untouched, even `start`;
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

/// K-08: a tool that declares its own `context` receives it untouched, and it is still recorded as
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

/// B-055, B-056: `start` mints and an unknown value is unrecognized, each with its text first; a
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

/// B-058, B-069, C21: the mirror goes first into the structured content of a tool whose output
/// schema got `mcp_session` (a hand-written listing, and the macro's `structured` through
/// `get_tool`); a tool whose output schema was skipped or that has none gets no mirror, but still
/// the mint-back text first (B-056); a customer's own `mcp_session` stays.
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

/// B-059: the record keeps the customer's result: the same content and structured content as the
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
        "B-072: on the original result"
    );
}

// --- get_more_tools -------------------------------------------------------------------------

/// B-046: `get_more_tools` is answered by the SDK with the canned text after the mint-back, logged
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

/// B-028: with report-missing on, the SDK answers a customer's own `get_more_tools`; with it off,
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
        "B-083: never for the name get_more_tools"
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

/// R27, RD-29, C03: with tracing off, the SDK still strips what it injected before the handler
/// runs. A `deny_unknown_fields` tool receives its own field only, after a listing (the
/// registry) and before one (`get_tool`), and gets no mint-back; a tool the SDK has no schema
/// for (a handler without `get_tool` whose listing is empty) gets TS's heuristic strip of
/// `session_id`, `agent_id` and `context` (B-051).
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

/// R27, RD-29, K-08: with tracing off, a tool that declares its own `context` receives it, before
/// any listing (its `get_tool` schema) and after one (the registry); TS's heuristic would strip
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

/// K-05, B-023: with tracing off nothing is recorded: not a result, an `is_error` result, a
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

/// TS, K-05: the degraded path (no session ID could be issued) keeps the raw arguments, also
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

/// Carry-forward (plan 4 Task 9): the call path builds the hooks' [`ToolCall`] snapshot only when
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

/// Design §7.4, RD-05, C24: an `Err(ErrorData)` reaches the client exactly as without the SDK; the
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
    assert_eq!(record.error.unwrap()["type"], "InternalError", "C24");
    assert_eq!(record.response, None);
    assert_eq!(record.output_tokens, None);
}

/// RD-05, D16: rmcp's error for an unknown tool reaches the client unchanged (no mint-back is
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
    assert_eq!(record.error.unwrap()["type"], "InvalidParams", "C24");
}

/// D16, B-056, B-097: an `is_error` result gets the mint-back text first (the session part only,
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

/// B-025: an `InputRequiredResult` reaches the client as the handler answered it (no mint-back),
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

/// B-025, B-096: a call with input responses is tagged `agentcat_mrtr: continuation`, first among
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

/// B-022 steps 8 and 9, K-05: when no session ID can be minted, the call takes the degraded path:
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

/// Design §5: a tool reads its call's session and agent from the [`CallContext`]: the minted ID,
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

/// Task 7 fix round 1 ruling: a call the SDK does not trace (tracing off, or the degraded path
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

/// B-077, B-083, B-096: the resource name (`Unknown Tool` for an empty name), the intent rules,
/// the order of the tags, the token estimates, and the server and client of the record.
#[tokio::test]
async fn record_fields_follow_b077() {
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

/// TS's `custom-context-description.test.ts` end to end: with a custom `context` description,
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

/// B-029: concurrent calls, each with its own agent, each get their own minted session, in the
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

// --- cancelled calls (C41, RD-34) --------------------------------------------------------------

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

/// C41, RD-34: a call future dropped before the tool answers (here by the customer's timeout)
/// publishes its record with no response, `is_error`, the cancelled error in the RD-06 key order,
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

/// C41: a call that completes goes to the sink exactly once, never as cancelled: a result, an
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

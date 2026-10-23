//! Panics and captured errors of a tool call, over in-memory connections (design §6.4, C23,
//! RD-06, RD-17): a panic of the tool is recorded and resumed, so the client gets no response, as
//! without the SDK, and the server keeps serving; [`capture_error`] replaces the error of the
//! record; a panic in a customer call the SDK makes on the `tools/call` path (the rebuild's
//! `list_tools`, `get_tool`, `get_info`) is contained and the call goes on.
//!
//! These are Rust-only paths (panics, `Error::source`), so the expected values are shapes, not TS
//! outputs. Each panic prints its message through the default panic hook into the test's captured
//! output, which the test runner shows only for a failing test.
use std::any::type_name;
use std::backtrace::{Backtrace, BacktraceStatus};
use std::error::Error;
use std::fmt;
use std::future::ready;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, InitializeRequestParams,
    InitializeResult, JsonObject, ListToolsResult, PaginatedRequestParams, ProtocolVersion,
    ServerCapabilities, Tool,
};
use rmcp::service::{MaybeSendFuture, RequestContext, RoleClient, RunningService, ServiceError};
use rmcp::{ErrorData, RoleServer, ServerHandler, tool, tool_handler, tool_router};
use serde_json::{Map, Value, json};

use super::{CaptureSlot, capture_error, captured_backtrace, error_shape, panic_error, scope};
use crate::call::result_error;
use crate::client::AgentCat;
use crate::config::Options;
use crate::event::CallRecord;
use crate::test_support::rmcp::{RecordingSink, Sample, bare_context, connect};

/// A connected test client.
type Client = RunningService<RoleClient, InitializeRequestParams>;

/// How long a client waits for the response to a call that must get none.
const NO_RESPONSE: Duration = Duration::from_millis(500);

/// How long a call that must be answered may take (a rebuild that times out takes 5 s).
const ANSWERED: Duration = Duration::from_secs(2);

/// The log line of a rebuild that failed with the panic of [`PanickingList`]
/// (`callWrap.ts:108,124`).
const LIST_PANICKED: &str =
    "Warning: registry rebuild-on-demand failed - Error: list_tools exploded";

/// The log line of a `get_tool` lookup of `beta` that panicked on the call path (C30), in the
/// style of [`LIST_PANICKED`]. Rust-only: TS has no `get_tool`.
const LOOKUP_PANICKED: &str = "Warning: Tool lookup for beta panicked: get_tool exploded";

/// The log line of a `get_info` that panicked on the call path (C30), in the style of
/// [`LIST_PANICKED`]. Rust-only: TS reads the server info without customer code.
const INFO_PANICKED: &str = "Warning: get_info panicked: get_info exploded";

/// A handle with the default options whose log lines and records go to a [`RecordingSink`].
fn recording() -> (AgentCat, Arc<RecordingSink>) {
    let sink = Arc::new(RecordingSink::default());
    let agentcat = AgentCat::with_sink("", Options::default(), sink.clone());
    (agentcat, sink)
}

/// The request for the tool `name` with the arguments `arguments`, a JSON object.
fn params(name: &str, arguments: Value) -> CallToolRequestParams {
    let Value::Object(arguments) = arguments else {
        panic!("{arguments} is not an object");
    };
    CallToolRequestParams::new(name.to_owned()).with_arguments(arguments)
}

/// Calls the tool `name` with `arguments`; the server must answer within [`ANSWERED`].
async fn call(
    client: &Client,
    name: &str,
    arguments: Value,
) -> Result<CallToolResult, ServiceError> {
    tokio::time::timeout(ANSWERED, client.call_tool(params(name, arguments)))
        .await
        .unwrap_or_else(|_| panic!("{name}: no response within {ANSWERED:?}"))
}

/// Calls the tool `name` without arguments; the server must not answer within [`NO_RESPONSE`].
async fn unanswered(client: &Client, name: &str) {
    let call = tokio::time::timeout(NO_RESPONSE, client.call_tool(params(name, json!({}))));
    let outcome = call.await;
    assert!(
        outcome.is_err(),
        "{name}: expected no response, got {outcome:?}"
    );
}

/// The one record `sink` holds.
fn only_record(sink: &RecordingSink) -> CallRecord {
    let mut records = sink.take_records();
    assert_eq!(records.len(), 1, "one record: {records:?}");
    records.remove(0)
}

/// The keys of `object`, in order.
fn keys(object: &Map<String, Value>) -> Vec<&str> {
    object.keys().map(String::as_str).collect()
}

/// The `error` of a panic whose message is `message` (C23).
fn panic_shape(message: &str) -> Map<String, Value> {
    let shape = json!({"message": message, "type": "panic", "platform": "rust"});
    shape.as_object().cloned().expect("an object")
}

/// The JSON text of the last content block of `result`: what an [`echo_arguments`] tool received.
fn received(result: &CallToolResult) -> Value {
    let text = result
        .content
        .last()
        .and_then(|block| block.as_text())
        .expect("the last block is text");
    serde_json::from_str(&text.text).expect("the text is JSON")
}

/// A result whose text is the JSON of the arguments of `request`.
fn echo_arguments(request: CallToolRequestParams) -> CallToolResponse {
    let arguments = Value::Object(request.arguments.unwrap_or_default());
    CallToolResult::success(vec![ContentBlock::text(arguments.to_string())]).into()
}

/// Panics with the message `<method> exploded`, a `String` payload.
fn explode<T>(method: &str) -> T {
    panic!("{method} exploded")
}

/// The capabilities of a server with tools.
fn tools_info() -> InitializeResult {
    InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
}

/// The root of [`Wrapper`]'s chain.
#[derive(Debug)]
struct RootCause;

impl fmt::Display for RootCause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("root cause")
    }
}

impl Error for RootCause {}

/// A two-level error: `wrapper failed`, caused by [`RootCause`].
#[derive(Debug)]
struct Wrapper(RootCause);

impl fmt::Display for Wrapper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("wrapper failed")
    }
}

impl Error for Wrapper {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.0)
    }
}

/// The `error` that [`capture_error`] records for a [`Wrapper`] without a backtrace.
fn wrapper_shape() -> Value {
    json!({
        "message": "wrapper failed",
        "type": type_name::<Wrapper>(),
        "platform": "rust",
        "chained_errors": [{"message": "root cause"}],
    })
}

/// The error of `record`, a captured one. It has no `stack`: the record keeps the backtrace for
/// the delivery pipeline, which renders it there, and keeps one only when the environment turns
/// backtraces on (`RUST_BACKTRACE`, `RUST_LIB_BACKTRACE`).
fn captured_error(record: CallRecord) -> Value {
    if let Some(backtrace) = &record.backtrace {
        assert_eq!(backtrace.status(), BacktraceStatus::Captured);
    }
    let error = record.error.expect("the record has an error");
    assert!(!error.contains_key("stack"), "{error:?}");
    Value::Object(error)
}

/// A `#[tool_router]` server whose tools call [`capture_error`] with a [`Wrapper`]: `captured`
/// then fails (an `is_error` result), `captured_ok` then succeeds, and `spawned` calls it in a
/// task it spawns and then fails.
#[derive(Debug, Clone, Copy)]
struct Capturing;

#[tool_router]
impl Capturing {
    #[tool(description = "Captures a two-level error, then fails with its text")]
    async fn captured(&self) -> Result<String, String> {
        let error = Wrapper(RootCause);
        capture_error(&error);
        Err(error.to_string())
    }

    #[tool(description = "Captures a boxed dyn Error, then fails with its text")]
    async fn captured_dyn(&self) -> Result<String, String> {
        let error: Box<dyn Error> = Box::new(RootCause);
        capture_error(&*error);
        Err(error.to_string())
    }

    #[tool(description = "Captures a two-level error, then succeeds")]
    async fn captured_ok(&self) -> String {
        capture_error(&Wrapper(RootCause));
        "fine".to_owned()
    }

    #[tool(description = "Captures a two-level error in a task it spawns, then fails")]
    async fn spawned(&self) -> Result<String, String> {
        tokio::spawn(async { capture_error(&Wrapper(RootCause)) })
            .await
            .map_err(|error| error.to_string())?;
        Err("spawned failed".to_owned())
    }
}

#[expect(
    clippy::unused_async_trait_impl,
    reason = "the methods #[tool_handler] generates are async without an await"
)]
#[tool_handler]
impl ServerHandler for Capturing {}

/// A hand-written server whose `call_tool` calls [`capture_error`] with a [`Wrapper`] before it
/// returns its future: the tool `capture_then_panic` then panics with `call_tool exploded`; any
/// other tool answers the JSON-RPC error `data failed` (`-32603`).
#[derive(Debug, Clone, Copy)]
struct Eager;

impl ServerHandler for Eager {
    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResponse, ErrorData>> + MaybeSendFuture + '_ {
        capture_error(&Wrapper(RootCause));
        if request.name == "capture_then_panic" {
            return ready(explode("call_tool"));
        }
        ready(Err(ErrorData::internal_error("data failed", None)))
    }

    fn get_info(&self) -> InitializeResult {
        tools_info()
    }
}

/// A hand-written server without `get_tool` whose `list_tools` panics with `list_tools
/// exploded`; every tool answers the arguments it received ([`echo_arguments`]).
#[derive(Debug, Clone, Copy)]
struct PanickingList;

impl ServerHandler for PanickingList {
    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> + MaybeSendFuture + '_ {
        ready(explode("list_tools"))
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResponse, ErrorData>> + MaybeSendFuture + '_ {
        ready(Ok(echo_arguments(request)))
    }

    fn get_info(&self) -> InitializeResult {
        tools_info()
    }
}

/// A hand-written server whose `get_tool` panics with `get_tool exploded` and whose listing has
/// one tool, `beta {x, context}`, whose `context` is its own; every tool answers the arguments it
/// received ([`echo_arguments`]).
#[derive(Debug, Clone, Copy)]
struct PanickingGetTool;

impl ServerHandler for PanickingGetTool {
    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> + MaybeSendFuture + '_ {
        let schema = json!({
            "type": "object",
            "properties": {"x": {"type": "string"}, "context": {"type": "string"}},
        });
        let schema: JsonObject = schema.as_object().cloned().unwrap_or_default();
        let beta = Tool::new("beta", "declares its own context", schema);
        ready(Ok(ListToolsResult::with_all_items(vec![beta])))
    }

    fn get_tool(&self, _name: &str) -> Option<Tool> {
        explode("get_tool")
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResponse, ErrorData>> + MaybeSendFuture + '_ {
        ready(Ok(echo_arguments(request)))
    }

    fn get_info(&self) -> InitializeResult {
        tools_info()
    }
}

/// A hand-written server whose `get_info` panics with `get_info exploded`. Its `initialize`
/// answers without `get_info`, so a client can connect; every tool answers the arguments it
/// received ([`echo_arguments`]).
#[derive(Debug, Clone, Copy)]
struct PanickingInfo;

impl ServerHandler for PanickingInfo {
    fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<InitializeResult, ErrorData>> + MaybeSendFuture + '_ {
        context.peer.set_peer_info(request);
        let mut info = tools_info();
        info.protocol_version = ProtocolVersion::V_2025_11_25;
        ready(Ok(info))
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResponse, ErrorData>> + MaybeSendFuture + '_ {
        ready(Ok(echo_arguments(request)))
    }

    fn get_info(&self) -> InitializeResult {
        explode("get_info")
    }
}

/// Design §6.4, C23, Review Focus 3: a panic of the tool reaches the client as it does without
/// the SDK (no response), and the record has the panic: `type: "panic"`, the payload as the
/// message, no response and no output tokens. The SDK resumes the panic with the tool's own
/// payload.
#[tokio::test]
async fn panic_is_recorded_and_resumed() {
    let plain = connect(Sample, "plain").await;
    unanswered(&plain, "explode").await;

    let (agentcat, sink) = recording();
    let client = connect(agentcat.track(Sample), "tracked").await;
    unanswered(&client, "explode").await;
    let record = only_record(&sink);
    assert_eq!(record.resource_name, "explode");
    assert!(record.is_error);
    assert_eq!(record.error, Some(panic_shape("explode")));
    assert_eq!(record.response, None);
    assert_eq!(record.output_tokens, None);

    let tracked = agentcat.track(Sample);
    let call = tracked.call_tool(params("explode", json!({})), bare_context(None));
    let Err(payload) = AssertUnwindSafe(call).catch_unwind().await else {
        panic!("the panic is resumed");
    };
    assert_eq!(
        payload.downcast_ref::<&str>(),
        Some(&"explode"),
        "the tool's own payload"
    );
    assert_eq!(only_record(&sink).error, Some(panic_shape("explode")));
}

/// Review Focus 3: after a tool panicked, the server answers the next call, which gets its own
/// record without the panic.
#[tokio::test]
async fn server_keeps_serving_after_panic() {
    let (agentcat, sink) = recording();
    let client = connect(agentcat.track(Sample), "serving").await;
    unanswered(&client, "explode").await;
    let result = call(&client, "echo", json!({"msg": "still here"}))
        .await
        .expect("echo answers");
    let last = result.content.last().and_then(|block| block.as_text());
    assert_eq!(last.map(|text| text.text.as_str()), Some("still here"));

    let records = sink.take_records();
    assert_eq!(records.len(), 2, "{records:?}");
    assert_eq!(records[0].error, Some(panic_shape("explode")));
    assert_eq!(records[1].resource_name, "echo");
    assert!(!records[1].is_error);
    assert_eq!(records[1].error, None);
}

/// C23: the message of a panic is its payload when that is a `&str` (`panic!` with a literal) or a
/// `String` (`panic!` with arguments), and `Box<dyn Any>` for any other payload, as Rust's panic
/// hook names it.
#[test]
fn panic_payload_kinds() {
    let panics: [(fn(), &str, &str); 3] = [
        (|| panic!("a literal"), "a literal", "&str"),
        (
            || panic!("{}", "with arguments"),
            "with arguments",
            "String",
        ),
        (|| std::panic::panic_any(42_i32), "Box<dyn Any>", "i32"),
    ];
    for (f, message, kind) in panics {
        let payload = std::panic::catch_unwind(f).expect_err("f panics");
        let is_kind = match kind {
            "&str" => payload.is::<&str>(),
            "String" => payload.is::<String>(),
            _ => payload.is::<i32>(),
        };
        assert!(is_kind, "the payload of {message:?} is a {kind}");
        assert_eq!(panic_error(&*payload), panic_shape(message), "{kind}");
    }
}

/// R09, RD-06, RD-17: an error a tool records with `capture_error` before it fails is the error
/// of the record, in place of the result's (B-097): its `Display` text, its type path, the `rust`
/// platform and one chained entry per `source()`, in that key order. The result still reaches
/// the client as rmcp made it.
#[tokio::test]
async fn capture_error_records_type_and_chain() {
    let plain = connect(Capturing, "plain").await;
    let expected = call(&plain, "captured", json!({})).await.expect("a result");

    let (agentcat, sink) = recording();
    let client = connect(agentcat.track(Capturing), "capturing").await;
    let result = call(&client, "captured", json!({}))
        .await
        .expect("a result");
    assert_eq!(result.is_error, Some(true));
    assert_eq!(result.content.last(), expected.content.last());

    let record = only_record(&sink);
    assert!(record.is_error);
    assert!(
        record.runtime.is_some(),
        "the record keeps the call's runtime"
    );
    let error = captured_error(record);
    let order = ["message", "type", "platform", "chained_errors"];
    assert_eq!(keys(error.as_object().expect("an object")), order);
    assert_eq!(error, wrapper_shape());
    assert!(
        type_name::<Wrapper>().ends_with("capture::tests::Wrapper"),
        "the type path"
    );
}

/// `capture_error` keeps the backtrace of its call site, unrendered, when one was captured, and
/// nothing otherwise; the delivery pipeline renders it as `error.stack` (`render_backtrace`), so
/// the error itself has no `stack`. `chained_errors` holds at most 10 entries and is absent for an
/// error without a source.
#[tokio::test]
async fn capture_keeps_the_backtrace() {
    let kept = captured_backtrace(Backtrace::force_capture()).expect("a captured backtrace");
    assert_eq!(kept.status(), BacktraceStatus::Captured);
    assert!(captured_backtrace(Backtrace::disabled()).is_none());

    let slot = CaptureSlot::default();
    scope(slot.clone(), async { capture_error(&Wrapper(RootCause)) }).await;
    let (error, backtrace) = slot.take().expect("the slot holds the error");
    assert_eq!(Value::Object(error), wrapper_shape());
    assert!(
        backtrace.is_none_or(|backtrace| backtrace.status() == BacktraceStatus::Captured),
        "only a captured backtrace is kept"
    );
    assert!(slot.take().is_none(), "take empties the slot");

    let with_chain = error_shape(&Wrapper(RootCause));
    assert_eq!(
        keys(&with_chain),
        ["message", "type", "platform", "chained_errors"]
    );

    let without = error_shape(&RootCause);
    let expected =
        json!({"message": "root cause", "type": type_name::<RootCause>(), "platform": "rust"});
    assert_eq!(Value::Object(without), expected);

    let looping = error_shape(&Looping);
    let chain = looping["chained_errors"].as_array().expect("a chain");
    assert_eq!(chain.len(), 10, "at most 10 entries");
    assert!(
        chain
            .iter()
            .all(|entry| *entry == json!({"message": "looping"}))
    );
}

/// An error that is its own source: its chain never ends.
#[derive(Debug)]
struct Looping;

impl fmt::Display for Looping {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("looping")
    }
}

impl Error for Looping {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self)
    }
}

/// `capture_error` outside a tool call does nothing and does not panic. A task the tool spawns
/// does not inherit the call, and a call's captured error does not outlive it, so the next call's
/// record has the error of its own result (B-097).
#[tokio::test]
async fn capture_error_outside_a_call_does_nothing() {
    capture_error(&Wrapper(RootCause));

    let plain = connect(Capturing, "plain").await;
    let failed = call(&plain, "spawned", json!({})).await.expect("a result");

    let (agentcat, sink) = recording();
    let client = connect(agentcat.track(Capturing), "outside").await;
    capture_error(&Wrapper(RootCause));
    call(&client, "captured", json!({}))
        .await
        .expect("a result");
    assert_eq!(captured_error(only_record(&sink)), wrapper_shape());

    let result = call(&client, "spawned", json!({})).await.expect("a result");
    assert_eq!(result.is_error, Some(true));
    let record = only_record(&sink);
    assert_eq!(record.error, Some(result_error(&failed)));
}

/// B-097: a call that does not fail records no error, even after `capture_error`.
#[tokio::test]
async fn captured_error_without_a_failure_is_dropped() {
    let (agentcat, sink) = recording();
    let client = connect(agentcat.track(Capturing), "ok").await;
    let result = call(&client, "captured_ok", json!({}))
        .await
        .expect("a result");
    assert_ne!(result.is_error, Some(true));
    let record = only_record(&sink);
    assert!(!record.is_error);
    assert_eq!(record.error, None);
}

/// C23: an error recorded with `capture_error` replaces the JSON-RPC code name of an
/// `Err(ErrorData)` (R09), which still reaches the client unchanged; a panic after it gives
/// `type: "panic"`. The handler calls `capture_error` before it returns its future, so the SDK's
/// scope covers that part of `call_tool` too.
#[tokio::test]
async fn captured_error_wins_over_error_data() {
    let plain = connect(Eager, "plain").await;
    let Err(ServiceError::McpError(expected)) = call(&plain, "data", json!({})).await else {
        panic!("a JSON-RPC error");
    };

    let (agentcat, sink) = recording();
    let client = connect(agentcat.track(Eager), "tracked").await;
    let Err(ServiceError::McpError(actual)) = call(&client, "data", json!({})).await else {
        panic!("a JSON-RPC error");
    };
    assert_eq!(
        serde_json::to_value(&actual).unwrap(),
        serde_json::to_value(&expected).unwrap()
    );
    let record = only_record(&sink);
    assert!(record.is_error);
    assert_eq!(record.response, None);
    assert_eq!(captured_error(record), wrapper_shape(), "not InternalError");

    unanswered(&client, "capture_then_panic").await;
    let record = only_record(&sink);
    assert_eq!(record.error, Some(panic_shape("call_tool exploded")));
    assert!(
        record.backtrace.is_none(),
        "a panic keeps no captured backtrace"
    );
}

/// Controller ruling (Task 8): a `list_tools` that panics in the registry rebuild is a failed
/// rebuild (B-027): the warning carries the panic's message, nothing is recorded, so the next call
/// tries again, and the call goes on with TS's heuristic strip (B-051).
#[tokio::test]
async fn rebuild_panic_is_a_failed_rebuild() {
    let (agentcat, sink) = recording();
    let client = connect(agentcat.track(PanickingList), "rebuild").await;
    for _ in 0..2 {
        let arguments = json!({"x": "1", "session_id": "start", "context": "why"});
        let result = call(&client, "anything", arguments)
            .await
            .expect("a result");
        assert_eq!(received(&result), json!({"x": "1"}), "B-051");
    }
    assert_eq!(sink.logs(), [LIST_PANICKED, LIST_PANICKED]);
    let listed = agentcat
        .inner()
        .with_registry::<PanickingList, _>(|registry| registry.listed());
    assert!(!listed, "the failed rebuild recorded nothing");
    let records = sink.take_records();
    assert_eq!(records.len(), 2);
    assert!(records.iter().all(|record| record.error.is_none()));
}

/// Controller ruling (Task 8): a `get_tool` that panics gives no schema (C03), so the call takes
/// the registry's entry after a rebuild: `beta` keeps its own `context`, which the heuristic strip
/// would drop. Each panic is logged with its message (C30).
#[tokio::test]
async fn get_tool_panic_is_no_schema() {
    let (agentcat, sink) = recording();
    let client = connect(agentcat.track(PanickingGetTool), "get-tool").await;
    for _ in 0..2 {
        let arguments = json!({"x": "1", "context": "mine", "session_id": "start"});
        let result = call(&client, "beta", arguments).await.expect("a result");
        assert_eq!(received(&result), json!({"x": "1", "context": "mine"}));
    }
    let listed = agentcat
        .inner()
        .with_registry::<PanickingGetTool, _>(|registry| registry.listed());
    assert!(listed, "the rebuild recorded the listing");
    let records = sink.take_records();
    assert_eq!(records.len(), 2);
    assert!(records.iter().all(|record| record.error.is_none()));
    let logs = sink.logs();
    let lookups: Vec<&String> = logs.iter().filter(|line| line.contains("lookup")).collect();
    assert_eq!(lookups, [LOOKUP_PANICKED, LOOKUP_PANICKED], "{logs:?}");
    assert_eq!(
        logs.first().map(String::as_str),
        Some(LOOKUP_PANICKED),
        "{logs:?}"
    );
    assert_eq!(
        logs.last().map(String::as_str),
        Some(LOOKUP_PANICKED),
        "{logs:?}"
    );
}

/// Controller ruling (Task 8): a `get_info` that panics leaves the record without a server name
/// and version (B-086), and the panic is logged with its message (C30); the call goes on.
#[tokio::test]
async fn get_info_panic_records_no_server() {
    let (agentcat, sink) = recording();
    let client = connect(agentcat.track(PanickingInfo), "get-info").await;
    let result = call(&client, "anything", json!({"x": "1"}))
        .await
        .expect("a result");
    assert_eq!(received(&result), json!({"x": "1"}));
    let record = only_record(&sink);
    assert_eq!(record.server_name, None);
    assert_eq!(record.server_version, None);
    assert!(record.error.is_none());
    let logs = sink.logs();
    let infos: Vec<&String> = logs
        .iter()
        .filter(|line| line.contains("get_info"))
        .collect();
    assert_eq!(infos, [INFO_PANICKED], "{logs:?}");
}

/// R27: `capture_error` takes an unsized `dyn Error`, such as the content of a `Box<dyn Error>`;
/// the record has its message, and the `dyn` type's name as its type.
#[tokio::test]
async fn capture_error_accepts_dyn_error() {
    let (agentcat, sink) = recording();
    let client = connect(agentcat.track(Capturing), "dyn").await;
    call(&client, "captured_dyn", json!({}))
        .await
        .expect("a result");
    let error = only_record(&sink).error.expect("the record has an error");
    assert_eq!(error["message"], json!("root cause"));
    assert_eq!(error["type"], json!("dyn core::error::Error"));
    assert_eq!(error["platform"], json!("rust"));
}

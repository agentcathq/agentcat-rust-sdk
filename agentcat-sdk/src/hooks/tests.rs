//! The hooks (B-022 step 7, B-026, B-064): what a [`ToolCall`] holds, the order in which the hooks
//! start, that they start before the tool runs and never delay the call, that a panic stays in its
//! task, and what hook mode changes for the listing, the resolution and the mirror. The TS
//! behavior is the contract; the settling of the hooks and the event body are plan 4's.
use std::future::{Future, pending, ready};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, InitializeResult, ServerCapabilities,
};
use rmcp::service::{MaybeSendFuture, RequestContext, RoleClient, RunningService};
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde_json::{Map, Value, json};

use super::{ToolCall, fire};
use crate::client::AgentCat;
use crate::config::{HookError, Options};
use crate::event::CallRecord;
use crate::handles::SessionSource;
use crate::sink::Sink;
use crate::test_support::rmcp::{
    CREDENTIAL_HEADERS, RecordingSink, Sample, assert_credentials_hidden, bare_context, connect,
    credential_parts,
};

type Client = RunningService<RoleClient, rmcp::model::InitializeRequestParams>;

/// The labels that hooks and tools push, in the order they happen.
type Log = Arc<Mutex<Vec<String>>>;

fn push(log: &Log, label: &str) {
    log.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(label.to_owned());
}

fn labels(log: &Log) -> Vec<String> {
    log.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

/// A hook that pushes `label` when it is first polled and gives nothing.
fn labelled<T: Send + 'static>(
    log: &Log,
    label: &'static str,
) -> impl Fn(ToolCall) -> std::future::Ready<Result<Option<T>, HookError>> + Send + Sync + 'static {
    let log = Arc::clone(log);
    move |_call| {
        push(&log, label);
        ready(Ok(None))
    }
}

/// Serves `Sample` tracked with `options`; the records go to the returned sink.
async fn serve(options: Options) -> (Client, Arc<RecordingSink>) {
    let sink = Arc::new(RecordingSink::default());
    let agentcat = AgentCat::with_sink("", options, sink.clone());
    (connect(agentcat.track(Sample), "hook-client").await, sink)
}

fn params(name: &str, arguments: Value) -> CallToolRequestParams {
    let Value::Object(arguments) = arguments else {
        panic!("arguments must be an object");
    };
    CallToolRequestParams::new(name.to_owned()).with_arguments(arguments)
}

async fn call(client: &Client, name: &str, arguments: Value) -> CallToolResult {
    client
        .call_tool(params(name, arguments))
        .await
        .unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn only_record(sink: &RecordingSink) -> CallRecord {
    let mut records = sink.take_records();
    assert_eq!(records.len(), 1, "one record: {records:?}");
    records.remove(0)
}

fn texts(result: &CallToolResult) -> Vec<&str> {
    result
        .content
        .iter()
        .filter_map(|block| block.as_text().map(|text| text.text.as_str()))
        .collect()
}

#[tokio::test(flavor = "current_thread")]
async fn hooks_receive_raw_arguments() {
    let seen: Arc<Mutex<Vec<ToolCall>>> = Arc::default();
    let options = Options::default().enable_agent_tracking(true).event_tags({
        let seen = Arc::clone(&seen);
        move |call| {
            seen.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(call);
            ready(Ok(None))
        }
    });
    let (client, _sink) = serve(options).await;
    let sent = json!({
        "msg": "hi",
        "session_id": "start",
        "agent_id": "opus|test|a1b2c",
        "context": "Checking the raw arguments."
    });
    call(&client, "echo", sent.clone()).await;

    client.cancel().await.unwrap();
    let seen = seen.lock().unwrap_or_else(PoisonError::into_inner);
    let [tool_call] = seen.as_slice() else {
        panic!("one hook call, got {}", seen.len());
    };
    assert_eq!(tool_call.tool_name(), "echo");
    assert_eq!(
        tool_call.arguments().map(|a| Value::Object(a.clone())),
        Some(sent),
        "injected keys included, in the order sent"
    );
    assert_eq!(tool_call.client_name(), Some("hook-client"));
    assert_eq!(tool_call.client_version(), Some("1.0.0"));
    // rmcp's client puts a progress token in the `_meta` of each request it sends.
    assert!(tool_call.meta().contains_key("progressToken"));
    assert_eq!(tool_call.headers(), None, "no HTTP on an in-memory pipe");
    assert_eq!(tool_call.transport_session_id(), None);
    assert_eq!(tool_call.protocol_version(), None);
    assert!(
        tool_call
            .request_context()
            .extensions
            .get::<crate::CallContext>()
            .is_none(),
        "the snapshot is taken before the CallContext is added"
    );
}

/// D06 keeps the credential headers in the snapshot, and its `Debug` output hides them: the values
/// of `authorization`, `proxy-authorization`, `cookie` and `set-cookie` print as `<redacted>`
/// under their names, other headers print as they are, and the request context prints as a
/// summary (its id, whether HTTP parts are present), not with the raw headers of its parts.
/// Rust-only: TS has no `Debug`.
#[tokio::test(flavor = "current_thread")]
async fn tool_call_debug_hides_credentials() {
    let mut context = bare_context(None);
    context.extensions.insert(credential_parts());
    let call = ToolCall::new(&params("echo", json!({"x": 1})), &context, None);

    let text = format!("{call:?}");
    assert_credentials_hidden(&text);
    assert!(text.contains("<redacted>"), "{text}");
    assert!(text.contains("\"echo\""), "the other fields print: {text}");
    assert!(text.contains("http_parts: true"), "{text}");
    let headers = call.headers().expect("HTTP headers");
    for (name, value) in CREDENTIAL_HEADERS {
        assert_eq!(headers[name], value, "D06: the data keeps {name}");
    }
}

#[tokio::test(flavor = "current_thread")]
async fn hooks_fire_in_ts_order() {
    let log = Log::default();
    // Set in an order that differs from TS's, so that the firing order is not the setting order.
    let options = Options::default()
        .event_properties(labelled(&log, "eventProperties"))
        .event_tags(labelled(&log, "eventTags"))
        .identify(labelled(&log, "identify"))
        .resolve_session_id(labelled(&log, "resolveSessionId"));
    let (client, _sink) = serve(options).await;
    call(&client, "echo", json!({"msg": "hi"})).await;
    assert_eq!(
        labels(&log),
        [
            "resolveSessionId",
            "identify",
            "eventTags",
            "eventProperties"
        ]
    );
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn hooks_fire_in_ts_order_without_hook_mode() {
    let log = Log::default();
    let options = Options::default()
        .event_properties(labelled(&log, "eventProperties"))
        .event_tags(labelled(&log, "eventTags"))
        .identify(labelled(&log, "identify"));
    let (client, _sink) = serve(options).await;
    call(&client, "echo", json!({"msg": "hi"})).await;
    assert_eq!(labels(&log), ["identify", "eventTags", "eventProperties"]);
    client.cancel().await.unwrap();
}

/// A tool whose `call_tool` pushes `tool` to the log.
struct Recorder(Log);

impl ServerHandler for Recorder {
    fn call_tool(
        &self,
        _request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResponse, ErrorData>> + MaybeSendFuture + '_ {
        push(&self.0, "tool");
        ready(Ok(CallToolResult::success(vec![]).into()))
    }

    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
    }
}

#[tokio::test(flavor = "current_thread")]
async fn hooks_fire_before_the_tool_runs() {
    let log = Log::default();
    let options = Options::default()
        .identify(labelled(&log, "identify"))
        .event_tags(labelled(&log, "eventTags"));
    let agentcat = AgentCat::with_sink("", options, Arc::new(RecordingSink::default()));
    let client = connect(agentcat.track(Recorder(Arc::clone(&log))), "c").await;
    call(&client, "anything", json!({})).await;
    assert_eq!(labels(&log), ["identify", "eventTags", "tool"]);
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn hooks_never_delay_the_call() {
    let options = Options::default()
        .identify(|_call| pending::<Result<Option<crate::UserIdentity>, HookError>>())
        .event_tags(|_call| pending::<Result<Option<Map<String, Value>>, HookError>>());
    let (client, sink) = serve(options).await;
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        client.call_tool(params("echo", json!({"msg": "hi"}))),
    )
    .await
    .expect("the call returns while the hooks hang")
    .expect("the call works");
    assert_eq!(texts(&result).last(), Some(&"hi"));

    let record = only_record(&sink);
    let identify = record.pending.identify.as_ref().expect("identify started");
    let tags = record.pending.event_tags.as_ref().expect("tags started");
    assert!(!identify.is_finished() && !tags.is_finished());
    assert!(record.pending.event_properties.is_none());
    identify.abort();
    tags.abort();
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn hook_panic_does_not_reach_the_call() {
    let options = Options::default()
        .identify(
            |_call| -> std::future::Ready<Result<Option<crate::UserIdentity>, HookError>> {
                panic!("identify blew up")
            },
        )
        .event_tags(|call| async move {
            assert!(call.tool_name().is_empty(), "tags blew up");
            Ok(None::<Map<String, Value>>)
        });
    let (client, sink) = serve(options).await;
    for _ in 0..2 {
        let result = call(&client, "echo", json!({"msg": "hi"})).await;
        assert_eq!(
            texts(&result).last(),
            Some(&"hi"),
            "the server keeps serving"
        );
    }
    for record in sink.take_records() {
        let mut pending = record.pending;
        let identify = pending.identify.take().expect("identify started");
        let tags = pending.event_tags.take().expect("tags started");
        assert!(identify.await.unwrap_err().is_panic());
        assert!(tags.await.unwrap_err().is_panic());
    }
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn no_hooks_no_tasks() {
    let (client, sink) = serve(Options::default()).await;
    call(&client, "echo", json!({"msg": "hi"})).await;
    let pending = only_record(&sink).pending;
    assert!(pending.resolve_session_id.is_none());
    assert!(pending.identify.is_none());
    assert!(pending.event_tags.is_none());
    assert!(pending.event_properties.is_none());
    assert!(!pending.any());
    client.cancel().await.unwrap();
}

/// C14: without a runtime a hook does not start, and the sink says so.
#[test]
fn no_runtime_no_start() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let call = runtime.block_on(async {
        let context = bare_context(None);
        ToolCall::new(&params("echo", json!({})), &context, None)
    });
    let options = Options::default()
        .resolve_session_id(|_call| ready(Ok(Some("s".to_owned()))))
        .identify(|_call| ready(Ok(None)))
        .event_tags(|_call| ready(Ok(None)))
        .event_properties(|_call| ready(Ok(None)));
    let sink = RecordingSink::default();
    let pending = fire(&options, &call, true, &sink as &dyn Sink);
    assert!(!pending.any());
    assert_eq!(
        sink.logs(),
        [
            "Warning: resolveSessionId hook not started - no tokio runtime",
            "Warning: identify hook not started - no tokio runtime",
            "Warning: eventTags hook not started - no tokio runtime",
            "Warning: eventProperties hook not started - no tokio runtime",
        ]
    );
}

fn hook_mode_options() -> Options {
    Options::default()
        .enable_agent_tracking(true)
        .resolve_session_id(|_call| ready(Ok(Some("customer-session".to_owned()))))
}

#[tokio::test(flavor = "current_thread")]
async fn hook_mode_listing_has_no_session_id() {
    let (client, _sink) = serve(hook_mode_options()).await;
    let tools = client.list_all_tools().await.unwrap();
    let echo = tools.iter().find(|tool| tool.name == "echo").unwrap();
    let properties = echo.input_schema["properties"].as_object().unwrap();
    assert!(!properties.contains_key("session_id"));
    assert!(properties.contains_key("agent_id"));
    assert!(properties.contains_key("context"));
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn hook_mode_resolution_is_provisional() {
    let (client, sink) = serve(hook_mode_options()).await;
    let result = call(
        &client,
        "echo",
        json!({"msg": "hi", "session_id": "start", "agent_id": "opus|t|abcde"}),
    )
    .await;
    assert_eq!(texts(&result), ["hi"], "no mint-back text");
    let record = only_record(&sink);
    assert_eq!(record.resolution.session_id, "");
    assert_eq!(record.resolution.session_source, SessionSource::Hook);
    assert!(record.resolution.hook_mode);
    assert_eq!(record.tags["agentcat_session_id_source"], "hook");
    client.cancel().await.unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn hook_mode_mirror_has_only_agent_id() {
    let (client, _sink) = serve(hook_mode_options()).await;
    let result = call(
        &client,
        "structured",
        json!({"msg": "hi", "agent_id": "opus|t|abcde"}),
    )
    .await;
    let structured = result.structured_content.expect("structured content");
    assert_eq!(
        structured["mcp_session"],
        json!({"agent_id": "opus|t|abcde"})
    );
    assert_eq!(structured["msg"], "hi");
    client.cancel().await.unwrap();
}

/// B-026: a hook that returns an error gives the call no trace of it, and the error stays in the
/// task for plan 4 to settle.
#[tokio::test(flavor = "current_thread")]
async fn hook_error_does_not_reach_the_call() {
    let options = Options::default().event_properties(|_call| async {
        Err::<Option<Map<String, Value>>, HookError>("properties failed".into())
    });
    let (client, sink) = serve(options).await;
    let result = call(&client, "echo", json!({"msg": "hi"})).await;
    assert_eq!(texts(&result).last(), Some(&"hi"));
    assert_ne!(result.is_error, Some(true));
    let mut pending = only_record(&sink).pending;
    let settled = pending.event_properties.take().expect("started").await;
    let error = settled.expect("no panic").expect_err("the hook failed");
    assert_eq!(error.to_string(), "properties failed");
    client.cancel().await.unwrap();
}

/// B-022: only `tools/call` runs the hooks. `initialize` and `tools/list` run none and record
/// nothing.
#[tokio::test(flavor = "current_thread")]
async fn listing_fires_no_hooks() {
    let log = Log::default();
    let options = Options::default()
        .resolve_session_id(labelled(&log, "resolveSessionId"))
        .identify(labelled(&log, "identify"))
        .event_tags(labelled(&log, "eventTags"))
        .event_properties(labelled(&log, "eventProperties"));
    let (client, sink) = serve(options).await;
    client.list_all_tools().await.unwrap();
    assert_eq!(labels(&log), Vec::<String>::new());
    assert!(sink.take_records().is_empty());
    client.cancel().await.unwrap();
}

//! The TS in-memory tool-call event that [`from_call`] builds from a [`CallRecord`] (B-077): its
//! key order against TS's (`tests/vectors/event_order.json`, case `tool_call`), its fields, and
//! the backtrace of a captured error that [`render_backtrace`] puts in `error.stack`.
use std::backtrace::Backtrace;
use std::time::{Duration, UNIX_EPOCH};

use rmcp::model::{CallToolResult, ContentBlock, InputRequiredResult};
use serde_json::{Value, json};

use super::{from_call, render_backtrace};
use crate::config::UserIdentity;
use crate::event::{CallRecord, ClientIdentity, extra_value};
use crate::handles::{HandleResolution, SessionSource};
use crate::hooks::PendingHooks;
use crate::pipeline::settle::{HookResults, apply};
use crate::pipeline::{EventObject, TOOL_CALL_KEYS};
use crate::test_support::json::object;
use crate::test_support::rmcp::{assert_credentials_hidden, bare_context, credential_parts};
use crate::test_support::vectors;

/// The record of a call that worked, every field set: a supplied session, a client, a server,
/// tokens, an intent and rmcp's result of the tool (with its `resultType: "complete"`).
fn record() -> CallRecord {
    let response = CallToolResult::success(vec![ContentBlock::text("hi")]);
    CallRecord {
        started_at: UNIX_EPOCH + Duration::from_millis(1_700_000_000_250),
        duration_ms: 750,
        resolution: HandleResolution {
            session_id: "ses_1".to_owned(),
            session_source: SessionSource::Supplied,
            hook_mode: false,
            agent_id: None,
        },
        resource_name: "echo".to_owned(),
        request: json!({"method": "tools/call", "params": {"name": "echo"}, "jsonrpc": "2.0", "id": 1}),
        extra: json!({"requestId": 1}),
        input_tokens: Some(5),
        output_tokens: Some(2),
        tags: object(json!({"agentcat_session_id_source": "supplied"})),
        user_intent: Some(json!("why")),
        response: Some(serde_json::to_value(response).expect("a result is JSON")),
        is_error: false,
        error: None,
        server_name: Some("server".to_owned()),
        server_version: Some("1.0.0".to_owned()),
        client: Some(ClientIdentity {
            name: Some("client".to_owned()),
            version: Some("2.0.0".to_owned()),
        }),
        pending: PendingHooks::default(),
        runtime: None,
        backtrace: None,
    }
}

/// The `tool_call` case of `event_order.json`: its event keys, each with whether TS left its
/// value undefined.
fn ts_event_keys() -> Vec<(String, bool)> {
    let cases = vectors::cases("event_order");
    let case = cases
        .iter()
        .find(|case| vectors::name(case) == "tool_call")
        .expect("event_order.json has the tool_call case");
    case["event"]
        .as_array()
        .expect("event is a list")
        .iter()
        .map(|entry| {
            let key = entry["key"].as_str().expect("a key").to_owned();
            (key, entry.get("undefined") == Some(&json!(true)))
        })
        .collect()
}

/// The keys of `event`, in order.
fn keys(event: &EventObject) -> Vec<&str> {
    event.keys().map(String::as_str).collect()
}

/// The keys of TS's tool-call event, in TS's order (`event_order.json`). TS's probe ran a call
/// that worked, with an intent and the identify, eventTags and eventProperties hooks: after the
/// hooks apply, Rust's event of such a call has the keys TS's has, in TS's order (`ipAddress`,
/// `isError` and `error` are undefined in TS, so JSON has none). A failed call has `isError` and
/// `error` at TS's places.
#[test]
fn tool_call_event_follows_ts_key_order() {
    let ts = ts_event_keys();
    let all: Vec<&str> = ts.iter().map(|(key, _)| key.as_str()).collect();
    assert_eq!(TOOL_CALL_KEYS.as_slice(), all.as_slice());
    let defined: Vec<&str> = ts
        .iter()
        .filter(|(_, undefined)| !undefined)
        .map(|(key, _)| key.as_str())
        .collect();

    let mut job = from_call(record(), "proj_1");
    let results = HookResults {
        session_value: None,
        identity: Some(
            UserIdentity::new("user-1")
                .user_name("User One")
                .user_data(object(json!({"plan": "pro"}))),
        ),
        tags: Some(object(json!({"env": "probe"}))),
        properties: Some(object(json!({"device": "probe"}))),
    };
    apply(&mut job.event, results, false).expect("no session step");
    assert_eq!(keys(&job.event), defined);

    let mut failed = record();
    failed.is_error = true;
    failed.error = Some(object(json!({"message": "failed", "platform": "rust"})));
    let job = from_call(failed, "proj_1");
    let present: Vec<&str> = all
        .iter()
        .copied()
        .filter(|key| job.event.contains_key(*key))
        .collect();
    assert_eq!(keys(&job.event), present);
    assert_eq!(job.event["isError"], json!(true));
    assert!(job.event.contains_key("error"));
}

/// B-077: the fields of the event, taken from the record, with the SDK's own values: an empty
/// `id`, the `mcp:tools/call` type, `sdkLanguage` `Rust` and the crate version (D01, RD-14), and
/// an empty `identifyActorData`. Both redaction stages are on, and the job takes the hook mode of
/// the record's resolution (the one source of it), the hooks, the runtime and the backtrace.
#[tokio::test]
async fn event_takes_the_record_fields() {
    let mut record = record();
    record.runtime = tokio::runtime::Handle::try_current().ok();
    record.backtrace = Some(Backtrace::force_capture());
    record.resolution.hook_mode = true;
    let job = from_call(record, "proj_1");
    let expected = json!({
        "id": "",
        "sessionId": "ses_1",
        "projectId": "proj_1",
        "eventType": "mcp:tools/call",
        "timestamp": "2023-11-14T22:13:20.250Z",
        "duration": 750,
        "inputTokens": 5,
        "outputTokens": 2,
        "sdkLanguage": "Rust",
        "agentcatVersion": env!("CARGO_PKG_VERSION"),
        "serverName": "server",
        "serverVersion": "1.0.0",
        "clientName": "client",
        "clientVersion": "2.0.0",
        "identifyActorData": {},
        "resourceName": "echo",
        "parameters": {
            "request": {"method": "tools/call", "params": {"name": "echo"}, "jsonrpc": "2.0", "id": 1},
            "extra": {"requestId": 1},
        },
        "response": {"content": [{"type": "text", "text": "hi"}], "isError": false},
        "userIntent": "why",
        "tags": {"agentcat_session_id_source": "supplied"},
    });
    assert_eq!(Value::Object(job.event.clone()), expected);
    assert!(job.hook_mode);
    assert!(job.string_redaction && job.event_redaction);
    assert!(job.runtime.is_some());
    assert!(job.backtrace.is_some());
    assert!(!job.hooks.any());
}

/// A field the record does not have is an absent key (TS's `undefined`, which JSON drops); the
/// client fields come from the client's name and version separately.
#[test]
fn absent_fields_are_absent_keys() {
    let mut record = record();
    record.input_tokens = None;
    record.output_tokens = None;
    record.user_intent = None;
    record.response = None;
    record.server_name = None;
    record.server_version = None;
    record.client = Some(ClientIdentity {
        name: None,
        version: Some("2.0.0".to_owned()),
    });
    let job = from_call(record, "");
    for key in [
        "inputTokens",
        "outputTokens",
        "userIntent",
        "response",
        "serverName",
        "serverVersion",
        "clientName",
        "isError",
        "error",
        "identifyActorGivenId",
        "identifyActorName",
        "properties",
    ] {
        assert!(!job.event.contains_key(key), "{key} is present");
    }
    assert_eq!(job.event["clientVersion"], json!("2.0.0"));
    assert_eq!(job.event["projectId"], json!(""));
}

/// B-077: `timestamp` is the start of the call as `toISOString` writes it, three millisecond
/// digits included when they are zero.
#[test]
fn event_timestamp_is_iso_millis() {
    let mut record = record();
    record.started_at = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let job = from_call(record, "proj_1");
    assert_eq!(job.event["timestamp"], json!("2023-11-14T22:13:20.000Z"));
}

/// rmcp writes `resultType: "complete"` into a tool's result; the customer's result in a TS event
/// has none, so the event drops it. An `InputRequiredResult` keeps its `input_required` type, as
/// the customer's result in TS carries it (B-025).
#[test]
fn response_has_no_result_type() {
    let job = from_call(record(), "proj_1");
    let response = job.event["response"].as_object().expect("a response");
    assert!(!response.contains_key("resultType"), "{response:?}");
    assert_eq!(response["content"], json!([{"type": "text", "text": "hi"}]));

    let mut record = record();
    let input_required = InputRequiredResult::from_request_state("state-1");
    record.response = Some(serde_json::to_value(input_required).expect("a result is JSON"));
    let job = from_call(record, "proj_1");
    assert_eq!(job.event["response"]["resultType"], json!("input_required"));
}

/// B-077: a call without a session (an invalid or foreign value, or hook mode before the hook
/// settles) has the session `""`, which the wire sends as `null`.
#[test]
fn sessionless_call_has_empty_session() {
    let mut record = record();
    record.resolution.session_id = String::new();
    record.resolution.session_source = SessionSource::Invalid;
    let job = from_call(record, "proj_1");
    assert_eq!(job.event["sessionId"], json!(""));
}

/// RD-06: the backtrace of a captured error becomes `error.stack`, after `platform` and before
/// `chained_errors`, as `capture_error` used to write it; a backtrace that was not captured adds
/// nothing.
#[test]
fn backtrace_renders_after_platform() {
    let error = json!({
        "message": "failed",
        "type": "my::Error",
        "platform": "rust",
        "chained_errors": [{"message": "root cause"}],
    });
    let mut event = object(json!({"isError": true, "error": error.clone(), "tags": {}}));
    let backtrace = Backtrace::force_capture();
    render_backtrace(&mut event, &backtrace);
    let rendered = event["error"].as_object().expect("an error");
    assert_eq!(
        keys(rendered),
        ["message", "type", "platform", "stack", "chained_errors"]
    );
    assert_eq!(rendered["stack"], json!(backtrace.to_string()));

    let mut event = object(json!({"error": error.clone()}));
    render_backtrace(&mut event, &Backtrace::disabled());
    assert_eq!(event["error"], error);
}

/// The job's `Debug` output hides the credential headers of `extra`, as the record's does.
#[tokio::test]
async fn event_job_debug_hides_credentials() {
    let mut context = bare_context(None);
    context.extensions.insert(credential_parts());
    let mut record = record();
    record.extra = extra_value(&context);
    let job = from_call(record, "proj_1");
    let text = format!("{job:?}");
    assert_credentials_hidden(&text);
    assert!(text.contains("echo"), "the other fields print: {text}");
}

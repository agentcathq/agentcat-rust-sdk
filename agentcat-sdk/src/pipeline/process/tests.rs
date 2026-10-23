//! The per-event pipeline (B-087) against TS: every `queue.json` case but the `retry:*` ones
//! with each case's hooks as finished or hanging tasks under a 50 ms budget, and the `retry:*`
//! ones through the delivery worker; the Rust-only paths of the customer's redaction hooks (C38: the
//! runtime they run on, RD-36 without one, a panic); [`assign`]; and tool calls served by rmcp,
//! whose records go through `from_call` and `process` as TS's e2e redaction, sanitization,
//! truncation and error-capture tests publish them.
use std::future::{Future, ready};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, UNIX_EPOCH};

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, InitializeResult,
    ServerCapabilities,
};
use rmcp::service::{MaybeSendFuture, RequestContext};
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde_json::{Map, Value, json};
use tokio::runtime::{Builder, Handle};

use super::{Redactors, assign, process};
use crate::client::AgentCat;
use crate::config::{HookError, Options, UserIdentity};
use crate::delivery::wire::publish_request;
use crate::delivery::{Delivery, DeliveryConfig};
use crate::event::Event;
use crate::event::assemble::{EventJob, from_call};
use crate::handles::SessionIdFailure;
use crate::hooks::PendingHooks;
use crate::js;
use crate::pipeline::TsEvent;
use crate::pipeline::truncate::MAX_EVENT_BYTES;
use crate::sink::Sink;
use crate::test_support::rmcp::{RecordingSink, Sample, connect};
use crate::test_support::{assert_js_eq, queue, vectors};

/// The hook budget of every test.
const BUDGET: Duration = Duration::from_millis(50);

/// The JSON object `value`.
fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(object) => object,
        other => panic!("{other} is not an object"),
    }
}

/// A tool-call event as `from_call` builds it, whose strings hold `secret` outside the protected
/// fields.
fn call_event() -> TsEvent {
    object(json!({
        "id": "",
        "sessionId": "ses_queue0000000000000000000000",
        "projectId": "proj_queue",
        "eventType": "mcp:tools/call",
        "timestamp": "2026-10-23T11:59:59.250Z",
        "duration": 750,
        "sdkLanguage": "Rust",
        "identifyActorData": {},
        "resourceName": "echo",
        "parameters": {"request": {"params": {"name": "echo", "arguments": {"msg": "a secret"}}}},
        "response": {"content": [{"type": "text", "text": "the secret"}]},
        "userIntent": "find the secret",
        "tags": {"agentcat_session_id_source": "supplied"},
    }))
}

/// The job of `event` with no hooks, both redaction stages on and the redaction hooks' runtime
/// `runtime`.
fn job(event: TsEvent, runtime: Option<Handle>) -> EventJob {
    EventJob {
        event,
        hooks: PendingHooks::default(),
        hook_mode: false,
        runtime,
        string_redaction: true,
        event_redaction: true,
        backtrace: None,
    }
}

/// The string-redaction hook `hook` alone.
fn string_hook<F, Fut>(hook: F) -> Redactors
where
    F: Fn(String) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<String, HookError>> + Send + 'static,
{
    let options = Options::default().redact_sensitive_information(hook);
    Redactors {
        string: options.string_redactor().cloned(),
        event: None,
    }
}

/// The event-level redaction hook `hook` alone.
fn event_hook<F, Fut>(hook: F) -> Redactors
where
    F: Fn(Event) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Option<Event>, HookError>> + Send + 'static,
{
    let options = Options::default().redact_event(hook);
    Redactors {
        string: None,
        event: options.event_redactor().cloned(),
    }
}

/// The value at `pointer` (RFC 6901) of `event`.
fn at<'a>(event: &'a TsEvent, pointer: &str) -> &'a Value {
    let (key, rest) = pointer[1..].split_once('/').unwrap_or((&pointer[1..], ""));
    let value = &event[key];
    if rest.is_empty() {
        value
    } else {
        value
            .pointer(&format!("/{rest}"))
            .unwrap_or_else(|| panic!("no {pointer} in {event:?}"))
    }
}

// ------------------------------------------------------------------------------- queue.json

/// Whether the delivery worker posts `event`: TS's `sendEvent` posts only an event with a project
/// (B-104).
fn posted(event: &TsEvent) -> bool {
    event
        .get("projectId")
        .and_then(Value::as_str)
        .is_some_and(|project| !project.is_empty())
}

/// Each `queue.json` case but `retry:*`: the event the queue takes goes through `process` with the
/// case's hooks (tasks that return, throw or hang under [`BUDGET`]) and redaction hooks, on the
/// test runtime; the lines `process` writes and the request of what it gives must be TS's, as
/// `test_support::queue` compares them (the delivery lines and a custom event's builder lines
/// left out, TS's `Error: M` as `M`, `<evt>` and `<minted>` masks, RD-33).
#[tokio::test]
async fn queue_vectors_match_ts() {
    let mut ran = Vec::new();
    for case in queue::cases() {
        let name = vectors::name(&case).to_owned();
        if name.starts_with("retry:") {
            continue;
        }
        let job = queue::job_of(&case, Some(&Handle::current()));
        let mut log = Vec::new();
        let published = process(job, &queue::redactors_of(&case), BUDGET, &mut log).await;
        assert_eq!(log, queue::expected_logs(&case), "{name}: logs");

        let requests: Vec<Value> = published
            .iter()
            .filter(|event| posted(event))
            .map(|event| queue::to_ts_model(&publish_request(event)))
            .collect();
        let expected = queue::expected_requests(&case);
        assert_eq!(requests.len(), expected.len(), "{name}: {requests:?}");
        for (actual, expected) in requests.iter().zip(&expected) {
            assert_js_eq(actual, expected, &name);
        }
        ran.push(name);
    }
    assert_eq!(ran.len(), 56, "every non-retry case ran: {ran:?}");
}

/// The shrunk retry delays of [`queue_retry_vectors_match_ts`]: the real 1, 2 and 4 s would break
/// the 3 s limit of a test (ruling P1).
const SHRUNK_RETRY_DELAYS: [Duration; 3] = [
    Duration::from_millis(1),
    Duration::from_millis(2),
    Duration::from_millis(4),
];

/// The `retry:*` cases of `queue.json` through the delivery worker (ruling P1): a publisher fails
/// the case's first `fail_attempts` attempts with `mock failure`. Every attempt's request must be
/// TS's (`<evt>` masks), so the attempt count is TS's; the worker's lines must be TS's after
/// [`queue::worker_logs`] (TS's JS error rendering as the message, ruling P7) and with each
/// `evt_` ID as `<evt>`. The delays TS waited before each retry must be the default
/// `retry_delays`; the worker runs with [`SHRUNK_RETRY_DELAYS`].
#[test]
fn queue_retry_vectors_match_ts() {
    let defaults: Vec<u128> = DeliveryConfig::default()
        .retry_delays
        .iter()
        .map(Duration::as_millis)
        .collect();
    let mut ran = Vec::new();
    for case in queue::cases() {
        let name = vectors::name(&case).to_owned();
        if !name.starts_with("retry:") {
            continue;
        }
        let delays: Vec<u128> = case["delays"]
            .as_array()
            .expect("delays")
            .iter()
            .map(|delay| u128::from(delay.as_u64().expect("milliseconds")))
            .collect();
        assert_eq!(delays, defaults[..delays.len()], "{name}: delays");

        let failures = usize::try_from(case["fail_attempts"].as_u64().expect("a count")).unwrap();
        let publisher = queue::RecordingPublisher::failing(failures);
        let lines = queue::CapturedLines::default();
        let config = DeliveryConfig {
            retry_delays: SHRUNK_RETRY_DELAYS,
            ..DeliveryConfig::default()
        };
        let delivery = Delivery::with_writer(
            String::new(),
            &Options::default(),
            Arc::new(publisher.clone()),
            None,
            config,
            lines.writer(),
        );
        delivery.event(queue::job_of(&case, None));
        assert!(delivery.flush_blocking(Duration::from_secs(2)), "{name}");
        drop(delivery);

        let requests: Vec<Value> = publisher
            .requests()
            .iter()
            .map(queue::to_ts_model)
            .collect();
        let expected = queue::expected_requests(&case);
        assert_eq!(requests.len(), expected.len(), "{name}: attempts");
        for (actual, expected) in requests.iter().zip(&expected) {
            assert_js_eq(actual, expected, &name);
        }
        let logs: Vec<String> = lines
            .lines()
            .iter()
            .map(|line| queue::mask_event_ids(line))
            .collect();
        assert_eq!(logs, queue::worker_logs(&case), "{name}: logs");
        ran.push(name);
    }
    assert_eq!(ran, ["retry:2-failures", "retry:4-failures"]);
}

// ---------------------------------------------------------------------------------- assign

/// Carry-forward (plan 2 final review): TS merges each stage's result into the event with
/// `Object.assign`, so a key the stage's result lacks stays, a key it has keeps its place, and a
/// new key goes last.
#[test]
fn assign_keeps_keys_the_stage_dropped() {
    let mut event = object(json!({
        "id": "",
        "response": {"text": "long"},
        "error": {"message": "failed"},
        "tags": {},
    }));
    assign(
        &mut event,
        object(json!({"tags": {"a": "b"}, "response": {"text": "cut"}, "extra": true})),
    );
    assert_eq!(
        js::stringify(&Value::Object(event)),
        r#"{"id":"","response":{"text":"cut"},"error":{"message":"failed"},"tags":{"a":"b"},"extra":true}"#
    );
}

// ------------------------------------------------------------------------ redaction hooks

/// RD-36, C38: without a runtime for the customer's hooks, an event with a redaction hook is
/// dropped with TS's stage line and the text `no tokio runtime`; the hook never runs.
#[tokio::test]
async fn redaction_without_runtime_drops_event() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    let event_hooks = event_hook(move |event| {
        counted.fetch_add(1, Ordering::SeqCst);
        ready(Ok(Some(event)))
    });
    let counted = Arc::clone(&calls);
    let string_hooks = string_hook(move |text| {
        counted.fetch_add(1, Ordering::SeqCst);
        ready(Ok(text))
    });

    let mut log = Vec::new();
    let published = process(job(call_event(), None), &event_hooks, BUDGET, &mut log).await;
    assert_eq!(published, None);
    assert_eq!(
        log,
        ["Failed to redact event (event-level hook): no tokio runtime"]
    );

    let mut log = Vec::new();
    let published = process(job(call_event(), None), &string_hooks, BUDGET, &mut log).await;
    assert_eq!(published, None);
    assert_eq!(log, ["Failed to redact event: no tokio runtime"]);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

/// C38: when the task of a redaction hook panics, the event is dropped with TS's stage line and
/// the panic's message (the process panic hook prints the panic, as for any task).
#[tokio::test]
async fn string_redactor_panic_drops_event() {
    let string_hooks = string_hook(|text: String| async move {
        assert!(!text.contains("secret"), "string redactor exploded");
        Ok(text)
    });
    let mut log = Vec::new();
    let runtime = Some(Handle::current());
    let published = process(job(call_event(), runtime), &string_hooks, BUDGET, &mut log).await;
    assert_eq!(published, None);
    assert_eq!(log, ["Failed to redact event: string redactor exploded"]);

    let event_hooks = event_hook(|event: Event| async move {
        assert!(!event.id.is_empty(), "event redactor exploded");
        Ok(Some(event))
    });
    let mut log = Vec::new();
    let runtime = Some(Handle::current());
    let published = process(job(call_event(), runtime), &event_hooks, BUDGET, &mut log).await;
    assert_eq!(published, None);
    assert_eq!(
        log,
        ["Failed to redact event (event-level hook): event redactor exploded"]
    );
}

/// C38: both redaction hooks run on the job's runtime (the customer's), not on the runtime that
/// runs `process` (the delivery worker's); the string hook runs once per string, in walk order.
#[test]
fn string_redactor_runs_on_customer_runtime() {
    let customer = Builder::new_multi_thread()
        .worker_threads(1)
        .build()
        .unwrap();
    let worker = Builder::new_current_thread().enable_time().build().unwrap();
    let seen: Arc<Mutex<Vec<(String, tokio::runtime::Id)>>> = Arc::default();
    let record = |seen: &Arc<Mutex<Vec<(String, tokio::runtime::Id)>>>, what: String| {
        seen.lock().unwrap().push((what, Handle::current().id()));
    };
    let string_seen = Arc::clone(&seen);
    let event_seen = Arc::clone(&seen);
    let redactors = Redactors {
        string: string_hook(move |text: String| {
            record(&string_seen, text.clone());
            ready(Ok(text))
        })
        .string,
        event: event_hook(move |event: Event| {
            record(&event_seen, "<event>".to_owned());
            ready(Ok(Some(event)))
        })
        .event,
    };

    let mut log = Vec::new();
    let job = job(call_event(), Some(customer.handle().clone()));
    let published = worker.block_on(process(job, &redactors, BUDGET, &mut log));
    assert!(published.is_some(), "{log:?}");
    let seen = seen.lock().unwrap().clone();
    let texts: Vec<&str> = seen.iter().map(|(text, _)| text.as_str()).collect();
    assert_eq!(
        texts,
        [
            "<event>",
            "Rust",
            "echo",
            "a secret",
            "text",
            "the secret",
            "find the secret"
        ]
    );
    assert_ne!(customer.handle().id(), worker.handle().id());
    for (text, id) in &seen {
        assert_eq!(*id, customer.handle().id(), "{text} ran on another runtime");
    }
}

/// Ruling P14 (RD-35's reason): a `process` future dropped while the string hook runs, as the
/// delivery worker drops a job it abandons at a shutdown deadline, aborts the hook's task on the
/// customer's runtime instead of leaving it running detached.
#[test]
fn dropped_process_aborts_redaction_task() {
    let customer = Builder::new_multi_thread()
        .worker_threads(1)
        .build()
        .unwrap();
    let worker = Builder::new_current_thread().enable_time().build().unwrap();
    let (started, started_rx) = tokio::sync::oneshot::channel::<()>();
    let (signal, dropped) = queue::DropSignal::new();
    let hook_state = Arc::new(Mutex::new(Some((started, signal))));
    let redactors = string_hook(move |text: String| {
        let state = hook_state.lock().unwrap().take();
        async move {
            // The first call holds the signal and never returns, so only an abort drops it.
            let (started, _signal) = state.expect("one call: the first never returns");
            started.send(()).unwrap();
            std::future::pending::<()>().await;
            Ok(text)
        }
    });

    let job = job(call_event(), Some(customer.handle().clone()));
    let mut log = Vec::new();
    let outcome = worker.block_on(async {
        tokio::select! {
            _ = process(job, &redactors, BUDGET, &mut log) => "finished",
            _ = started_rx => "the hook started",
        }
    });
    assert_eq!(outcome, "the hook started");
    dropped
        .recv_timeout(Duration::from_secs(2))
        .expect("dropping process aborts the redaction task");
}

/// B-088, C39: the event-level hook cannot change the system fields: the ID, the session, the
/// project, the event type and the timestamp come back from the event it was given; its other
/// changes stay.
#[tokio::test]
async fn event_hook_cannot_change_restored_fields() {
    let mut original = call_event();
    original.insert("id".to_owned(), json!("evt_from_the_call"));
    let hooks = event_hook(|event: Event| {
        ready(Ok(Some(Event {
            id: "evt_from_the_hook".to_owned(),
            session_id: None,
            project_id: "proj_other".to_owned(),
            event_type: "agentcat:custom".to_owned(),
            timestamp: UNIX_EPOCH,
            resource_name: Some("renamed".to_owned()),
            ..event
        })))
    });
    let mut log = Vec::new();
    let runtime = Some(Handle::current());
    let published = process(job(original, runtime), &hooks, BUDGET, &mut log)
        .await
        .expect("the event stays");
    assert_eq!(log, Vec::<String>::new());
    assert_eq!(published["id"], json!("evt_from_the_call"));
    assert_eq!(
        published["sessionId"],
        json!("ses_queue0000000000000000000000")
    );
    assert_eq!(published["projectId"], json!("proj_queue"));
    assert_eq!(published["eventType"], json!("mcp:tools/call"));
    assert_eq!(published["timestamp"], json!("2026-10-23T11:59:59.250Z"));
    assert_eq!(published["resourceName"], json!("renamed"));
}

/// K-03: an event without the string stage (a custom event) keeps its strings, and the
/// string-redaction hook never runs on it; without the event stage, the event hook does not run
/// either.
#[tokio::test]
async fn custom_events_skip_string_redaction() {
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&calls);
    let mut redactors = string_hook(move |text: String| {
        counted.fetch_add(1, Ordering::SeqCst);
        ready(Ok(text.replace("secret", "[REDACTED]")))
    });
    let counted = Arc::clone(&calls);
    redactors.event = event_hook(move |_event| {
        counted.fetch_add(1, Ordering::SeqCst);
        ready(Ok(None))
    })
    .event;

    let mut event = call_event();
    event.insert("eventType".to_owned(), json!("agentcat:custom"));
    let mut custom = job(event, Some(Handle::current()));
    custom.string_redaction = false;
    custom.event_redaction = false;
    let mut log = Vec::new();
    let published = process(custom, &redactors, BUDGET, &mut log)
        .await
        .expect("the event stays");
    assert_eq!(log, Vec::<String>::new());
    assert_eq!(
        at(&published, "/parameters/request/params/arguments/msg"),
        "a secret"
    );
    assert_eq!(at(&published, "/response/content/0/text"), "the secret");
    assert_eq!(published["userIntent"], json!("find the secret"));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

/// TS's `catch` around `applyPendingEventFields`: when a hook-mode event cannot get a minted
/// session, the line says why and the event goes on without a session.
#[tokio::test]
async fn session_mint_failure_logs_and_continues() {
    let _failure = SessionIdFailure::start();
    let mut event = call_event();
    event.insert("sessionId".to_owned(), json!(""));
    let mut hook_mode = job(event, Some(Handle::current()));
    hook_mode.hook_mode = true;
    let mut log = Vec::new();
    let published = process(hook_mode, &Redactors::default(), BUDGET, &mut log)
        .await
        .expect("the event goes on");
    assert_eq!(
        log,
        ["Failed to resolve pending hook results: could not generate a session ID"]
    );
    assert_eq!(published["sessionId"], json!(""));
    assert!(published["id"].as_str().unwrap().starts_with("evt_"));
}

// ------------------------------------------------------------------------ calls through rmcp

/// The project of the calls through rmcp.
const E2E_PROJECT: &str = "proj_e2e";

/// A PNG image as base64.
const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJ";

/// What `get_attachment` of TS's e2e tests answers.
const IMAGE_NOTE: &str = "[image content redacted - not supported by AgentCat]";

/// A server with the tools of TS's e2e sanitization and truncation tests: `attachment` (a text and
/// an image), `audio_clip` (an audio block), `report` (a text over 50 000 characters),
/// `screenshot` (a text over 50 000 characters and an image) and `add_todo {text}`
/// (`Added todo: <text>`); any other tool answers `done`.
#[derive(Debug, Clone, Copy)]
struct Payloads;

impl ServerHandler for Payloads {
    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResponse, ErrorData>> + MaybeSendFuture + '_ {
        let long = "x".repeat(50_000);
        let content = match request.name.as_ref() {
            "attachment" => vec![
                ContentBlock::text("Here is the attachment:"),
                ContentBlock::image(PNG, "image/png"),
            ],
            "audio_clip" => vec![ContentBlock::audio(
                "UklGRiQAAABXQVZFZm10IBAAAAABAAEA",
                "audio/wav",
            )],
            "report" => vec![ContentBlock::text(format!("Report on sales: {long}"))],
            "screenshot" => vec![
                ContentBlock::text(long),
                ContentBlock::image("iVBORw0KGgo=", "image/png"),
            ],
            "add_todo" => {
                let arguments = request.arguments.unwrap_or_default();
                let text = arguments.get("text").and_then(Value::as_str).unwrap_or("");
                vec![ContentBlock::text(format!("Added todo: {text}"))]
            }
            _ => vec![ContentBlock::text("done")],
        };
        ready(Ok(CallToolResult::success(content).into()))
    }

    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
    }
}

/// The call `name` with `arguments`.
fn params(name: &str, arguments: Value) -> CallToolRequestParams {
    CallToolRequestParams::new(name.to_owned()).with_arguments(object(arguments))
}

/// Serves `server` tracked for [`E2E_PROJECT`] with `options`, makes the calls `calls` in order
/// (a call may fail), and runs each call's record through `from_call` and `process` with the
/// options' hooks, on the test runtime: what `process` gives for each call, and its log lines.
async fn publish_calls<S: ServerHandler>(
    server: S,
    options: Options,
    calls: Vec<(&str, Value)>,
) -> (Vec<Option<TsEvent>>, Vec<String>) {
    let sink = Arc::new(RecordingSink::default());
    let agentcat = AgentCat::with_sink(E2E_PROJECT, options.clone(), sink.clone());
    let client = connect(agentcat.track(server), "e2e-client").await;
    for (name, arguments) in calls {
        let _outcome = client.call_tool(params(name, arguments)).await;
    }
    client.cancel().await.unwrap();

    let redactors = Redactors {
        string: options.string_redactor().cloned(),
        event: options.event_redactor().cloned(),
    };
    let mut log = Vec::new();
    let mut published = Vec::new();
    for record in sink.take_records() {
        let job = from_call(record, E2E_PROJECT);
        published.push(process(job, &redactors, Duration::from_secs(1), &mut log).await);
    }
    (published, log)
}

/// The events of `published`, each one there.
fn all(published: Vec<Option<TsEvent>>) -> Vec<TsEvent> {
    published
        .into_iter()
        .map(|event| event.expect("an event"))
        .collect()
}

/// TS's e2e sanitization tests (`e2e-sanitization.test.ts`, `v2/pipeline.test.ts`): an image and
/// an audio block of a tool's response become TS's notes, a text block stays, and a call without
/// such blocks keeps its response.
#[tokio::test]
async fn tool_call_media_blocks_are_sanitized() {
    let calls = vec![
        ("attachment", json!({"id": "att_1"})),
        ("audio_clip", json!({"clipId": "clip_1"})),
        ("add_todo", json!({"text": "Buy groceries"})),
    ];
    let (published, log) = publish_calls(Payloads, Options::default(), calls).await;
    assert_eq!(log, Vec::<String>::new());
    let events = all(published);
    assert_eq!(
        events[0]["response"]["content"],
        json!([
            {"type": "text", "text": "Here is the attachment:"},
            {"type": "text", "text": IMAGE_NOTE},
        ])
    );
    assert_eq!(
        events[1]["response"]["content"],
        json!([{"type": "text", "text": "[audio content redacted - not supported by AgentCat]"}])
    );
    assert_eq!(
        events[2]["response"]["content"],
        json!([{"type": "text", "text": "Added todo: Buy groceries"}])
    );
}

/// TS's e2e sanitization test of a large base64 argument: it becomes TS's note, and the other
/// arguments stay.
#[tokio::test]
async fn tool_call_base64_argument_is_sanitized() {
    let data = format!("{}=", "A".repeat(12_000));
    let calls = vec![("upload", json!({"filename": "photo.png", "data": data}))];
    let (published, _log) = publish_calls(Payloads, Options::default(), calls).await;
    let events = all(published);
    let arguments = at(&events[0], "/parameters/request/params/arguments");
    assert_eq!(
        arguments["data"],
        json!("[binary data redacted - not supported by AgentCat]")
    );
    assert_eq!(arguments["filename"], json!("photo.png"));
}

/// TS's e2e truncation tests (`e2e-truncation.test.ts`): a response text over 32 768 units is cut
/// to at most 32 768 units and `...`, an image of the same response is sanitized, the event fits
/// in 100 KB, and a small response stays whole.
#[tokio::test]
async fn tool_call_long_text_is_truncated() {
    let calls = vec![
        ("report", json!({"topic": "quarterly sales"})),
        ("screenshot", json!({"page": "dashboard"})),
        ("add_todo", json!({"text": "Write tests"})),
    ];
    let (published, _log) = publish_calls(Payloads, Options::default(), calls).await;
    let events = all(published);
    for event in &events[..2] {
        let text = at(event, "/response/content/0/text").as_str().unwrap();
        assert!(js::utf16_len(text) <= 32_768 + 3, "{}", text.len());
        assert!(text.ends_with("..."));
        let bytes = js::stringify(&Value::Object(event.clone())).len();
        assert!(bytes <= MAX_EVENT_BYTES, "{bytes} bytes");
    }
    assert_eq!(
        at(&events[1], "/response/content/1"),
        &json!({"type": "text", "text": IMAGE_NOTE})
    );
    assert_eq!(
        at(&events[2], "/response/content/0/text"),
        "Added todo: Write tests"
    );
}

/// TS's e2e truncation test of oversized arguments: the event's `JSON.stringify` text stays within
/// 100 KB.
#[tokio::test]
async fn tool_call_oversized_arguments_fit_the_size_guard() {
    let arguments = json!({"dataset": "d".repeat(80_000), "metadata": "m".repeat(40_000)});
    let calls = vec![("bulk", arguments)];
    let (published, _log) = publish_calls(Payloads, Options::default(), calls).await;
    let events = all(published);
    let bytes = js::stringify(&Value::Object(events[0].clone())).len();
    assert!(bytes <= MAX_EVENT_BYTES, "{bytes} bytes");
}

/// The string redactor of TS's redaction integration tests: an e-mail address and an `id` become
/// marks.
fn mark_sensitive(text: String) -> std::future::Ready<Result<String, HookError>> {
    let text = if text.contains('@') {
        "[REDACTED-EMAIL]".to_owned()
    } else if text.to_lowercase().contains("id") {
        "[REDACTED-ID]".to_owned()
    } else {
        text
    };
    ready(Ok(text))
}

/// TS's string-redaction integration tests (`redaction.test.ts` `redactEvent integration tests`,
/// `v2/pipeline.test.ts`): the hook redacts the arguments, the response and the actor's data, and
/// leaves the protected fields (session, project, tool, event type, actor ID and name) as they are,
/// even when they hold what it redacts.
#[tokio::test]
async fn tool_call_strings_are_redacted_except_protected_fields() {
    let options = Options::default()
        .redact_sensitive_information(mark_sensitive)
        .identify(|_call| {
            let user = UserIdentity::new("user-with-id-123")
                .user_name("David Smith")
                .user_data(object(json!({"email": "user@example.com"})));
            ready(Ok(Some(user)))
        });
    let calls = vec![(
        "echo",
        json!({"msg": "Send email to admin@company.com", "context": "Adding a todo item"}),
    )];
    let (published, log) = publish_calls(Sample, options, calls).await;
    assert_eq!(log, Vec::<String>::new());
    let event = all(published).remove(0);
    let arguments = at(&event, "/parameters/request/params/arguments");
    assert_eq!(arguments["msg"], json!("[REDACTED-EMAIL]"));
    assert_eq!(arguments["context"], json!("Adding a todo item"));
    assert_eq!(at(&event, "/response/content/0/text"), "[REDACTED-EMAIL]");
    assert_eq!(
        event["identifyActorData"],
        json!({"email": "[REDACTED-EMAIL]"})
    );
    let request = publish_request(&event);
    assert!(request.session_id.flatten().unwrap().starts_with("ses_"));
    assert_eq!(request.project_id, E2E_PROJECT);
    assert_eq!(request.resource_name.as_deref(), Some("echo"));
    assert_eq!(
        request.identify_actor_given_id.as_deref(),
        Some("user-with-id-123")
    );
    assert_eq!(request.actor_id.as_deref(), Some("user-with-id-123"));
    assert_eq!(request.identify_actor_name.as_deref(), Some("David Smith"));
}

/// TS's event-hook integration tests (`redaction.test.ts` `redactEvent hook integration tests`):
/// the hook sees each event's tool, a rejected event is dropped while the others publish, and the
/// fields it replaces stay replaced except the system ones.
#[tokio::test]
async fn event_hook_replaces_fields_and_drops_rejected_events() {
    let seen: Arc<Mutex<Vec<Option<String>>>> = Arc::default();
    let tools = Arc::clone(&seen);
    let options = Options::default().redact_event(move |event: Event| {
        tools.lock().unwrap().push(event.resource_name.clone());
        let result = (event.resource_name.as_deref() != Some("add_todo")).then(|| Event {
            parameters: Some(json!({"replaced": true})),
            project_id: "proj_other".to_owned(),
            session_id: None,
            ..event
        });
        ready(Ok(result))
    });
    let calls = vec![
        (
            "add_todo",
            json!({"text": "Buy milk", "context": "Adding a todo item"}),
        ),
        ("list_todos", json!({"context": "Listing todos"})),
    ];
    let (published, log) = publish_calls(Payloads, options, calls).await;
    assert_eq!(log, ["Event dropped by redactEvent hook"]);
    assert_eq!(
        *seen.lock().unwrap(),
        [Some("add_todo".to_owned()), Some("list_todos".to_owned())]
    );
    assert_eq!(published[0], None);
    let event = published[1].as_ref().expect("list_todos publishes");
    assert_eq!(event["parameters"], json!({"replaced": true}));
    assert!(event["sessionId"].as_str().unwrap().starts_with("ses_"));
    assert_eq!(event["projectId"], json!(E2E_PROJECT));
    assert_eq!(event["eventType"], json!("mcp:tools/call"));
}

/// TS's `should run the event hook on raw values before string redaction`: the event hook sees
/// the raw argument, and the string hook still redacts the published event.
#[tokio::test]
async fn event_hook_sees_raw_values_before_string_redaction() {
    let raw: Arc<Mutex<Vec<Value>>> = Arc::default();
    let texts = Arc::clone(&raw);
    let options = Options::default()
        .redact_event(move |event: Event| {
            let text = event
                .parameters
                .as_ref()
                .and_then(|p| p.pointer("/request/params/arguments/text"))
                .cloned();
            texts.lock().unwrap().extend(text);
            ready(Ok(Some(event)))
        })
        .redact_sensitive_information(|text: String| {
            let text = if text.contains("secret") {
                "[REDACTED-SECRET]".to_owned()
            } else {
                text
            };
            ready(Ok(text))
        });
    let calls = vec![(
        "add_todo",
        json!({"text": "this contains a secret value", "context": "Adding a todo item"}),
    )];
    let (published, _log) = publish_calls(Payloads, options, calls).await;
    assert_eq!(
        *raw.lock().unwrap(),
        [json!("this contains a secret value")]
    );
    let event = all(published).remove(0);
    assert_eq!(
        at(&event, "/parameters/request/params/arguments/text"),
        "[REDACTED-SECRET]"
    );
}

/// TS's error-capture integration tests (`error-capture.test.ts`): each call publishes exactly one
/// event; a call that works has no `isError` and no `error`; an unknown tool and arguments the
/// tool refuses (a missing field, an unknown one: rmcp's validation, not zod's, RD-13) publish
/// `isError` with the error's text (rmcp's JSON-RPC error for the unknown tool, RD-05).
#[tokio::test]
async fn call_outcomes_publish_one_event_each() {
    let calls = vec![
        (
            "echo",
            json!({"msg": "Test todo", "context": "Testing successful tool calls"}),
        ),
        (
            "nonexistent_tool",
            json!({"context": "Testing unknown tool error"}),
        ),
        ("echo", json!({"context": "Testing missing parameter"})),
        ("strict", json!({"text": "x", "operation": "modulo"})),
    ];
    let (published, log) = publish_calls(Sample, Options::default(), calls).await;
    assert_eq!(log, Vec::<String>::new());
    let requests: Vec<_> = all(published).iter().map(publish_request).collect();
    assert_eq!(requests.len(), 4);
    assert_eq!(requests[0].is_error, None);
    assert_eq!(requests[0].error, None);
    let message = |n: usize| {
        assert_eq!(requests[n].is_error, Some(true), "call {n}");
        let error = requests[n].error.as_ref().expect("an error");
        error["message"].as_str().expect("a message").to_owned()
    };
    assert!(message(1).contains("not found"), "{}", message(1));
    assert!(message(2).contains("msg"), "{}", message(2));
    assert!(message(3).contains("operation"), "{}", message(3));
}

/// TS's `should NOT publish identify events when identify function throws`: the identify error is
/// a log line, the call still publishes, without an actor, and no other event exists.
#[tokio::test]
async fn identify_error_still_publishes_the_call() {
    let options = Options::default().identify(|_call| ready(Err("Identify error".into())));
    let calls = vec![(
        "echo",
        json!({"msg": "hi", "context": "Testing identify error"}),
    )];
    let (published, log) = publish_calls(Sample, options, calls).await;
    assert_eq!(log, ["Error: identify callback threw - Identify error"]);
    let events = all(published);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["eventType"], json!("mcp:tools/call"));
    assert_eq!(events[0].get("identifyActorGivenId"), None);
}

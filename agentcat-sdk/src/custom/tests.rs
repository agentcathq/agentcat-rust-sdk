//! `CustomEvent`: each setter stores its own field and no other, and the two `publish_custom_event`
//! forms build the event the fixture gives.
use std::future::ready;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use agentcat_api::models::PublishEventRequest;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, Implementation, InitializeResult,
    ServerCapabilities,
};
use rmcp::service::{MaybeSendFuture, RequestContext};
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde_json::{Map, Value, json};
use tokio::runtime::Runtime;

use super::{CustomEvent, publish_server, publish_string};
use crate::client::{AgentCat, track};
use crate::config::{HookError, Options};
use crate::context::CallContext;
use crate::delivery::{Delivery, DeliveryConfig};
use crate::error::Error;
use crate::event::assemble::EventJob;
use crate::event::{CallRecord, ClientIdentity, Event};
use crate::sink::Sink;
use crate::test_support::queue::{self, CapturedLines, RecordingPublisher};
use crate::test_support::rmcp::{Sample, connect};
use crate::wire;

/// How long a test waits for a flush.
const WAIT: Duration = Duration::from_secs(2);

/// The text the worker logs for a server-form event without a session.
const NO_SESSION_LINE: &str = "publish_custom_event: no session_id provided; event will be published without a session. Pass CustomEvent::session_id to attribute it to a task.";

/// The frozen clock of the vectors.
const FROZEN: &str = "2026-10-23T12:00:00.000Z";

/// `n` milliseconds.
const fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// The queue limits with short budgets.
fn fast() -> DeliveryConfig {
    DeliveryConfig {
        max_pending: 10_000,
        concurrency: 5,
        retry_delays: [ms(10), ms(20), ms(40)],
        hook_budget: ms(50),
        drop_drain: ms(1_000),
    }
}

/// A handle for `project` whose delivery posts through a recording publisher and writes its lines
/// to a capture.
fn rig(project: &str, options: Options) -> (AgentCat, RecordingPublisher, CapturedLines) {
    let publisher = RecordingPublisher::new();
    let lines = CapturedLines::default();
    let delivery = Delivery::with_writer(
        project.to_owned(),
        &options,
        Arc::new(publisher.clone()),
        None,
        fast(),
        lines.writer(),
    );
    (
        AgentCat::with_sink(project, options, Arc::new(delivery)),
        publisher,
        lines,
    )
}

/// The requests the handle delivered, after a flush.
fn delivered(agentcat: &AgentCat, publisher: &RecordingPublisher) -> Vec<PublishEventRequest> {
    assert!(agentcat.flush_blocking(WAIT), "the flush finishes");
    publisher.requests()
}

/// A runtime with worker threads for the customer's redaction hooks.
fn hooks_runtime() -> Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("a runtime")
}

/// The context of a call of `agentcat` with no session and no identity.
fn bare_call(agentcat: &AgentCat) -> CallContext {
    CallContext::new(String::new(), None, agentcat.clone())
}

fn object(key: &str, value: Value) -> Map<String, Value> {
    let mut map = Map::new();
    map.insert(key.to_owned(), value);
    map
}

#[test]
fn custom_event_setters_store_fields() {
    let empty = CustomEvent::new();
    assert!(empty.session_id.is_none() && empty.resource_name.is_none());
    assert!(empty.parameters.is_none() && empty.response.is_none());
    assert!(empty.message.is_none() && empty.duration.is_none());
    assert!(empty.is_error.is_none() && empty.error.is_none());
    assert!(empty.tags.is_none() && empty.properties.is_none());

    let event = CustomEvent::new()
        .session_id("ses_1")
        .resource_name(String::from("cache"))
        .parameters(json!({"key": 1}))
        .response(json!([true]))
        .message("warmed")
        .duration(Duration::from_millis(40))
        .is_error(true)
        .error(json!({"message": "boom"}))
        .tags(object("team", json!("a")))
        .properties(object("n", json!(2)));
    assert_eq!(event.session_id.as_deref(), Some("ses_1"));
    assert_eq!(event.resource_name.as_deref(), Some("cache"));
    assert_eq!(event.parameters, Some(json!({"key": 1})));
    assert_eq!(event.response, Some(json!([true])));
    assert_eq!(event.message.as_deref(), Some("warmed"));
    assert_eq!(event.duration, Some(Duration::from_millis(40)));
    assert_eq!(event.is_error, Some(true));
    assert_eq!(event.error, Some(json!({"message": "boom"})));
    assert_eq!(event.tags, Some(object("team", json!("a"))));
    assert_eq!(event.properties, Some(object("n", json!(2))));
    assert_eq!(format!("{:?}", event.clone()), format!("{event:?}"));
}

// ------------------------------------------------------------------------------ the string form

#[test]
fn string_form_requires_project() {
    let (agentcat, publisher, _lines) = rig("proj_handle", Options::default());
    let result = agentcat.publish_custom_event("ses_1", "", CustomEvent::new());
    assert!(matches!(result, Err(Error::ProjectIdRequired)));
    assert_eq!(
        result.unwrap_err().to_string(),
        "project_id is required for publish_custom_event"
    );
    assert_eq!(delivered(&agentcat, &publisher).len(), 0);
}

#[test]
fn string_form_honors_project_argument() {
    // The string form sends the argument's project; the server form sends the handle's.
    let (agentcat, publisher, _lines) = rig("proj_handle", Options::default());
    agentcat
        .publish_custom_event("ses_1", "proj_argument", CustomEvent::new())
        .expect("queued");
    let requests = delivered(&agentcat, &publisher);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].project_id, "proj_argument");
}

#[test]
fn string_form_session_precedence() {
    let (agentcat, publisher, lines) = rig("proj_handle", Options::default());
    let publish = |argument: &str, event: CustomEvent| {
        agentcat
            .publish_custom_event(argument, "proj", event)
            .expect("queued");
    };
    publish("from-argument", CustomEvent::new().session_id("from-event"));
    publish("from-argument", CustomEvent::new().session_id(""));
    publish("from-argument", CustomEvent::new());
    publish("", CustomEvent::new());
    let sessions: Vec<Option<String>> = delivered(&agentcat, &publisher)
        .into_iter()
        .map(|request| request.session_id.flatten())
        .collect();
    let mut sessions: Vec<String> = sessions
        .into_iter()
        .map(Option::unwrap_or_default)
        .collect();
    sessions.sort();
    assert_eq!(
        sessions,
        ["", "from-argument", "from-argument", "from-event"]
    );
    let logged = lines.lines();
    assert!(logged.contains(
        &"Published custom event for session from-event with type 'agentcat:custom'".to_owned()
    ));
    assert!(logged.contains(
        &"Published custom event without a session with type 'agentcat:custom'".to_owned()
    ));
}

#[test]
fn string_form_has_no_server_fields() {
    let (agentcat, publisher, _lines) = rig("proj_handle", Options::default());
    agentcat
        .publish_custom_event("ses_1", "proj", CustomEvent::new().resource_name("r"))
        .expect("queued");
    let request = &delivered(&agentcat, &publisher)[0];
    assert!(request.server_name.is_none() && request.server_version.is_none());
    assert!(request.client_name.is_none() && request.client_version.is_none());
    assert!(request.sdk_language.is_none() && request.agentcat_version.is_none());
    assert!(request.identify_data.is_none());
}

// ------------------------------------------------------------------------------ the server form

#[test]
fn server_form_uses_handle_project() {
    let (agentcat, publisher, _lines) = rig("proj_handle", Options::default());
    bare_call(&agentcat)
        .publish_custom_event("proj_argument", CustomEvent::new().session_id("ses_1"))
        .expect("queued");
    let request = &delivered(&agentcat, &publisher)[0];
    assert_eq!(request.project_id, "proj_handle");
    assert_eq!(request.sdk_language.as_deref(), Some("Rust"));
    assert_eq!(
        request.agentcat_version.as_deref(),
        Some(env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(request.identify_data, Some(json!({})));
    assert_eq!(request.duration, Some(0));

    let result = bare_call(&agentcat).publish_custom_event("", CustomEvent::new());
    assert!(matches!(result, Err(Error::ProjectIdRequired)));
}

#[test]
fn server_form_dropped_with_tracing_off() {
    let (agentcat, publisher, lines) = rig("proj_handle", Options::default().enable_tracing(false));
    bare_call(&agentcat)
        .publish_custom_event("proj", CustomEvent::new())
        .expect("accepted");
    assert_eq!(delivered(&agentcat, &publisher).len(), 0);
    assert_eq!(
        lines.lines(),
        [
            NO_SESSION_LINE,
            "Published custom event without a session with type 'agentcat:custom'"
        ]
    );
    // The string form is not gated by tracing (it goes to the queue directly).
    agentcat
        .publish_custom_event("ses_1", "proj", CustomEvent::new())
        .expect("queued");
    assert_eq!(delivered(&agentcat, &publisher).len(), 1);
}

#[test]
fn server_form_without_session_logs_the_no_session_warning() {
    let (agentcat, _publisher, lines) = rig("proj_handle", Options::default());
    let call = bare_call(&agentcat);
    call.publish_custom_event("proj", CustomEvent::new())
        .expect("queued");
    call.publish_custom_event("proj", CustomEvent::new().session_id("ses_1"))
        .expect("queued");
    assert!(agentcat.flush_blocking(WAIT));
    let logged = lines.lines();
    assert_eq!(logged.iter().filter(|l| *l == NO_SESSION_LINE).count(), 1);
    assert_eq!(logged[0], NO_SESSION_LINE);
}

#[test]
fn server_form_applies_redact_event_only() {
    // The event hook runs on a custom event of the server form, the string hook never.
    let options = Options::default()
        .redact_event(|event: Event| {
            let mut fixture = event.into_object();
            fixture.insert("response".to_owned(), json!({"scrubbed": true}));
            ready(Ok::<_, HookError>(Some(Event::from_object(&fixture))))
        })
        .redact_sensitive_information(|text: String| {
            ready(Ok::<_, HookError>(text.replace("SECRET", "[REDACTED]")))
        });
    let (agentcat, publisher, _lines) = rig("proj_handle", options);
    let runtime = hooks_runtime();
    let _entered = runtime.enter();
    bare_call(&agentcat)
        .publish_custom_event(
            "proj",
            CustomEvent::new()
                .session_id("ses_1")
                .parameters(json!({"key": "SECRET"}))
                .response(json!({"secret": "raw"})),
        )
        .expect("queued");
    let request = &delivered(&agentcat, &publisher)[0];
    assert_eq!(request.response, Some(json!({"scrubbed": true})));
    assert_eq!(request.parameters, Some(json!({"key": "SECRET"})));
}

/// A server whose tool publishes a custom event through its [`CallContext`].
#[derive(Debug, Clone, Copy)]
struct Emitter;

impl ServerHandler for Emitter {
    fn call_tool(
        &self,
        _request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResponse, ErrorData>> + MaybeSendFuture + '_ {
        let call = context.extensions.get::<CallContext>().cloned();
        let published = call.map(|call| {
            call.publish_custom_event("proj", CustomEvent::new().resource_name("custom-from-tool"))
        });
        assert!(matches!(published, Some(Ok(()))), "{published:?}");
        ready(Ok(CallToolResult::structured(json!({})).into()))
    }

    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("emitter-server", "9.9.9"))
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn call_context_form_uses_call_identity() {
    let (agentcat, publisher, _lines) = rig("proj_handle", Options::default());
    let client = connect(agentcat.track(Emitter), "caller-x").await;
    client
        .call_tool(CallToolRequestParams::new("anything"))
        .await
        .expect("the call answers");
    assert!(agentcat.flush(WAIT).await);
    let custom: Vec<_> = publisher
        .requests()
        .into_iter()
        .filter(|request| request.resource_name.as_deref() == Some("custom-from-tool"))
        .collect();
    assert_eq!(custom.len(), 1);
    let request = &custom[0];
    assert_eq!(request.client_name.as_deref(), Some("caller-x"));
    assert_eq!(request.client_version.as_deref(), Some("1.0.0"));
    assert_eq!(request.server_name.as_deref(), Some("emitter-server"));
    assert_eq!(request.server_version.as_deref(), Some("9.9.9"));
    assert_eq!(request.project_id, "proj_handle");
}

#[tokio::test(flavor = "multi_thread")]
async fn tracked_form_uses_initialize_identity() {
    let (agentcat, publisher, _lines) = rig("proj_handle", Options::default());
    let tracked = Arc::new(agentcat.track(Sample));
    tracked
        .publish_custom_event("proj", CustomEvent::new().resource_name("before"))
        .expect("queued");
    let _client = connect(tracked.clone(), "identity-test").await;
    tracked
        .publish_custom_event("proj", CustomEvent::new().resource_name("after"))
        .expect("queued");
    assert!(agentcat.flush(WAIT).await);
    let requests = publisher.requests();
    let find = |name: &str| {
        requests
            .iter()
            .find(|request| request.resource_name.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("{name} was delivered"))
    };
    let server = ServerHandler::get_info(&Sample).server_info;
    let before = find("before");
    assert!(before.client_name.is_none() && before.client_version.is_none());
    assert_eq!(before.server_name.as_deref(), Some(server.name.as_str()));
    let after = find("after");
    assert_eq!(after.client_name.as_deref(), Some("identity-test"));
    assert_eq!(after.client_version.as_deref(), Some("1.0.0"));
    assert_eq!(
        after.server_version.as_deref(),
        Some(server.version.as_str())
    );
    assert!(
        track(Sample, "p", Options::default())
            .publish_custom_event("", CustomEvent::new())
            .is_err()
    );
}

#[test]
fn server_form_duration_is_given_or_elapsed() {
    let (agentcat, publisher, _lines) = rig("proj_handle", Options::default());
    let start = wire::parse_iso_millis(FROZEN).expect("a time");
    let later = start + ms(250);
    let publish = |event: CustomEvent| {
        publish_server(&agentcat, "proj", event, None, None, start, later).expect("queued");
    };
    publish(CustomEvent::new().resource_name("given").duration(ms(1500)));
    publish(CustomEvent::new().resource_name("elapsed"));
    publish(
        CustomEvent::new()
            .resource_name("zero")
            .duration(Duration::ZERO),
    );
    let durations: Vec<(String, Option<i32>)> = delivered(&agentcat, &publisher)
        .into_iter()
        .map(|r| (r.resource_name.unwrap_or_default(), r.duration))
        .collect();
    let duration = |name: &str| {
        durations
            .iter()
            .find(|(n, _)| n == name)
            .and_then(|(_, d)| *d)
    };
    assert_eq!(duration("given"), Some(1500));
    assert_eq!(duration("elapsed"), Some(250));
    // A zero duration counts as none: `duration || elapsed`.
    assert_eq!(duration("zero"), Some(250));
}

// ------------------------------------------------------------------------------------ vectors

/// The `CustomEvent` of a vector's `data`.
fn event_of(data: &Map<String, Value>) -> CustomEvent {
    let mut event = CustomEvent::new();
    let text = |key: &str| data.get(key).and_then(Value::as_str);
    let object = |key: &str| data.get(key).and_then(Value::as_object).cloned();
    if let Some(session) = text("sessionId") {
        event = event.session_id(session);
    }
    if let Some(name) = text("resourceName") {
        event = event.resource_name(name);
    }
    if let Some(value) = data.get("parameters") {
        event = event.parameters(value.clone());
    }
    if let Some(value) = data.get("response") {
        event = event.response(value.clone());
    }
    if let Some(message) = text("message") {
        event = event.message(message);
    }
    if let Some(millis) = data.get("duration").and_then(Value::as_u64) {
        event = event.duration(ms(millis));
    }
    if let Some(is_error) = data.get("isError").and_then(Value::as_bool) {
        event = event.is_error(is_error);
    }
    if let Some(value) = data.get("error") {
        event = event.error(value.clone());
    }
    if let Some(tags) = object("tags") {
        event = event.tags(tags);
    }
    if let Some(properties) = object("properties") {
        event = event.properties(properties);
    }
    event
}

/// The identity a vector names: `{name, version}`.
fn identity_of(info: &Value) -> (String, String) {
    (
        info["name"].as_str().expect("a name").to_owned(),
        info["version"].as_str().expect("a version").to_owned(),
    )
}

/// `request` as the vector compares it: an `agentcatVersion` is this crate's version.
fn rust_identity(mut request: Value) -> Value {
    if let Some(fields) = request.as_object_mut()
        && fields.contains_key("agentcatVersion")
    {
        fields.insert(
            "agentcatVersion".to_owned(),
            Value::from(env!("CARGO_PKG_VERSION")),
        );
    }
    request
}

/// Each `custom:*` case of `queue.json` through the builders and the whole delivery worker, with a
/// recording publisher and the vector's frozen clock: the requests and every log line, the
/// builder's included, must be the fixture's.
#[test]
fn custom_fixture_cases_hold() {
    let runtime = hooks_runtime();
    let _entered = runtime.enter();
    let now: SystemTime = wire::parse_iso_millis(FROZEN).expect("a time");
    let mut ran = 0;
    for case in queue::cases().into_iter().filter(|case| {
        case["name"]
            .as_str()
            .is_some_and(|n| n.starts_with("custom:"))
    }) {
        let name = case["name"].as_str().expect("a name").to_owned();
        let custom = case["custom"].as_object().expect("custom");
        let data = custom["data"].as_object().expect("data");
        let project = custom["project_id"].as_str().expect("a project");
        let options = queue::options_of(&case);
        let event = event_of(data);

        let result = match custom["form"].as_str().expect("a form") {
            "string" => {
                let (agentcat, publisher, lines) = rig("proj_handle", options);
                let session = custom["session"].as_str().expect("a session");
                publish_string(&agentcat, session, project, event, now).expect("queued");
                (agentcat, publisher, lines)
            }
            form => {
                assert_eq!(form, "server", "{name}");
                let tracked = custom["tracked_project_id"].as_str().expect("a project");
                let (agentcat, publisher, lines) = rig(tracked, options);
                let (server_name, server_version) = identity_of(&custom["server_info"]);
                let (client_name, client_version) = identity_of(&custom["client_info"]);
                publish_server(
                    &agentcat,
                    project,
                    event,
                    Some(&ClientIdentity {
                        name: Some(client_name),
                        version: Some(client_version),
                    }),
                    Some(&Implementation::new(server_name, server_version)),
                    now,
                    now,
                )
                .expect("queued");
                (agentcat, publisher, lines)
            }
        };
        let (agentcat, publisher, lines) = result;
        let requests: Vec<Value> = delivered(&agentcat, &publisher)
            .iter()
            .map(queue::to_request_model)
            .collect();
        let expected: Vec<Value> = queue::expected_requests(&case)
            .into_iter()
            .map(rust_identity)
            .collect();
        assert_eq!(requests, expected, "{name}: requests");

        let logs: Vec<String> = lines
            .lines()
            .iter()
            .map(|line| queue::mask_event_ids(line))
            .collect();
        assert_eq!(logs, queue::full_logs(&case), "{name}: logs");
        ran += 1;
    }
    assert_eq!(ran, 18, "every custom case ran");
}

/// The queue writes its drop line inside `add`, before `publish_custom_event` writes its own: the
/// event and the `Published` line reach the worker together. The test waits for the two lines on
/// the capture's signal of each new line.
#[test]
fn queue_drop_line_comes_before_published_line() {
    let (agentcat, publisher, lines) = rig("proj_handle", Options::default());
    // A first event starts the worker; a stopped handle that never started writes nothing.
    agentcat
        .publish_custom_event("ses_0", "proj", CustomEvent::new())
        .expect("queued");
    assert_eq!(delivered(&agentcat, &publisher).len(), 1);
    assert!(agentcat.shutdown_blocking(WAIT));
    let before = lines.lines().len();
    agentcat
        .publish_custom_event("ses_1", "proj", CustomEvent::new())
        .expect("accepted");
    assert_eq!(publisher.requests().len(), 1);
    assert!(lines.wait_for(before + 2, WAIT), "{:?}", lines.lines());
    assert_eq!(
        lines.lines()[before..],
        [
            "Queue is shutting down, event dropped",
            "Published custom event for session ses_1 with type 'agentcat:custom'"
        ]
    );
}

/// A sink that keeps its calls in order and has no `event_then_log` of its own.
#[derive(Default)]
struct OrderSink(Mutex<Vec<String>>);

impl Sink for OrderSink {
    fn log(&self, message: String) {
        self.0.lock().unwrap().push(message);
    }

    fn record(&self, _record: CallRecord) {
        self.0.lock().unwrap().push("record".to_owned());
    }

    fn event(&self, job: EventJob) {
        let event_type = job.event.get("eventType").and_then(Value::as_str);
        self.0
            .lock()
            .unwrap()
            .push(format!("event {}", event_type.unwrap_or("-")));
    }
}

/// `publish_custom_event` logs its `Published` line after the queue's `add`: a sink without its own
/// `event_then_log` takes the event, then the line, through the trait's default.
#[test]
fn default_event_then_log_hands_the_event_then_the_line() {
    let sink = Arc::new(OrderSink::default());
    let agentcat = AgentCat::with_sink("proj_handle", Options::default(), sink.clone());
    agentcat
        .publish_custom_event("ses_1", "proj", CustomEvent::new())
        .expect("queued");
    assert_eq!(
        *sink.0.lock().unwrap(),
        [
            "event agentcat:custom",
            "Published custom event for session ses_1 with type 'agentcat:custom'"
        ]
    );
}

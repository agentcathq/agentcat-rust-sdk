//! The `queue.json` vectors (TS's event queue, written by `oracle/src/vectors/gen/queue.mjs`): the
//! loader, each case's hook outcomes as tasks and its redaction hooks as [`Redactors`], TS's log
//! lines as the pipeline writes them, and TS's camelCase request model of a
//! [`PublishEventRequest`].
//!
//! Log lines compare after these steps ([`expected_logs`]):
//!
//! - The delivery lines (`Successfully sent event ...`, `Failed to send event ...`) are the
//!   delivery worker's (plan 4 task 6), not the pipeline's, so they leave the expected lines.
//! - In a `custom:*` case, the lines `publishCustomEvent` writes before the queue takes the event
//!   (`Published custom event ...`, `publishCustomEvent: ...` and its `Dropping invalid tag: ...`)
//!   are the custom-event builder's (plan 4 task 7), so they leave too ([`full_logs`] keeps them
//!   for `custom::tests::custom_vectors_match_ts`, which runs the builders).
//! - One normalization (ruling P7): where TS writes a thrown `Error` after a stage's or a hook's
//!   text, as `Error: <message>`, Rust writes the error's `Display`, `<message>`
//!   ([`ts_line_as_rust`]). The truncation error needs none: Rust's `TruncateError` carries TS's
//!   V8 text (`TypeError: str.slice is not a function`), so that line compares whole, more than the
//!   prefix `Failed to truncate event: ` the ruling allows.
//!
//! The delivery worker's runners ([`worker_logs`], plan 4 task 6) keep the delivery lines, with one
//! more normalization (ruling P7): where a TS delivery line embeds the error of a failed attempt,
//! after `[Error: ` and after `Failed to deliver AgentCat event after retries: `, TS writes the
//! error as `getMCPCompatibleErrorMessage` renders a JS `Error`, `{"stack":"Error: M","message":M}`
//! (or `Error: M`), and Rust writes the publisher's text `M` ([`ts_delivery_line_as_rust`]). On the
//! Rust side each `evt_` ID becomes `<evt>` ([`mask_event_ids`]), as the generator masks TS's.
//!
//! The delivery tests share a [`RecordingPublisher`] (scripted failures, a gate that holds every
//! attempt, start notices, a per-attempt delay) and [`CapturedLines`], the worker's log lines with
//! the thread that wrote each one.
//!
//! Requests compare as [`to_ts_model`] writes the Rust request and [`expected_requests`] the TS
//! one: each `evt_` ID is `<evt>` and each minted session `<minted>` (as the generator masks
//! them), the timestamp is compared to the millisecond, tags compare as a map (sorted keys: the
//! Rust request holds them in a `HashMap`, RD-03), a `null` `userIntent` is omitted, as the Rust
//! wire omits it (ruling P12), and any other `userIntent` that is not a string is its
//! `JSON.stringify` text (RD-33).
use std::collections::BTreeMap;
use std::future::{pending, ready};
use std::mem;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError, mpsc};
use std::time::{Duration, Instant};

use agentcat_api::models::PublishEventRequest;
use serde_json::{Map, Value};
use tokio::runtime::Handle;
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;

use super::json::object;
use super::vectors;
use crate::config::{HookError, Options, UserIdentity};
use crate::delivery::{LineWriter, Publisher};
use crate::event::Event;
use crate::event::assemble::EventJob;
use crate::handles::is_valid_session_id;
use crate::hooks::PendingHooks;
use crate::pipeline::process::Redactors;
use crate::sink::BoxFuture;
use crate::wire;

/// The cases of `queue.json`.
pub(crate) fn cases() -> Vec<Map<String, Value>> {
    vectors::cases("queue")
}

/// The `queue.json` case `name`.
///
/// # Panics
///
/// When `queue.json` has no such case.
pub(crate) fn case(name: &str) -> Map<String, Value> {
    cases()
        .into_iter()
        .find(|case| vectors::name(case) == name)
        .unwrap_or_else(|| panic!("queue.json has no case {name}"))
}

// ------------------------------------------------------------------------------- hook outcomes

/// The task of one call hook.
pub(crate) type Task<T> = JoinHandle<Result<Option<T>, HookError>>;

/// A hook task that gives `value`.
pub(crate) fn returns<T: Send + 'static>(value: Option<T>) -> Task<T> {
    tokio::spawn(async move { Ok(value) })
}

/// A hook task that fails with the error `message`.
pub(crate) fn throws<T: Send + 'static>(message: &str) -> Task<T> {
    let message = message.to_owned();
    tokio::spawn(async move { Err(message.into()) })
}

/// A hook task that never finishes.
pub(crate) fn hangs<T: Send + 'static>() -> Task<T> {
    tokio::spawn(pending())
}

/// The task of a raw hook outcome of `queue.json`: `{"returns": v}` (`null` gives nothing),
/// `{"throws": message}` or `{"hangs": true}`; `value` reads a returned value.
fn outcome_task<T: Send + 'static>(outcome: &Value, value: fn(&Value) -> T) -> Task<T> {
    if outcome.get("hangs").is_some() {
        return hangs();
    }
    if let Some(message) = outcome.get("throws") {
        return throws(message.as_str().expect("a message"));
    }
    let returned = &outcome["returns"];
    returns((!returned.is_null()).then(|| value(returned)))
}

/// The `UserIdentity` of an identify result `{userId, userName?, userData?}`.
fn identity_of(value: &Value) -> UserIdentity {
    let mut identity = UserIdentity::new(value["userId"].as_str().expect("a userId"));
    if let Some(name) = value.get("userName").and_then(Value::as_str) {
        identity = identity.user_name(name);
    }
    if let Some(Value::Object(data)) = value.get("userData") {
        identity = identity.user_data(data.clone());
    }
    identity
}

/// The hook tasks of a case's `hooks`, spawned on the current runtime.
pub(crate) fn pending_of(hooks: &Map<String, Value>) -> PendingHooks {
    PendingHooks {
        resolve_session_id: hooks.get("resolveSessionId").map(|outcome| {
            outcome_task(outcome, |value| {
                value.as_str().expect("a string").to_owned()
            })
        }),
        identify: hooks
            .get("identify")
            .map(|outcome| outcome_task(outcome, identity_of)),
        event_tags: hooks
            .get("eventTags")
            .map(|outcome| outcome_task(outcome, |value| object(value.clone()))),
        event_properties: hooks
            .get("eventProperties")
            .map(|outcome| outcome_task(outcome, |value| object(value.clone()))),
    }
}

// ------------------------------------------------------------------------------------- jobs

/// The redaction stages of a case's job: a tool call runs both; a custom event never runs the
/// string stage (K-03), and only its server form runs the event stage (as plan 4 task 7 builds
/// custom events; `custom::tests::custom_vectors_match_ts` runs these cases through the builders).
fn redaction_stages(case: &Map<String, Value>) -> (bool, bool) {
    match case
        .get("custom")
        .and_then(|custom| custom["form"].as_str())
    {
        None => (true, true),
        Some("string") => (false, false),
        Some(_) => (false, true),
    }
}

/// The job of the event the queue takes in `case`: its hooks ([`pending_of`]) spawned on
/// `runtime`, which also runs the redaction hooks; the case's hook mode; the redaction stages of
/// [`redaction_stages`].
///
/// # Panics
///
/// When the case has hooks and `runtime` is `None`.
pub(crate) fn job_of(case: &Map<String, Value>, runtime: Option<&Handle>) -> EventJob {
    let hooks = case.get("hooks").and_then(Value::as_object).map(|hooks| {
        let _entered = runtime.expect("a runtime for the hooks").enter();
        pending_of(hooks)
    });
    let (string_redaction, event_redaction) = redaction_stages(case);
    EventJob {
        event: object(case["event"].clone()),
        hooks: hooks.unwrap_or_default(),
        hook_mode: case.get("hook_mode") == Some(&Value::Bool(true)),
        runtime: runtime.cloned(),
        string_redaction,
        event_redaction,
        backtrace: None,
    }
}

// ---------------------------------------------------------------------------- redaction hooks

/// The redaction hooks of `case` ([`options_of`]).
pub(crate) fn redactors_of(case: &Map<String, Value>) -> Redactors {
    let options = options_of(case);
    Redactors {
        string: options.string_redactor().cloned(),
        event: options.event_redactor().cloned(),
    }
}

/// Options with the redaction hooks of `case`: its `redact_event` (`{"returns": null}` drops the
/// event, `{"throws": message}` fails, `{"merge": fields}` returns `{...event, ...fields}`) and its
/// `redact_strings` (`{"replace": secret}` replaces each occurrence with `[REDACTED]`,
/// `{"throw_at": n}` fails on call `n` with `redactor failed` and returns the other strings).
pub(crate) fn options_of(case: &Map<String, Value>) -> Options {
    let mut options = Options::default();
    if let Some(spec) = case.get("redact_event") {
        let spec = spec.clone();
        options = options.redact_event(move |event: Event| {
            let result: Result<Option<Event>, HookError> = if spec.get("throws").is_some() {
                Err(spec["throws"].as_str().expect("a message").into())
            } else if let Some(Value::Object(fields)) = spec.get("merge") {
                let mut merged = event.into_object();
                merged.extend(fields.clone());
                Ok(Some(Event::from_object(&merged)))
            } else {
                Ok(None)
            };
            ready(result)
        });
    }
    if let Some(spec) = case.get("redact_strings") {
        let replace = spec
            .get("replace")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let throw_at = spec
            .get("throw_at")
            .and_then(Value::as_u64)
            .map(|n| usize::try_from(n).expect("a call number"));
        let calls = Arc::new(AtomicUsize::new(0));
        options = options.redact_sensitive_information(move |text: String| {
            let call = calls.fetch_add(1, Ordering::SeqCst) + 1;
            let result: Result<String, HookError> = match (&replace, throw_at) {
                (Some(secret), _) => Ok(text.replace(secret.as_str(), "[REDACTED]")),
                (None, Some(n)) if n == call => Err("redactor failed".into()),
                (None, _) => Ok(text),
            };
            ready(result)
        });
    }
    options
}

// --------------------------------------------------------------------------------- log lines

/// The starts of the delivery worker's lines (module docs).
const DELIVERY_LINES: [&str; 2] = ["Successfully sent event ", "Failed to send event "];

/// The starts of the lines `publishCustomEvent` writes before the queue takes the event (module
/// docs).
const CUSTOM_BUILDER_LINES: [&str; 3] = [
    "Published custom event ",
    "publish_custom_event: ",
    "Dropping invalid tag: ",
];

/// The texts after which TS writes a thrown `Error` (the hooks' and the stages'): the text of each
/// call hook's `catch`, then the queue's stage lines.
const ERROR_PREFIXES: [&str; 8] = [
    "resolve_session_id hook error: ",
    "Error: identify hook returned an error - ",
    "event_tags callback error: ",
    "event_properties callback error: ",
    "Failed to resolve pending hook results: ",
    "Failed to redact event (event-level hook): ",
    "Failed to redact event: ",
    "Failed to truncate event: ",
];

/// A TS log line as Rust writes it (ruling P7, module docs): after one of [`ERROR_PREFIXES`], TS's
/// `Error: M` is Rust's `M`.
pub(crate) fn ts_line_as_rust(line: &str) -> String {
    for prefix in ERROR_PREFIXES {
        let message = line
            .strip_prefix(prefix)
            .and_then(|rest| rest.strip_prefix("Error: "));
        if let Some(message) = message {
            return format!("{prefix}{message}");
        }
    }
    line.to_owned()
}

/// The lines of `case`'s `logs` that the queue writes: without a custom event's builder lines
/// (module docs).
fn queue_lines(case: &Map<String, Value>) -> impl Iterator<Item = &str> {
    let custom = case.contains_key("custom");
    case["logs"]
        .as_array()
        .expect("logs")
        .iter()
        .map(|line| line.as_str().expect("a line"))
        .filter(move |line| {
            !custom
                || !CUSTOM_BUILDER_LINES
                    .iter()
                    .any(|start| line.starts_with(start))
        })
}

/// The lines of `case`'s `logs` that the pipeline writes, as Rust writes them (module docs).
pub(crate) fn expected_logs(case: &Map<String, Value>) -> Vec<String> {
    queue_lines(case)
        .filter(|line| !DELIVERY_LINES.iter().any(|start| line.starts_with(start)))
        .map(ts_line_as_rust)
        .collect()
}

/// The start of TS's line after the last failed attempt of an event.
const DELIVERY_FAILED: &str = "Failed to deliver AgentCat event after retries: ";

/// Where TS's line after a failed attempt embeds the attempt's error.
const ATTEMPT_ERROR: &str = "[Error: ";

/// A TS delivery line as Rust writes it (ruling P7, module docs): the error TS embeds after
/// [`ATTEMPT_ERROR`] (up to the closing `]`) or after [`DELIVERY_FAILED`] becomes its message
/// ([`js_error_message`]). Any other line stays as it is.
pub(crate) fn ts_delivery_line_as_rust(line: &str) -> String {
    if let Some(rendered) = line.strip_prefix(DELIVERY_FAILED) {
        return format!("{DELIVERY_FAILED}{}", js_error_message(rendered));
    }
    if line.starts_with(DELIVERY_LINES[1])
        && let Some(at) = line.find(ATTEMPT_ERROR)
        && let Some(rendered) = line[at + ATTEMPT_ERROR.len()..].strip_suffix(']')
    {
        let head = &line[..at + ATTEMPT_ERROR.len()];
        return format!("{head}{}]", js_error_message(rendered));
    }
    line.to_owned()
}

/// The message of a thrown JS `Error` as TS's `getMCPCompatibleErrorMessage` renders it: the
/// `message` of its JSON `{"stack": "Error: M", "message": M}`, or `M` of `Error: M` (ruling P7);
/// any other text as it is.
fn js_error_message(rendered: &str) -> String {
    if let Ok(Value::Object(error)) = serde_json::from_str::<Value>(rendered)
        && let Some(Value::String(message)) = error.get("message")
    {
        return message.clone();
    }
    rendered
        .strip_prefix("Error: ")
        .unwrap_or(rendered)
        .to_owned()
}

/// The lines of `case`'s `logs` that the delivery worker writes, the delivery lines included, as
/// Rust writes them: [`ts_line_as_rust`], then [`ts_delivery_line_as_rust`] (module docs).
pub(crate) fn worker_logs(case: &Map<String, Value>) -> Vec<String> {
    queue_lines(case)
        .map(|line| ts_delivery_line_as_rust(&ts_line_as_rust(line)))
        .collect()
}

/// Every line of `case`'s `logs`, as Rust writes it: [`ts_line_as_rust`], then
/// [`ts_delivery_line_as_rust`], with the custom-event builder's lines kept (module docs). The
/// runner of `custom::tests::custom_vectors_match_ts` compares these against all that the worker
/// writes.
pub(crate) fn full_logs(case: &Map<String, Value>) -> Vec<String> {
    case["logs"]
        .as_array()
        .expect("logs")
        .iter()
        .map(|line| ts_delivery_line_as_rust(&ts_line_as_rust(line.as_str().expect("a line"))))
        .collect()
}

/// The length of the body of an `evt_` ID (a KSUID in base 62).
const EVENT_ID_BODY: usize = 27;

/// `line` with each `evt_` ID (`evt_` and 27 ASCII letters and digits) as `<evt>`.
pub(crate) fn mask_event_ids(line: &str) -> String {
    let mut masked = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(at) = rest.find("evt_") {
        let body = &rest[at + 4..];
        let is_id = body.len() >= EVENT_ID_BODY
            && body.as_bytes()[..EVENT_ID_BODY]
                .iter()
                .all(u8::is_ascii_alphanumeric);
        masked.push_str(&rest[..at]);
        if is_id {
            masked.push_str("<evt>");
            rest = &body[EVENT_ID_BODY..];
        } else {
            masked.push_str("evt_");
            rest = body;
        }
    }
    masked.push_str(rest);
    masked
}

// ---------------------------------------------------------------------------------- requests

/// The wire fields of `PublishEventRequest` with the names of TS's model, in the order of TS's
/// `toPublishEventRequest`.
const MODEL_FIELDS: [(&str, &str); 28] = [
    ("id", "id"),
    ("project_id", "projectId"),
    ("session_id", "sessionId"),
    ("timestamp", "timestamp"),
    ("duration", "duration"),
    ("input_tokens", "inputTokens"),
    ("output_tokens", "outputTokens"),
    ("event_type", "eventType"),
    ("resource_name", "resourceName"),
    ("parameters", "parameters"),
    ("response", "response"),
    ("user_intent", "userIntent"),
    ("is_error", "isError"),
    ("error", "error"),
    ("identify_actor_given_id", "identifyActorGivenId"),
    ("identify_actor_name", "identifyActorName"),
    ("identify_data", "identifyData"),
    ("ip_address", "ipAddress"),
    ("sdk_language", "sdkLanguage"),
    ("agentcat_version", "agentcatVersion"),
    ("server_name", "serverName"),
    ("server_version", "serverVersion"),
    ("client_name", "clientName"),
    ("client_version", "clientVersion"),
    ("actor_id", "actorId"),
    ("event_id", "eventId"),
    ("tags", "tags"),
    ("properties", "properties"),
];

/// `tags` of `model`, when an object, with its keys sorted (through a `BTreeMap`: `Map::sort_keys`
/// is newer than the `serde_json` floor).
fn sort_tags(model: &mut Map<String, Value>) {
    if let Some(Value::Object(tags)) = model.get_mut("tags") {
        let sorted: BTreeMap<String, Value> = mem::take(tags).into_iter().collect();
        *tags = sorted.into_iter().collect();
    }
}

/// `request` as TS's camelCase model, from its `serde_json` text (module docs): the `evt_` ID as
/// `<evt>`, a session minted for it (its `agentcat_session_id_source` tag says `minted`) as
/// `<minted>`, the timestamp as `toISOString` writes it, the tags sorted.
///
/// # Panics
///
/// When the request has a field the model does not name, an ID that is not an `evt_` ID, a minted
/// session that is not a session ID, or a timestamp that does not parse.
pub(crate) fn to_ts_model(request: &PublishEventRequest) -> Value {
    let mut wire = object(serde_json::to_value(request).expect("a request is JSON"));
    let mut model = Map::new();
    for (field, name) in MODEL_FIELDS {
        if let Some(value) = wire.remove(field) {
            model.insert(name.to_owned(), value);
        }
    }
    assert!(
        wire.is_empty(),
        "fields the TS model does not name: {wire:?}"
    );

    let id = model["id"].as_str().expect("an id");
    let body = id.strip_prefix("evt_").expect("an evt_ ID");
    assert!(
        body.len() == 27 && body.bytes().all(|b| b.is_ascii_alphanumeric()),
        "{id} is not an evt_ ID"
    );
    model.insert("id".to_owned(), Value::from("<evt>"));

    if model
        .get("tags")
        .and_then(|tags| tags.get("agentcat_session_id_source"))
        == Some(&Value::from("minted"))
    {
        let session = model["sessionId"].as_str().expect("a session");
        assert!(is_valid_session_id(session), "{session}");
        model.insert("sessionId".to_owned(), Value::from("<minted>"));
    }

    if let Some(Value::String(timestamp)) = model.get_mut("timestamp") {
        let time = wire::parse_iso_millis(timestamp).expect("an RFC 3339 timestamp");
        *timestamp = wire::iso_millis(time);
    }
    sort_tags(&mut model);
    Value::Object(model)
}

/// The `requests` of `case`, compared as the module docs say: the tags sorted, a `null`
/// `userIntent` omitted (ruling P12), any other `userIntent` that is not a string as its
/// `JSON.stringify` text (RD-33).
pub(crate) fn expected_requests(case: &Map<String, Value>) -> Vec<Value> {
    case["requests"]
        .as_array()
        .expect("requests")
        .iter()
        .map(|request| {
            let mut request = object(request.clone());
            match request.get_mut("userIntent") {
                Some(Value::Null) => {
                    request.remove("userIntent");
                }
                Some(intent) if !intent.is_string() => {
                    *intent = Value::String(wire::stringify(intent));
                }
                _ => {}
            }
            sort_tags(&mut request);
            Value::Object(request)
        })
        .collect()
}

// ------------------------------------------------------------------------- delivery doubles

/// The text of each failed attempt of a [`RecordingPublisher`].
pub(crate) const MOCK_FAILURE: &str = "mock failure";

/// One attempt a [`RecordingPublisher`] saw: when it started and its request.
#[derive(Debug, Clone)]
pub(crate) struct Attempt {
    /// When the attempt started.
    pub(crate) at: Instant,
    /// The request of the attempt.
    pub(crate) request: PublishEventRequest,
}

/// A gate that holds every attempt of a [`RecordingPublisher`] until it opens.
#[derive(Debug, Clone)]
pub(crate) struct Gate(Arc<Semaphore>);

impl Gate {
    /// Lets every held attempt and every later one go on.
    pub(crate) fn open(&self) {
        self.0.close();
    }
}

/// What the clones of one [`RecordingPublisher`] share.
#[derive(Default)]
struct Recorded {
    /// How many of the next attempts fail with [`MOCK_FAILURE`].
    failures: Mutex<usize>,
    /// When set, each attempt waits until it opens.
    gate: Option<Gate>,
    /// How long each attempt takes once the gate lets it go.
    delay: Option<Duration>,
    /// Gets one message as each attempt starts.
    starts: Mutex<Option<mpsc::Sender<()>>>,
    /// Every attempt, in the order they started.
    attempts: Mutex<Vec<Attempt>>,
    /// The attempts running now.
    running: AtomicUsize,
    /// The most attempts that ran at once.
    most_running: AtomicUsize,
    /// The attempts that came to their end.
    finished: AtomicUsize,
}

/// A [`Publisher`] for tests that records every attempt and fails the first ones as told. Clones
/// share the record.
#[derive(Clone, Default)]
pub(crate) struct RecordingPublisher(Arc<Recorded>);

/// Takes one attempt off the running count when the attempt ends or is dropped.
struct Running(Arc<Recorded>);

impl Drop for Running {
    fn drop(&mut self) {
        self.0.running.fetch_sub(1, Ordering::SeqCst);
    }
}

impl RecordingPublisher {
    /// A publisher whose attempts all work.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// A publisher whose first `attempts` attempts fail with [`MOCK_FAILURE`].
    pub(crate) fn failing(attempts: usize) -> Self {
        Self::build(attempts, None, None)
    }

    /// A publisher whose attempts wait for the returned gate.
    pub(crate) fn gated() -> (Self, Gate) {
        let gate = Gate(Arc::new(Semaphore::new(0)));
        (Self::build(0, Some(gate.clone()), None), gate)
    }

    /// A publisher whose attempts each take `delay`.
    pub(crate) fn slow(delay: Duration) -> Self {
        Self::build(0, None, Some(delay))
    }

    fn build(failures: usize, gate: Option<Gate>, delay: Option<Duration>) -> Self {
        Self(Arc::new(Recorded {
            failures: Mutex::new(failures),
            gate,
            delay,
            ..Recorded::default()
        }))
    }

    /// A receiver that gets one message as each later attempt starts.
    pub(crate) fn starts(&self) -> mpsc::Receiver<()> {
        let (sender, receiver) = mpsc::channel();
        *self.0.starts.lock().unwrap_or_else(PoisonError::into_inner) = Some(sender);
        receiver
    }

    /// Every attempt so far, in the order they started.
    pub(crate) fn attempts(&self) -> Vec<Attempt> {
        self.0
            .attempts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The request of every attempt so far, in the order they started.
    pub(crate) fn requests(&self) -> Vec<PublishEventRequest> {
        self.attempts()
            .into_iter()
            .map(|attempt| attempt.request)
            .collect()
    }

    /// The `resource_name` of every attempt so far, in the order they started.
    pub(crate) fn resource_names(&self) -> Vec<String> {
        self.requests()
            .into_iter()
            .map(|request| request.resource_name.unwrap_or_default())
            .collect()
    }

    /// The most attempts that ran at once.
    pub(crate) fn most_running(&self) -> usize {
        self.0.most_running.load(Ordering::SeqCst)
    }

    /// The attempts that came to their end.
    pub(crate) fn finished(&self) -> usize {
        self.0.finished.load(Ordering::SeqCst)
    }
}

impl Publisher for RecordingPublisher {
    fn publish(&self, request: PublishEventRequest) -> BoxFuture<'static, Result<(), String>> {
        let recorded = Arc::clone(&self.0);
        Box::pin(async move {
            recorded
                .attempts
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(Attempt {
                    at: Instant::now(),
                    request,
                });
            let fail = {
                let mut left = recorded
                    .failures
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner);
                let fail = *left > 0;
                *left = left.saturating_sub(1);
                fail
            };
            let now = recorded.running.fetch_add(1, Ordering::SeqCst) + 1;
            recorded.most_running.fetch_max(now, Ordering::SeqCst);
            let _running = Running(Arc::clone(&recorded));
            if let Some(starts) = &*recorded
                .starts
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
            {
                let _ = starts.send(());
            }
            if let Some(Gate(gate)) = &recorded.gate {
                // The gate opens by closing: `acquire` then fails at once.
                let _ = gate.acquire().await;
            }
            if let Some(delay) = recorded.delay {
                tokio::time::sleep(delay).await;
            }
            recorded.finished.fetch_add(1, Ordering::SeqCst);
            if fail {
                Err(MOCK_FAILURE.to_owned())
            } else {
                Ok(())
            }
        })
    }
}

/// Sends one message when it is dropped. A hook's future that holds one and never returns shows
/// that its task was aborted: only an abort drops that future.
pub(crate) struct DropSignal(mpsc::Sender<()>);

impl DropSignal {
    /// A signal and the receiver of its message.
    pub(crate) fn new() -> (Self, mpsc::Receiver<()>) {
        let (sender, receiver) = mpsc::channel();
        (Self(sender), receiver)
    }
}

impl Drop for DropSignal {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

/// A log line and the name of the thread that wrote it.
type Entry = (String, Option<String>);

/// The log lines a delivery worker wrote through [`CapturedLines::writer`], each with the name of
/// the thread that wrote it, and the signal of each new line. Clones share the lines.
#[derive(Debug, Clone, Default)]
pub(crate) struct CapturedLines(Arc<(Mutex<Vec<Entry>>, Condvar)>);

impl CapturedLines {
    /// A writer that appends each line here and wakes [`CapturedLines::wait_for`].
    pub(crate) fn writer(&self) -> LineWriter {
        let lines = self.clone();
        Arc::new(move |line: &str| {
            let thread = std::thread::current().name().map(str::to_owned);
            let (entries, added) = &*lines.0;
            entries
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((line.to_owned(), thread));
            added.notify_all();
        })
    }

    /// Waits until there are `count` lines, woken by each new one, for at most `timeout`;
    /// `false` when fewer came.
    pub(crate) fn wait_for(&self, count: usize, timeout: Duration) -> bool {
        let (entries, added) = &*self.0;
        let entries = entries.lock().unwrap_or_else(PoisonError::into_inner);
        let (entries, _) = added
            .wait_timeout_while(entries, timeout, |entries| entries.len() < count)
            .unwrap_or_else(PoisonError::into_inner);
        entries.len() >= count
    }

    /// The lines so far, oldest first.
    pub(crate) fn lines(&self) -> Vec<String> {
        self.entries().into_iter().map(|(line, _)| line).collect()
    }

    /// The lines so far, oldest first, each with the name of the thread that wrote it.
    pub(crate) fn entries(&self) -> Vec<Entry> {
        self.0
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

mod tests {
    use serde_json::{Value, json};

    use super::{expected_requests, object};

    /// Ruling P12, RD-33: a TS `null` `userIntent` leaves the expected request, as the Rust wire
    /// omits it; any other value that is not a string becomes its `JSON.stringify` text; a string
    /// stays as it is.
    #[test]
    fn expected_user_intent_follows_the_wire() {
        let case = object(json!({
            "requests": [
                {"id": "<evt>", "userIntent": null},
                {"id": "<evt>", "userIntent": ["a", 1]},
                {"id": "<evt>", "userIntent": {"why": true}},
                {"id": "<evt>", "userIntent": "plain"},
                {"id": "<evt>"},
            ],
        }));
        let intents: Vec<Option<Value>> = expected_requests(&case)
            .iter()
            .map(|request| request.get("userIntent").cloned())
            .collect();
        assert_eq!(
            intents,
            [
                None,
                Some(json!("[\"a\",1]")),
                Some(json!("{\"why\":true}")),
                Some(json!("plain")),
                None,
            ]
        );
    }
}

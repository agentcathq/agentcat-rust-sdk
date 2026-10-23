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
//!   are the custom-event builder's (plan 4 task 7), so they leave too.
//! - One normalization (ruling P7): where TS writes a thrown `Error` after a stage's or a hook's
//!   text, as `Error: <message>`, Rust writes the error's `Display`, `<message>`
//!   ([`ts_line_as_rust`]). The truncation error needs none: Rust's `TruncateError` carries TS's
//!   V8 text (`TypeError: str.slice is not a function`), so that line compares whole, more than the
//!   prefix `Failed to truncate event: ` the ruling allows.
//!
//! Requests compare as [`to_ts_model`] writes the Rust request and [`expected_requests`] the TS
//! one: each `evt_` ID is `<evt>` and each minted session `<minted>` (as the generator masks
//! them), the timestamp is compared to the millisecond, tags compare as a map (sorted keys: the
//! Rust request holds them in a `HashMap`, RD-03), and a `userIntent` that is not a string is its
//! `JSON.stringify` text (RD-33).
use std::future::{pending, ready};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use agentcat_api::models::PublishEventRequest;
use serde_json::{Map, Value};
use tokio::task::JoinHandle;

use super::vectors;
use crate::config::{HookError, Options, UserIdentity};
use crate::event::Event;
use crate::handles::is_valid_session_id;
use crate::hooks::PendingHooks;
use crate::js;
use crate::pipeline::process::Redactors;

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

/// The JSON object `value`.
fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(object) => object,
        other => panic!("{other} is not an object"),
    }
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

// ---------------------------------------------------------------------------- redaction hooks

/// The redaction hooks of `case`: its `redact_event` (`{"returns": null}` drops the event,
/// `{"throws": message}` fails, `{"merge": fields}` returns `{...event, ...fields}`) and its
/// `redact_strings` (`{"replace": secret}` replaces each occurrence with `[REDACTED]`,
/// `{"throw_at": n}` fails on call `n` with `redactor failed` and returns the other strings).
pub(crate) fn redactors_of(case: &Map<String, Value>) -> Redactors {
    let mut options = Options::default();
    if let Some(spec) = case.get("redact_event") {
        let spec = spec.clone();
        options = options.redact_event(move |event: Event| {
            let result: Result<Option<Event>, HookError> = if spec.get("throws").is_some() {
                Err(spec["throws"].as_str().expect("a message").into())
            } else if let Some(Value::Object(fields)) = spec.get("merge") {
                let mut merged = event.into_ts();
                merged.extend(fields.clone());
                Ok(Some(Event::from_ts(&merged)))
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
    Redactors {
        string: options.string_redactor().cloned(),
        event: options.event_redactor().cloned(),
    }
}

// --------------------------------------------------------------------------------- log lines

/// The starts of the delivery worker's lines (module docs).
const DELIVERY_LINES: [&str; 2] = ["Successfully sent event ", "Failed to send event "];

/// The starts of the lines `publishCustomEvent` writes before the queue takes the event (module
/// docs).
const CUSTOM_BUILDER_LINES: [&str; 3] = [
    "Published custom event ",
    "publishCustomEvent: ",
    "Dropping invalid tag: ",
];

/// The texts after which TS writes a thrown `Error` (the hooks' and the stages'): the text of each
/// call hook's `catch`, then the queue's stage lines.
const ERROR_PREFIXES: [&str; 8] = [
    "resolveSessionId hook error: ",
    "Error: identify callback threw - ",
    "eventTags callback error: ",
    "eventProperties callback error: ",
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

/// The lines of `case`'s `logs` that the pipeline writes, as Rust writes them (module docs).
pub(crate) fn expected_logs(case: &Map<String, Value>) -> Vec<String> {
    let custom = case.contains_key("custom");
    case["logs"]
        .as_array()
        .expect("logs")
        .iter()
        .map(|line| line.as_str().expect("a line"))
        .filter(|line| !DELIVERY_LINES.iter().any(|start| line.starts_with(start)))
        .filter(|line| {
            !custom
                || !CUSTOM_BUILDER_LINES
                    .iter()
                    .any(|start| line.starts_with(start))
        })
        .map(ts_line_as_rust)
        .collect()
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

/// `tags` of `model`, when an object, with its keys sorted.
fn sort_tags(model: &mut Map<String, Value>) {
    if let Some(Value::Object(tags)) = model.get_mut("tags") {
        tags.sort_keys();
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
        let time = js::parse_iso_millis(timestamp).expect("an RFC 3339 timestamp");
        *timestamp = js::iso_millis(time);
    }
    sort_tags(&mut model);
    Value::Object(model)
}

/// The `requests` of `case`, compared as the module docs say: the tags sorted, a `userIntent` that
/// is not a string as its `JSON.stringify` text (RD-33).
pub(crate) fn expected_requests(case: &Map<String, Value>) -> Vec<Value> {
    case["requests"]
        .as_array()
        .expect("requests")
        .iter()
        .map(|request| {
            let mut request = object(request.clone());
            if let Some(intent) = request.get_mut("userIntent")
                && !intent.is_string()
            {
                *intent = Value::String(js::stringify(intent));
            }
            sort_tags(&mut request);
            Value::Object(request)
        })
        .collect()
}

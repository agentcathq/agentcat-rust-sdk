//! [`CustomEvent`]: an event the server publishes by itself, apart from tool calls (B-009), and
//! the two forms that publish it (C11, C16, C47): [`publish_string`] for the session-string form
//! on an [`AgentCat`] handle and [`publish_server`] for the server form on a
//! [`CallContext`](crate::CallContext) or a [`Tracked`](crate::Tracked). Both only queue the event.
use std::time::{Duration, SystemTime};

use rmcp::model::Implementation;
use serde_json::{Map, Value};
use tokio::runtime::Handle;

use crate::client::AgentCat;
use crate::error::Error;
use crate::event::ClientIdentity;
use crate::event::assemble::{EventJob, SDK_LANGUAGE};
use crate::hooks::PendingHooks;
use crate::js;
use crate::pipeline::TsEvent;
use crate::pipeline::tags::validate_tags;

#[cfg(test)]
mod tests;

/// The data of a custom event, as TypeScript's `CustomEventData`. Every field is optional; each
/// setter consumes the event and returns it changed.
///
/// ```
/// use std::time::Duration;
/// use agentcat_sdk::CustomEvent;
///
/// let event = CustomEvent::new()
///     .session_id("ses_1")
///     .message("cache warmed")
///     .duration(Duration::from_millis(40));
/// # let _ = event;
/// ```
#[derive(Clone, Debug, Default)]
pub struct CustomEvent {
    pub(crate) session_id: Option<String>,
    pub(crate) resource_name: Option<String>,
    pub(crate) parameters: Option<Value>,
    pub(crate) response: Option<Value>,
    pub(crate) message: Option<String>,
    pub(crate) duration: Option<Duration>,
    pub(crate) is_error: Option<bool>,
    pub(crate) error: Option<Value>,
    pub(crate) tags: Option<Map<String, Value>>,
    pub(crate) properties: Option<Map<String, Value>>,
}

impl CustomEvent {
    /// An event with no data.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The session the event belongs to.
    #[must_use]
    pub fn session_id(mut self, id: impl Into<String>) -> Self {
        self.session_id = Some(id.into());
        self
    }

    /// The name of the resource the event is about.
    #[must_use]
    pub fn resource_name(mut self, name: impl Into<String>) -> Self {
        self.resource_name = Some(name.into());
        self
    }

    /// The parameters of what happened.
    #[must_use]
    pub fn parameters(mut self, parameters: Value) -> Self {
        self.parameters = Some(parameters);
        self
    }

    /// The response of what happened.
    #[must_use]
    pub fn response(mut self, response: Value) -> Self {
        self.response = Some(response);
        self
    }

    /// A free text about the event.
    #[must_use]
    pub fn message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }

    /// How long it took.
    #[must_use]
    pub fn duration(mut self, duration: Duration) -> Self {
        self.duration = Some(duration);
        self
    }

    /// Whether the event is an error.
    #[must_use]
    pub fn is_error(mut self, is_error: bool) -> Self {
        self.is_error = Some(is_error);
        self
    }

    /// The error, when there is one.
    #[must_use]
    pub fn error(mut self, error: Value) -> Self {
        self.error = Some(error);
        self
    }

    /// Tags on the event.
    #[must_use]
    pub fn tags(mut self, tags: Map<String, Value>) -> Self {
        self.tags = Some(tags);
        self
    }

    /// Properties on the event.
    #[must_use]
    pub fn properties(mut self, properties: Map<String, Value>) -> Self {
        self.properties = Some(properties);
        self
    }
}

/// The event type of every custom event (B-009).
const CUSTOM_EVENT_TYPE: &str = "agentcat:custom";

/// TS's text for a server-form event without a session.
const NO_SESSION_WARNING: &str = "publishCustomEvent: no sessionId provided; event will be published without a session. Pass eventData.sessionId to attribute it to a task.";

/// Sets `key` of `event` when there is a value: TS's `undefined` leaves no key.
fn put(event: &mut TsEvent, key: &str, value: Option<Value>) {
    if let Some(value) = value {
        event.insert(key.to_owned(), value);
    }
}

/// A duration as TS holds it: whole milliseconds.
fn millis(duration: Duration) -> Value {
    Value::from(u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
}

/// The data fields that both forms write after the session, in TS's order up to `userIntent`.
fn put_payload(
    event: &mut TsEvent,
    resource_name: Option<String>,
    parameters: Option<Value>,
    response: Option<Value>,
    message: Option<String>,
) {
    put(event, "resourceName", resource_name.map(Value::String));
    put(event, "parameters", parameters);
    put(event, "response", response);
    put(event, "userIntent", message.map(Value::String));
}

/// The customer's `tags` and `properties`, last in the event: `tags` is the key with the valid
/// tags, or `null` when none is valid, whenever the customer gave any (TS's `if (eventData.tags)`);
/// `properties` only with at least one key. The log lines of dropped tags go to `log`.
fn put_metadata(
    event: &mut TsEvent,
    tags: Option<Map<String, Value>>,
    properties: Option<Map<String, Value>>,
    log: &mut Vec<String>,
) {
    if let Some(tags) = tags {
        let valid = validate_tags(&tags, log);
        event.insert("tags".to_owned(), valid.map_or(Value::Null, Value::Object));
    }
    if let Some(properties) = properties.filter(|properties| !properties.is_empty()) {
        event.insert("properties".to_owned(), Value::Object(properties));
    }
}

/// Hands the log lines, then (when there is one) the event followed by TS's `Published custom
/// event` line, to the handle's sink.
fn hand_over(agentcat: &AgentCat, log: Vec<String>, job: Option<EventJob>, session_id: &str) {
    let sink = &agentcat.inner().sink;
    for line in log {
        sink.log(line);
    }
    let whose = if session_id.is_empty() {
        "without a session".to_owned()
    } else {
        format!("for session {session_id}")
    };
    let published = format!("Published custom event {whose} with type '{CUSTOM_EVENT_TYPE}'");
    match job {
        Some(job) => sink.event_then_log(job, published),
        None => sink.log(published),
    }
}

/// The session-string form (B-009, C11): the event goes to the handle's queue with the project
/// `project_id` as given, the session of the event's own `session_id` or else `session_id`
/// verbatim, no redaction hook and no server or client fields. `now` is the event's timestamp.
///
/// # Errors
///
/// [`Error::ProjectIdRequired`] for an empty `project_id`.
pub(crate) fn publish_string(
    agentcat: &AgentCat,
    session_id: &str,
    project_id: &str,
    event: CustomEvent,
    now: SystemTime,
) -> Result<(), Error> {
    if project_id.is_empty() {
        return Err(Error::ProjectIdRequired);
    }
    let CustomEvent {
        session_id: own_session,
        resource_name,
        parameters,
        response,
        message,
        duration,
        is_error,
        error,
        tags,
        properties,
    } = event;
    let session = own_session
        .filter(|own| !own.is_empty())
        .unwrap_or_else(|| session_id.to_owned());

    let mut log = Vec::new();
    let mut ts = TsEvent::new();
    ts.insert("sessionId".to_owned(), Value::String(session.clone()));
    ts.insert("projectId".to_owned(), Value::String(project_id.to_owned()));
    ts.insert("eventType".to_owned(), Value::from(CUSTOM_EVENT_TYPE));
    ts.insert("timestamp".to_owned(), Value::String(js::iso_millis(now)));
    put_payload(&mut ts, resource_name, parameters, response, message);
    put(&mut ts, "duration", duration.map(millis));
    put(&mut ts, "isError", is_error.map(Value::Bool));
    put(&mut ts, "error", error);
    put_metadata(&mut ts, tags, properties, &mut log);

    let job = EventJob {
        event: ts,
        hooks: PendingHooks::default(),
        hook_mode: false,
        runtime: None,
        string_redaction: false,
        event_redaction: false,
        backtrace: None,
    };
    hand_over(agentcat, log, Some(job), &session);
    Ok(())
}

/// The server form (B-009, C16, C47, K-03): as TS's `publishEvent` builds it, with the handle's
/// project (`project_id` only has to be non-empty), the event's own session or none, the given
/// client and server, the event hook of the options (never the string hook) on the runtime of the
/// caller, and nothing queued while tracing is off. `created` is the event's timestamp and
/// `published` the moment of `publishEvent`: a missing or zero `duration` becomes the
/// milliseconds between them (TS's `duration || elapsed`).
///
/// # Errors
///
/// [`Error::ProjectIdRequired`] for an empty `project_id`.
pub(crate) fn publish_server(
    agentcat: &AgentCat,
    project_id: &str,
    event: CustomEvent,
    client: Option<&ClientIdentity>,
    server: Option<&Implementation>,
    created: SystemTime,
    published: SystemTime,
) -> Result<(), Error> {
    if project_id.is_empty() {
        return Err(Error::ProjectIdRequired);
    }
    let CustomEvent {
        session_id: own_session,
        resource_name,
        parameters,
        response,
        message,
        duration,
        is_error,
        error,
        tags,
        properties,
    } = event;
    let mut log = Vec::new();
    let session = own_session
        .filter(|own| !own.is_empty())
        .unwrap_or_else(|| {
            log.push(NO_SESSION_WARNING.to_owned());
            String::new()
        });
    let mut custom = TsEvent::new();
    put_metadata(&mut custom, tags, properties, &mut log);

    let shared = agentcat.inner();
    if !shared.options.tracing() {
        hand_over(agentcat, log, None, &session);
        return Ok(());
    }

    let duration = duration
        .filter(|given| !given.is_zero())
        .unwrap_or_else(|| published.duration_since(created).unwrap_or_default());
    let text = |value: &str| Value::String(value.to_owned());
    let mut ts = TsEvent::new();
    ts.insert("id".to_owned(), text(""));
    ts.insert("sessionId".to_owned(), Value::String(session.clone()));
    ts.insert("projectId".to_owned(), text(&shared.project_id));
    ts.insert("eventType".to_owned(), text(CUSTOM_EVENT_TYPE));
    ts.insert(
        "timestamp".to_owned(),
        Value::String(js::iso_millis(created)),
    );
    ts.insert("duration".to_owned(), millis(duration));
    ts.insert("sdkLanguage".to_owned(), text(SDK_LANGUAGE));
    ts.insert(
        "agentcatVersion".to_owned(),
        text(env!("CARGO_PKG_VERSION")),
    );
    put(&mut ts, "serverName", server.map(|info| text(&info.name)));
    put(
        &mut ts,
        "serverVersion",
        server.map(|info| text(&info.version)),
    );
    put(
        &mut ts,
        "clientName",
        client.and_then(|client| client.name.as_deref()).map(text),
    );
    put(
        &mut ts,
        "clientVersion",
        client
            .and_then(|client| client.version.as_deref())
            .map(text),
    );
    ts.insert("identifyActorData".to_owned(), Value::Object(Map::new()));
    put_payload(&mut ts, resource_name, parameters, response, message);
    put(&mut ts, "isError", is_error.map(Value::Bool));
    put(&mut ts, "error", error);
    ts.extend(custom);

    let job = EventJob {
        event: ts,
        hooks: PendingHooks::default(),
        hook_mode: false,
        runtime: Handle::try_current().ok(),
        string_redaction: false,
        event_redaction: true,
        backtrace: None,
    };
    hand_over(agentcat, log, Some(job), &session);
    Ok(())
}

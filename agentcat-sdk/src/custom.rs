//! [`CustomEvent`]: an event the server publishes by itself, apart from tool calls, and the two
//! forms that publish it: [`publish_string`] for the session-string form
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
use crate::pipeline::EventObject;
use crate::pipeline::tags::validate_tags;
use crate::wire;

#[cfg(test)]
mod tests;

/// The data of a custom event. Every field is optional; each setter consumes the event and
/// returns it changed.
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

/// The event type of every custom event.
const CUSTOM_EVENT_TYPE: &str = "agentcat:custom";

/// The warning that a server-form event without a session logs.
const NO_SESSION_WARNING: &str = "publish_custom_event: no session_id provided; event will be published without a session. Pass CustomEvent::session_id to attribute it to a task.";

/// Sets `key` of `event` when there is a value; `None` leaves no key.
fn put(event: &mut EventObject, key: &str, value: Option<Value>) {
    if let Some(value) = value {
        event.insert(key.to_owned(), value);
    }
}

/// A duration as an event holds it: whole milliseconds, at most `u64::MAX`.
fn millis(duration: Duration) -> Value {
    Value::from(u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
}

/// The data fields that both forms write after the session, in the event's key order up to
/// `userIntent`.
struct Payload {
    resource_name: Option<String>,
    parameters: Option<Value>,
    response: Option<Value>,
    message: Option<String>,
}

/// Writes `payload` into `event`, in this key order: `resourceName`, `parameters`, `response`,
/// `userIntent`.
fn put_payload(event: &mut EventObject, payload: Payload) {
    let Payload {
        resource_name,
        parameters,
        response,
        message,
    } = payload;
    put(event, "resourceName", resource_name.map(Value::String));
    put(event, "parameters", parameters);
    put(event, "response", response);
    put(event, "userIntent", message.map(Value::String));
}

/// A [`CustomEvent`] that passed the checks of both forms ([`accept`]), taken apart once.
struct Accepted {
    /// The event's own session, when it is not empty.
    session_id: Option<String>,
    payload: Payload,
    duration: Option<Duration>,
    is_error: Option<bool>,
    error: Option<Value>,
    tags: Option<Map<String, Value>>,
    properties: Option<Map<String, Value>>,
}

/// The checks both forms make before they build anything: `Ok(None)` on a handle that
/// `AGENTCAT_ENABLED` turned off, else `event` taken apart.
///
/// # Errors
///
/// [`Error::ProjectIdRequired`] for an empty `project_id` on a handle that is on.
fn accept(
    agentcat: &AgentCat,
    project_id: &str,
    event: CustomEvent,
) -> Result<Option<Accepted>, Error> {
    if !agentcat.inner().enabled {
        return Ok(None);
    }
    if project_id.is_empty() {
        return Err(Error::ProjectIdRequired);
    }
    let CustomEvent {
        session_id,
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
    Ok(Some(Accepted {
        session_id: session_id.filter(|own| !own.is_empty()),
        payload: Payload {
            resource_name,
            parameters,
            response,
            message,
        },
        duration,
        is_error,
        error,
        tags,
        properties,
    }))
}

/// The customer's `tags` and `properties`, last in the event: `tags` is the key with the valid
/// tags, or `null` when none is valid, whenever the customer set tags (an empty map too);
/// `properties` only with at least one key. The log lines of dropped tags go to `log`.
fn put_metadata(
    event: &mut EventObject,
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

/// Hands the log lines, then (when there is one) the event followed by the `Published custom
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

/// The session-string form: the event goes to the handle's queue with the project
/// `project_id` as given, the session of the event's own `session_id` or else `session_id`
/// verbatim, no redaction hook and no server or client fields. `now` is the event's timestamp.
/// A handle that `AGENTCAT_ENABLED` turned off returns `Ok(())` at once and builds nothing.
///
/// # Errors
///
/// [`Error::ProjectIdRequired`] for an empty `project_id` on a handle that is on.
pub(crate) fn publish_string(
    agentcat: &AgentCat,
    session_id: &str,
    project_id: &str,
    event: CustomEvent,
    now: SystemTime,
) -> Result<(), Error> {
    let Some(event) = accept(agentcat, project_id, event)? else {
        return Ok(());
    };
    let session = event.session_id.unwrap_or_else(|| session_id.to_owned());

    let mut log = Vec::new();
    let mut object = EventObject::new();
    object.insert("sessionId".to_owned(), Value::String(session.clone()));
    object.insert("projectId".to_owned(), Value::String(project_id.to_owned()));
    object.insert("eventType".to_owned(), Value::from(CUSTOM_EVENT_TYPE));
    object.insert("timestamp".to_owned(), Value::String(wire::iso_millis(now)));
    put_payload(&mut object, event.payload);
    put(&mut object, "duration", event.duration.map(millis));
    put(&mut object, "isError", event.is_error.map(Value::Bool));
    put(&mut object, "error", event.error);
    put_metadata(&mut object, event.tags, event.properties, &mut log);

    let job = EventJob {
        event: object,
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

/// The server form: the event with the handle's project (`project_id` only has to be non-empty),
/// the event's own session or none, the given client and server, the event hook of the options
/// (never the string hook) on the runtime of the caller, and nothing queued while tracing is off.
/// `created` is the event's timestamp and `published` the moment the event is published: a
/// missing or zero `duration` becomes the milliseconds between them. A handle that
/// `AGENTCAT_ENABLED` turned off returns `Ok(())` at once and builds nothing.
///
/// # Errors
///
/// [`Error::ProjectIdRequired`] for an empty `project_id` on a handle that is on.
pub(crate) fn publish_server(
    agentcat: &AgentCat,
    project_id: &str,
    event: CustomEvent,
    client: Option<&ClientIdentity>,
    server: Option<&Implementation>,
    created: SystemTime,
    published: SystemTime,
) -> Result<(), Error> {
    let Some(event) = accept(agentcat, project_id, event)? else {
        return Ok(());
    };
    let mut log = Vec::new();
    let session = event.session_id.unwrap_or_else(|| {
        log.push(NO_SESSION_WARNING.to_owned());
        String::new()
    });
    let mut custom = EventObject::new();
    put_metadata(&mut custom, event.tags, event.properties, &mut log);

    let shared = agentcat.inner();
    if !shared.options.tracing() {
        hand_over(agentcat, log, None, &session);
        return Ok(());
    }

    let duration = event
        .duration
        .filter(|given| !given.is_zero())
        .unwrap_or_else(|| published.duration_since(created).unwrap_or_default());
    let text = |value: &str| Value::String(value.to_owned());
    let mut object = EventObject::new();
    object.insert("id".to_owned(), text(""));
    object.insert("sessionId".to_owned(), Value::String(session.clone()));
    object.insert("projectId".to_owned(), text(&shared.project_id));
    object.insert("eventType".to_owned(), text(CUSTOM_EVENT_TYPE));
    object.insert(
        "timestamp".to_owned(),
        Value::String(wire::iso_millis(created)),
    );
    object.insert("duration".to_owned(), millis(duration));
    object.insert("sdkLanguage".to_owned(), text(SDK_LANGUAGE));
    object.insert(
        "agentcatVersion".to_owned(),
        text(env!("CARGO_PKG_VERSION")),
    );
    put(
        &mut object,
        "serverName",
        server.map(|info| text(&info.name)),
    );
    put(
        &mut object,
        "serverVersion",
        server.map(|info| text(&info.version)),
    );
    put(
        &mut object,
        "clientName",
        client.and_then(|client| client.name.as_deref()).map(text),
    );
    put(
        &mut object,
        "clientVersion",
        client
            .and_then(|client| client.version.as_deref())
            .map(text),
    );
    object.insert("identifyActorData".to_owned(), Value::Object(Map::new()));
    put_payload(&mut object, event.payload);
    put(&mut object, "isError", event.is_error.map(Value::Bool));
    put(&mut object, "error", event.error);
    object.extend(custom);

    let job = EventJob {
        event: object,
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

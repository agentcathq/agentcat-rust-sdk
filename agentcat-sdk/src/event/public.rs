//! [`Event`]: the event a `redact_event` hook sees and returns, typed, and its conversion from and
//! into the pipeline's [`EventObject`].
use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};

use crate::pipeline::EventObject;
use crate::wire;

/// An event as `AgentCat` publishes it, as the hook of
/// [`Options::redact_event`](crate::Options::redact_event) sees it. The hook may change any
/// field. It cannot add a field that events do not have, and the SDK restores the system fields
/// (`id`, `session_id`, `project_id`, `event_type` and `timestamp`) when the hook changes them.
///
/// The free-form fields (`parameters`, `response`, `user_intent`, `error`, `identify_actor_data`
/// and `properties`) hold JSON values, as the published event carries them.
///
/// To unit-test a `redact_event` hook, build an event with [`Event::default`], set the fields
/// the hook reads, and call the hook with it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Event {
    /// The ID of the event.
    pub id: String,
    /// The session the event belongs to.
    pub session_id: Option<String>,
    /// The project the event is published to.
    pub project_id: String,
    /// The kind of event, such as `mcp:tools/call`.
    pub event_type: String,
    /// When the event happened.
    pub timestamp: SystemTime,
    /// How long it took.
    pub duration: Option<Duration>,
    /// The estimated tokens of the request.
    pub input_tokens: Option<u32>,
    /// The estimated tokens of the response.
    pub output_tokens: Option<u32>,
    /// The language of the SDK: `"Rust"`.
    pub sdk_language: Option<String>,
    /// The version of the SDK.
    pub agentcat_version: Option<String>,
    /// The name of the MCP server.
    pub server_name: Option<String>,
    /// The version of the MCP server.
    pub server_version: Option<String>,
    /// The name of the MCP client.
    pub client_name: Option<String>,
    /// The version of the MCP client.
    pub client_version: Option<String>,
    /// The ID of the caller in the customer's system.
    pub identify_actor_given_id: Option<String>,
    /// The display name of the caller.
    pub identify_actor_name: Option<String>,
    /// Other data about the caller.
    pub identify_actor_data: Option<Map<String, Value>>,
    /// The name of the tool or resource.
    pub resource_name: Option<String>,
    /// The parameters of the call.
    pub parameters: Option<Value>,
    /// The response of the call.
    pub response: Option<Value>,
    /// Why the agent made the call.
    pub user_intent: Option<Value>,
    /// Whether the call failed.
    pub is_error: Option<bool>,
    /// The error of the call.
    pub error: Option<Value>,
    /// The tags of the event.
    pub tags: Option<BTreeMap<String, String>>,
    /// The properties of the event.
    pub properties: Option<Map<String, Value>>,
}

impl Default for Event {
    /// An empty event: `id`, `project_id` and `event_type` are empty, `timestamp` is the Unix
    /// epoch, and every other field is `None`. Set the fields a test needs:
    ///
    /// ```
    /// use agentcat_sdk::Event;
    /// use serde_json::json;
    ///
    /// let mut event = Event::default();
    /// event.parameters = Some(json!({"api_key": "fake-secret-42"}));
    /// # assert!(event.response.is_none());
    /// ```
    fn default() -> Self {
        Self {
            id: String::new(),
            session_id: None,
            project_id: String::new(),
            event_type: String::new(),
            timestamp: UNIX_EPOCH,
            duration: None,
            input_tokens: None,
            output_tokens: None,
            sdk_language: None,
            agentcat_version: None,
            server_name: None,
            server_version: None,
            client_name: None,
            client_version: None,
            identify_actor_given_id: None,
            identify_actor_name: None,
            identify_actor_data: None,
            resource_name: None,
            parameters: None,
            response: None,
            user_intent: None,
            is_error: None,
            error: None,
            tags: None,
            properties: None,
        }
    }
}

impl Event {
    /// The typed event of `event`. Keys the type has no field for are left out; a field of the
    /// wrong type is `None`; a `timestamp` that does not parse is the Unix epoch, which the
    /// restore of `timestamp` after the hook makes harmless.
    pub(crate) fn from_object(event: &EventObject) -> Event {
        let text = |key: &str| event.get(key).and_then(Value::as_str).map(str::to_owned);
        let object = |key: &str| event.get(key).and_then(Value::as_object).cloned();
        let value = |key: &str| event.get(key).cloned();
        let tokens = |key: &str| {
            event
                .get(key)
                .and_then(Value::as_u64)
                .and_then(|n| u32::try_from(n).ok())
        };
        Event {
            id: text("id").unwrap_or_default(),
            session_id: text("sessionId").filter(|id| !id.is_empty()),
            project_id: text("projectId").unwrap_or_default(),
            event_type: text("eventType").unwrap_or_default(),
            timestamp: event
                .get("timestamp")
                .and_then(Value::as_str)
                .and_then(wire::parse_iso_millis)
                .unwrap_or(UNIX_EPOCH),
            duration: event
                .get("duration")
                .and_then(Value::as_u64)
                .map(Duration::from_millis),
            input_tokens: tokens("inputTokens"),
            output_tokens: tokens("outputTokens"),
            sdk_language: text("sdkLanguage"),
            agentcat_version: text("agentcatVersion"),
            server_name: text("serverName"),
            server_version: text("serverVersion"),
            client_name: text("clientName"),
            client_version: text("clientVersion"),
            identify_actor_given_id: text("identifyActorGivenId"),
            identify_actor_name: text("identifyActorName"),
            identify_actor_data: object("identifyActorData"),
            resource_name: text("resourceName"),
            parameters: value("parameters"),
            response: value("response"),
            user_intent: value("userIntent"),
            is_error: event.get("isError").and_then(Value::as_bool),
            error: value("error"),
            tags: object("tags").map(|tags| {
                tags.into_iter()
                    .filter_map(|(key, tag)| Some((key, tag.as_str()?.to_owned())))
                    .collect()
            }),
            properties: object("properties"),
        }
    }

    /// The pipeline's event, with the keys in the order of a tool-call event (`id`,
    /// `sessionId`, `projectId`, `eventType`, `timestamp`, `duration`, ...). A field that is
    /// `None` is absent, except `sessionId`, which is `""` without a session. Custom events with
    /// a session string skip the redaction hooks, so only this order is needed.
    pub(crate) fn into_object(self) -> EventObject {
        let mut out = EventObject::new();
        let mut put = |key: &str, value: Option<Value>| {
            if let Some(value) = value {
                out.insert(key.to_owned(), value);
            }
        };
        let text = |value: Option<String>| value.map(Value::String);
        put("id", Some(Value::String(self.id)));
        put(
            "sessionId",
            Some(Value::String(self.session_id.unwrap_or_default())),
        );
        put("projectId", Some(Value::String(self.project_id)));
        put("eventType", Some(Value::String(self.event_type)));
        put(
            "timestamp",
            Some(Value::String(wire::iso_millis(self.timestamp))),
        );
        put(
            "duration",
            self.duration
                .and_then(|d| u64::try_from(d.as_millis()).ok())
                .map(Value::from),
        );
        put("inputTokens", self.input_tokens.map(Value::from));
        put("outputTokens", self.output_tokens.map(Value::from));
        put("sdkLanguage", text(self.sdk_language));
        put("agentcatVersion", text(self.agentcat_version));
        put("serverName", text(self.server_name));
        put("serverVersion", text(self.server_version));
        put("clientName", text(self.client_name));
        put("clientVersion", text(self.client_version));
        put("identifyActorGivenId", text(self.identify_actor_given_id));
        put("identifyActorName", text(self.identify_actor_name));
        put(
            "identifyActorData",
            self.identify_actor_data.map(Value::Object),
        );
        put("resourceName", text(self.resource_name));
        put("parameters", self.parameters);
        put("response", self.response);
        put("userIntent", self.user_intent);
        put("isError", self.is_error.map(Value::Bool));
        put("error", self.error);
        put(
            "tags",
            self.tags.map(|tags| {
                Value::Object(
                    tags.into_iter()
                        .map(|(key, tag)| (key, Value::String(tag)))
                        .collect(),
                )
            }),
        );
        put("properties", self.properties.map(Value::Object));
        out
    }
}

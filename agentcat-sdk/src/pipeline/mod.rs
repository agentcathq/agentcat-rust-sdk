//! The event pipeline: the settling of the call hooks and the application of their results, tag
//! validation, the event-level redaction hook's restore, string redaction, sanitization,
//! normalization and truncation.
//!
//! The pipeline works on the event as the delivery queue holds it ([`EventObject`]): a JSON object
//! with camelCase field names (`sessionId`, `eventType`, `userIntent`, `inputTokens`, ...), which
//! [`publish_request`](crate::delivery::request::publish_request) maps to the snake-case fields of
//! the publish request (`session_id`, `event_type`, ...). Its `timestamp` is an ISO-8601 string
//! with milliseconds, and the redaction walk treats that string as a time, not as text: it keeps it
//! and does not pass it to the redaction hook. Field values are JSON values; a field without a
//! value is an absent key. The keys are in the order of [`TOOL_CALL_KEYS`], which decides, for
//! example, which of two strings of equal length the size guard cuts first.
use serde_json::{Map, Value};

pub(crate) mod normalize;
pub(crate) mod process;
pub(crate) mod redact;
pub(crate) mod sanitize;
pub(crate) mod settle;
pub(crate) mod tags;
pub(crate) mod truncate;

/// The event as the delivery queue holds it (see the module docs).
pub(crate) type EventObject = Map<String, Value>;

/// The keys of a tool-call event, in their order (`tests/vectors/event_order.json`, case
/// `tool_call`), `ipAddress` included, which the SDK never sets.
pub(crate) const TOOL_CALL_KEYS: [&str; 26] = [
    "id",
    "sessionId",
    "projectId",
    "eventType",
    "timestamp",
    "duration",
    "inputTokens",
    "outputTokens",
    "ipAddress",
    "sdkLanguage",
    "agentcatVersion",
    "serverName",
    "serverVersion",
    "clientName",
    "clientVersion",
    "identifyActorGivenId",
    "identifyActorName",
    "identifyActorData",
    "resourceName",
    "parameters",
    "response",
    "userIntent",
    "isError",
    "error",
    "tags",
    "properties",
];

/// Sets `key` of `event` to `value`. A key the event has keeps its place. A new key goes to its
/// place in [`TOOL_CALL_KEYS`], before the first key of the event that comes after it there, as if
/// the event held each of these keys from the start, with no value where JSON has none. A key
/// outside that list goes last.
pub(crate) fn set_field(event: &mut EventObject, key: &str, value: Value) {
    if let Some(slot) = event.get_mut(key) {
        *slot = value;
        return;
    }
    let rank = |name: &str| TOOL_CALL_KEYS.iter().position(|known| *known == name);
    let index = rank(key)
        .and_then(|own| {
            event
                .keys()
                .position(|present| rank(present).is_some_and(|other| other > own))
        })
        .unwrap_or(event.len());
    event.shift_insert(index, key.to_owned(), value);
}

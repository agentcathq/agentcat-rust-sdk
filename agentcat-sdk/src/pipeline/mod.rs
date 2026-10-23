//! The event pipeline of the TS SDK: the settling of the call hooks and the application of their
//! results, tag validation, the event-level redaction hook's restore, string redaction,
//! sanitization, normalization and truncation.
//!
//! The pipeline works on the event as TS holds it in memory when the queue runs ([`TsEvent`]): a
//! JSON object with TS's camelCase field names (`sessionId`, `eventType`, `userIntent`,
//! `inputTokens`, ...). Its `timestamp` is an ISO-8601 string with milliseconds, and the
//! redaction walk treats that string as TS treats a `Date`: it keeps it and does not pass it to
//! the redaction hook. Field values are JSON values; TS values that JSON cannot hold (functions,
//! `undefined`) are absent keys. The keys are in TS's order ([`TOOL_CALL_KEYS`]), which decides,
//! for example, which of two strings of equal length the size guard cuts first.
use serde_json::{Map, Value};

pub(crate) mod normalize;
pub(crate) mod redact;
pub(crate) mod sanitize;
pub(crate) mod settle;
pub(crate) mod tags;
pub(crate) mod truncate;

/// The event as TS holds it in memory (see the module docs).
pub(crate) type TsEvent = Map<String, Value>;

/// The keys of TS's in-memory tool-call event, in TS's order (`tests/vectors/event_order.json`,
/// case `tool_call`): the order in which `publishEvent` writes them, `ipAddress` included, which
/// TS leaves undefined and Rust never sets.
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

/// Sets `key` of `event` to `value` as a JavaScript assignment to a TS event does. A key the
/// event has keeps its place. A new key goes to its place in [`TOOL_CALL_KEYS`], before the
/// first key of the event that comes after it there: TS's event holds each of these keys from the
/// start, with an undefined value where JSON has none. A key outside that list goes last.
pub(crate) fn set_field(event: &mut TsEvent, key: &str, value: Value) {
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

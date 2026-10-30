//! The redaction: the walk that passes the event's strings to the customer's string-redaction
//! hook, and the restore of the system-managed fields after the customer's event-level hook.
//!
//! The walk ([`redact_strings`]) visits objects in [`wire::own_entries`] order and arrays by index.
//! It does not visit the strings under a protected key ([`PROTECTED_FIELDS`]) of the top level,
//! nor the event's own `timestamp` string: that string is the event's time, which the walk
//! returns as it is (a `timestamp` of another type is walked). The top level is the event and an
//! object reached from it through `""` keys only (`{"": {"sessionId": ...}}` protects that
//! `sessionId` too). Elsewhere, the same names are not protected, and a `timestamp` string is
//! redacted. `identifyActorData`, the field that holds the actor's data, is redacted: the
//! protected list names `identifyData`, a name events do not use.
//!
//! A `__proto__` key is an ordinary key: the walk passes the strings under it to the hook and
//! keeps the key. The hook's result is a typed `Event`, so it has no such key, and the restore
//! never meets one.
//!
//! A restored field the original lacks is removed from the hook's result.
use serde_json::{Map, Value};

use super::EventObject;
use crate::wire;

#[cfg(test)]
mod tests;

/// The top-level keys whose strings the walk does not redact.
const PROTECTED_FIELDS: [&str; 16] = [
    "sessionId",
    "id",
    "projectId",
    "server",
    "serverName",
    "serverVersion",
    "clientName",
    "clientVersion",
    "identifyActorGivenId",
    "identifyActorName",
    "identifyData",
    "resourceName",
    "eventType",
    "actorId",
    "tags",
    "properties",
];

/// The system-managed fields the restore takes from the original event, in the order it sets
/// them.
const RESTORED_FIELDS: [&str; 5] = ["id", "sessionId", "projectId", "eventType", "timestamp"];

/// The event field for the string-redaction hook. The restore sets it from the original, before
/// the restored fields. The event never holds the hook, so the field is absent and the key leaves
/// the hook's result; a JSON value the original holds under the key is copied.
const STRING_HOOK_FIELD: &str = "redactionFn";

/// The event field for the event-level hook, which the restore removes from the hook's result.
const EVENT_HOOK_FIELD: &str = "eventRedactionFn";

/// Where an object is in the walk.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Level {
    /// The event: the protected names apply to its keys, and its `timestamp` string is kept.
    Event,
    /// An object reached from the event through `""` keys only: it is still the top level, so
    /// the protected names apply to its keys.
    EmptyPath,
    /// Any other object.
    Nested,
}

/// The event with each string the walk visits replaced by what `redact` gives for it (module
/// docs). `redact` gets the strings in walk order; the first `Err` stops the walk and is
/// returned.
///
/// [`walked`] is the one definition of which strings are redacted; this walk and [`visit_strings`]
/// both visit objects in [`wire::own_entries`] order and arrays by index. A caller with an async
/// hook collects the strings with [`visit_strings`], awaits the hook on each string in order and
/// stops at its first error, then runs this walk with a `redact` that gives the hook's results in
/// that order.
pub(crate) fn redact_strings<E>(
    event: &EventObject,
    mut redact: impl FnMut(&str) -> Result<String, E>,
) -> Result<EventObject, E> {
    redact_object(event, Level::Event, &mut redact)
}

/// Gives `visit` each string of `event` that [`redact_strings`] passes to its `redact`, in the
/// same order, borrowed from `event`: no copy of the event is built.
pub(crate) fn visit_strings<'a>(event: &'a EventObject, mut visit: impl FnMut(&'a str)) {
    visit_object(event, Level::Event, &mut visit);
}

/// How the walk treats the value under `key` of an object at `level`: `None` keeps the value as
/// it is (a protected key of the top level, or the event's `timestamp` string); otherwise the
/// walk enters the value, and an object in it is at the level given.
fn walked(level: Level, key: &str, value: &Value) -> Option<Level> {
    let top = level != Level::Nested;
    let kept = (top && PROTECTED_FIELDS.contains(&key))
        || (level == Level::Event && key == "timestamp" && value.is_string());
    if kept {
        None
    } else if top && key.is_empty() {
        Some(Level::EmptyPath)
    } else {
        Some(Level::Nested)
    }
}

/// `object` at `level` with its strings redacted, its keys in [`wire::own_entries`] order.
fn redact_object<E, F>(
    object: &Map<String, Value>,
    level: Level,
    redact: &mut F,
) -> Result<Map<String, Value>, E>
where
    F: FnMut(&str) -> Result<String, E>,
{
    let mut out = Map::with_capacity(object.len());
    for (key, value) in wire::own_entries(object) {
        let value = match walked(level, key, value) {
            None => value.clone(),
            Some(child) => redact_value(value, child, redact)?,
        };
        out.insert(key.clone(), value);
    }
    Ok(out)
}

/// `value` with its strings redacted; an object in it is at `level`, the items of an array are
/// [`Level::Nested`].
fn redact_value<E, F>(value: &Value, level: Level, redact: &mut F) -> Result<Value, E>
where
    F: FnMut(&str) -> Result<String, E>,
{
    Ok(match value {
        Value::String(text) => Value::String(redact(text)?),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| redact_value(item, Level::Nested, redact))
                .collect::<Result<_, _>>()?,
        ),
        Value::Object(map) => Value::Object(redact_object(map, level, redact)?),
        other => other.clone(),
    })
}

/// Gives `visit` the strings of `object` at `level` that [`redact_object`] redacts, in its order.
fn visit_object<'a, F>(object: &'a Map<String, Value>, level: Level, visit: &mut F)
where
    F: FnMut(&'a str),
{
    for (key, value) in wire::own_entries(object) {
        if let Some(child) = walked(level, key, value) {
            visit_value(value, child, visit);
        }
    }
}

/// Gives `visit` the strings of `value` that [`redact_value`] redacts, in its order.
fn visit_value<'a, F>(value: &'a Value, level: Level, visit: &mut F)
where
    F: FnMut(&'a str),
{
    match value {
        Value::String(text) => visit(text),
        Value::Array(items) => {
            for item in items {
                visit_value(item, Level::Nested, visit);
            }
        }
        Value::Object(map) => visit_object(map, level, visit),
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

/// The event after the customer's event-level hook. `hook_result` is what the hook returned:
/// `None` drops the event, and gives `None`.
///
/// Otherwise the event is the hook's result without [`EVENT_HOOK_FIELD`], with
/// [`STRING_HOOK_FIELD`] and then each of the [`RESTORED_FIELDS`], in that order, set to the
/// original's value: a field the hook kept keeps its place, a field the hook removed comes back
/// at the end, and a field the original lacks is removed.
pub(crate) fn apply_event_redaction(
    original: &EventObject,
    hook_result: Option<EventObject>,
) -> Option<EventObject> {
    let copied = || std::iter::once(STRING_HOOK_FIELD).chain(RESTORED_FIELDS);
    let mut event: EventObject = hook_result?
        .into_iter()
        .filter(|(key, _)| {
            key != EVENT_HOOK_FIELD && (original.contains_key(key) || !copied().any(|f| f == key))
        })
        .collect();
    for field in copied() {
        if let Some(value) = original.get(field) {
            event.insert(field.to_owned(), value.clone());
        }
    }
    Some(event)
}

//! The redaction of the TS SDK: the walk that passes the event's strings to the customer's
//! string-redaction hook (`redactEvent`, B-089), and the restore of the system-managed fields
//! after the customer's event-level hook (`applyEventRedaction`, B-088).
//!
//! The walk ([`redact_strings`]) visits objects in [`js::own_entries`] order and arrays by index.
//! It does not visit the strings under a protected key ([`PROTECTED_FIELDS`]) of the top level,
//! nor a top-level `timestamp` string: that string stands for TS's `Date`, which the walk returns
//! as it is (a top-level `timestamp` of another type is walked). The top level is where TS's path
//! is empty: the event, and an object reached from it through `""` keys only
//! (`{"": {"sessionId": ...}}` protects that `sessionId` too). Elsewhere, the same names are not
//! protected, and a `timestamp` string is redacted. `identifyActorData`, the field that holds the
//! actor's data, is redacted: TS protects `identifyData`, a name events do not use (K-04).
//!
//! Rust departs from TS in one recorded place: a `__proto__` key is kept (RD-24). TS passes the
//! strings under it to the hook, then assigns it to the new object, which sets that object's
//! prototype instead of making a key. The hook's result is a typed `Event` (C39), so it has no
//! such key, and the restore never meets one.
//!
//! The one `undefined` TS assigns is the restore's `redactionFn` when the original has no
//! string-redaction hook. JSON leaves it out, it moves no other key, and no later step counts
//! the event's own keys, so it is not a phantom property of the kind RD-28 records. A restored
//! field the original lacks is deleted, not set to `undefined`.
use serde_json::{Map, Value};

use super::TsEvent;
use crate::js;

#[cfg(test)]
mod tests;

/// The top-level keys whose strings the walk does not redact: the `PROTECTED_FIELDS` of TS.
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

/// The system-managed fields the restore takes from the original event, in TS's order: the
/// `RESTORED_FIELDS` of TS.
const RESTORED_FIELDS: [&str; 5] = ["id", "sessionId", "projectId", "eventType", "timestamp"];

/// TS's field for the string-redaction hook. The spread `{...result, redactionFn}` takes it from
/// the original, before the restored fields. The Rust event does not hold the hook, so the field
/// is absent and the key leaves the hook's result; a JSON value the original holds under the key
/// is copied as TS copies it.
const STRING_HOOK_FIELD: &str = "redactionFn";

/// TS's field for the event-level hook, which it deletes from the hook's result.
const EVENT_HOOK_FIELD: &str = "eventRedactionFn";

/// Where an object is in the walk.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Level {
    /// The event: TS's path is empty, and its `timestamp` is a `Date`.
    Event,
    /// An object reached from the event through `""` keys only: TS's path is still empty, so
    /// the protected names apply to its keys.
    EmptyPath,
    /// Any other object.
    Nested,
}

/// The event with each string the walk visits replaced by what `redact` gives for it, as the
/// `redactEvent` of TS gives it (module docs). `redact` gets the strings in TS's call order; the
/// first `Err` stops the walk and is returned.
///
/// [`walked`] is the one definition of which strings are redacted; this walk and
/// [`visit_strings`] both visit objects in [`js::own_entries`] order and arrays by index. A caller
/// with an async hook collects the strings with [`visit_strings`], awaits the hook on each string
/// in order and stops at its first error, then runs this walk with a `redact` that gives the
/// hook's results in that order.
pub(crate) fn redact_strings<E>(
    event: &TsEvent,
    mut redact: impl FnMut(&str) -> Result<String, E>,
) -> Result<TsEvent, E> {
    redact_object(event, Level::Event, &mut redact)
}

/// Gives `visit` each string of `event` that [`redact_strings`] passes to its `redact`, in the
/// same order, borrowed from `event`: no copy of the event is built.
pub(crate) fn visit_strings<'a>(event: &'a TsEvent, mut visit: impl FnMut(&'a str)) {
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

/// `object` at `level` with its strings redacted, its keys in [`js::own_entries`] order.
fn redact_object<E, F>(
    object: &Map<String, Value>,
    level: Level,
    redact: &mut F,
) -> Result<Map<String, Value>, E>
where
    F: FnMut(&str) -> Result<String, E>,
{
    let mut out = Map::with_capacity(object.len());
    for (key, value) in js::own_entries(object) {
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
    for (key, value) in js::own_entries(object) {
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

/// The event after the customer's event-level hook, as the `applyEventRedaction` of TS leaves
/// it. `hook_result` is what the hook returned: `None` (TS's `null` or `undefined`) drops the
/// event, and gives `None`.
///
/// Otherwise the event is the hook's result without [`EVENT_HOOK_FIELD`], with
/// [`STRING_HOOK_FIELD`] and then each of the [`RESTORED_FIELDS`], in that order, set to the
/// original's value: a field the hook kept keeps its place, a field the hook removed comes back
/// at the end, and a field the original lacks is removed.
pub(crate) fn apply_event_redaction(
    original: &TsEvent,
    hook_result: Option<TsEvent>,
) -> Option<TsEvent> {
    let copied = || std::iter::once(STRING_HOOK_FIELD).chain(RESTORED_FIELDS);
    let mut event: TsEvent = hook_result?
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

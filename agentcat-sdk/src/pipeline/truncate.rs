//! Event truncation: length limits on single fields, the cut of `error.frames`, `normalize` on
//! the customer's JSON fields, and the size guard that keeps an event's JSON at most
//! [`MAX_EVENT_BYTES`] bytes where it can.
//!
//! Lengths are UTF-16 units, and a cut inside a surrogate pair drops the whole character. These
//! rules also hold:
//!
//! - A `__proto__` key in `parameters`, `response`, `identifyActorData` or `error` stays, because
//!   `normalize` keeps it.
//! - An array `response` stays an array.
//! - An object in a limited field or in `error.frames` is an error, whatever its keys (a small
//!   `length` key included).
//! - An object item of a joined array becomes `[object Object]`, whatever its keys (a `toString`
//!   key included).
//! - An `error` without `message` gets no `message` key, so it does not count toward the 100-key
//!   breadth of `normalize`: an `error` of 100 entries gets no `"..."` marker, and an existing
//!   `"..."` key keeps its value.
//!
//! Each long string is cut once:
//!
//! - The `text` blocks of `response.content` get no cut of their own: `normalize` cuts every
//!   string over 32 768 units, theirs included. A second cut would not be the same: after a cut
//!   drops a whole character, the string is still over the limit and would lose one more unit.
//! - The size guard normalizes the field-limited value again at depth 9 down to 1, never the
//!   result of the depth before. For JSON values the result is the same as a normalization of
//!   that result, and each long string is cut once, for the same reason.
use std::cmp::Reverse;
use std::fmt;

use serde_json::{Map, Value};

use super::EventObject;
use super::normalize::{MAX_BREADTH, MAX_DEPTH, MAX_STRING_LENGTH, normalize};
use crate::wire;

#[cfg(test)]
mod tests;

/// The largest event, in UTF-8 bytes of its JSON text ([`wire::stringify`]), that the size
/// guard keeps whole.
pub(crate) const MAX_EVENT_BYTES: usize = 102_400;

/// The top-level fields with a length limit, in the order the limits apply, and their limits in
/// UTF-16 units.
const LIMITED_FIELDS: [(&str, usize); 6] = [
    ("userIntent", 2_048),
    ("resourceName", 256),
    ("serverName", 256),
    ("serverVersion", 256),
    ("clientName", 256),
    ("clientVersion", 256),
];

/// The limit of `error.message`, in UTF-16 units.
const MAX_ERROR_MESSAGE_LENGTH: usize = 2_048;

/// The most entries `error.frames` keeps: the first half and the last half.
const MAX_STACK_FRAMES: usize = 50;

/// The fields `normalize` limits (when present and not `null`).
const NORMALIZED_FIELDS: [&str; 4] = ["parameters", "response", "identifyActorData", "error"];

/// The suffix of a cut string.
const TRUNCATION_SUFFIX: &str = "...";

/// The most rounds of the last resort.
const MAX_ROUNDS: usize = 10;

/// The last resort cuts only strings longer than this, in UTF-16 units.
const MIN_CUT_LENGTH: usize = 100;

/// The bytes the last resort cuts beyond the excess, for the `"..."` it adds.
const BUFFER_BYTES: usize = 200;

/// The smallest cut, in UTF-16 units, the last resort makes.
const MIN_REDUCTION: usize = 10;

/// The error text when a limited field holds a number, a boolean or an object.
const NOT_A_STRING: &str = "TypeError: str.slice is not a function";

/// The error text when `error.frames` is `true`, a number other than 0, or an object.
const NOT_FRAMES: &str =
    "TypeError: frames.slice is not a function or its return value is not iterable";

/// The error of an event that [`truncate_event`] cannot limit; the caller drops the event. The
/// `Display` is the error text ([`NOT_A_STRING`] or [`NOT_FRAMES`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TruncateError {
    message: String,
}

impl TruncateError {
    fn new(message: &str) -> Self {
        Self {
            message: message.to_owned(),
        }
    }
}

impl fmt::Display for TruncateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

/// Truncates `event`: the field limits, then `normalize` on `parameters`, `response`,
/// `identifyActorData` and `error`, then the size guard.
///
/// The size guard measures the UTF-8 bytes of the event's JSON text. Over
/// [`MAX_EVENT_BYTES`], it normalizes those fields at depth 9, 8, ... 1 and returns the first
/// event that fits; failing that, it cuts the longest strings of the depth-1 event (see
/// [`cut_longest_strings`]). The result can still be over the limit.
///
/// # Errors
///
/// A [`TruncateError`] when a limited field or `error.frames` holds a value that has no length
/// to cut (see [`limit_string`] and [`limit_frames`]).
pub(crate) fn truncate_event(mut event: EventObject) -> Result<EventObject, TruncateError> {
    limit_fields(&mut event)?;
    let limited = take_normalized_fields(&mut event);
    let mut depth = MAX_DEPTH;
    loop {
        normalize_fields(&mut event, &limited, depth);
        if byte_size(&event) <= MAX_EVENT_BYTES {
            return Ok(event);
        }
        if depth == 1 {
            return Ok(cut_longest_strings(event));
        }
        depth -= 1;
    }
}

/// Applies the length limits of [`LIMITED_FIELDS`] and of `error.message` to `event` in place,
/// and cuts `error.frames`, in that order: the first error wins (and leaves earlier limits
/// applied, which the caller drops with the event).
fn limit_fields(event: &mut EventObject) -> Result<(), TruncateError> {
    for (key, limit) in LIMITED_FIELDS {
        if let Some(value) = event.get_mut(key)
            && let Some(limited) = limit_string(value, limit)?
        {
            *value = limited;
        }
    }
    if let Some(error) = event.get_mut("error") {
        limit_error(error)?;
    }
    Ok(())
}

/// The length limit of one field: `None` when `value` stays. `null` stays; a string longer than
/// `limit` UTF-16 units is cut there and gets `"..."`; an array of more than `limit` items
/// becomes the text [`wire::coerce_to_string`] gives for its first `limit` items, then `"..."`;
/// a shorter string or array stays.
///
/// # Errors
///
/// `TypeError: str.slice is not a function` for a number, a boolean or an object, whatever the
/// object's keys.
fn limit_string(value: &Value, limit: usize) -> Result<Option<Value>, TruncateError> {
    match value {
        Value::Null => Ok(None),
        Value::String(text) => {
            Ok((wire::utf16_len(text) > limit).then(|| Value::String(cut(text, limit))))
        }
        Value::Array(items) => Ok((items.len() > limit).then(|| {
            // An object item gives "[object Object]", a `toString` key or not.
            let head = Value::Array(items.iter().take(limit).cloned().collect());
            let mut joined = wire::coerce_to_string(Some(&head));
            joined.push_str(TRUNCATION_SUFFIX);
            Value::String(joined)
        })),
        Value::Bool(_) | Value::Number(_) | Value::Object(_) => {
            Err(TruncateError::new(NOT_A_STRING))
        }
    }
}

/// `text` cut at `units` UTF-16 units (a character the cut splits is dropped), then `"..."`.
fn cut(text: &str, units: usize) -> String {
    let mut cut = wire::cut_utf16(text, units).to_owned();
    cut.push_str(TRUNCATION_SUFFIX);
    cut
}

/// The limits on an `error` object: `message` as [`limit_string`] limits it, then `frames` as
/// [`limit_frames`] cuts it. An array `error` becomes an object with its index keys (`"0"`,
/// `"1"`, ...); any other value stays. An `error` object without `message` gets no `message`
/// key.
fn limit_error(error: &mut Value) -> Result<(), TruncateError> {
    match error {
        Value::Object(fields) => {
            if let Some(message) = fields.get_mut("message")
                && let Some(limited) = limit_string(message, MAX_ERROR_MESSAGE_LENGTH)?
            {
                *message = limited;
            }
            if let Some(frames) = fields.get_mut("frames")
                && let Some(limited) = limit_frames(frames)?
            {
                *frames = limited;
            }
        }
        Value::Array(items) => {
            let spread: Map<String, Value> = std::mem::take(items)
                .into_iter()
                .enumerate()
                .map(|(index, item)| (index.to_string(), item))
                .collect();
            *error = Value::Object(spread);
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
    Ok(())
}

/// The cut of `error.frames`: `None` when `frames` stays. `null`, `false` and 0 stay, as does an
/// array or a string of at most [`MAX_STACK_FRAMES`] entries or UTF-16 units. A longer array
/// keeps its first and its last 25 entries; a longer string becomes the array of the characters
/// of its first and its last 25 units (see [`spread_ends`]).
///
/// # Errors
///
/// `TypeError: frames.slice is not a function or its return value is not iterable` for `true`,
/// another number or an object, whatever the object's keys.
fn limit_frames(frames: &Value) -> Result<Option<Value>, TruncateError> {
    let half = MAX_STACK_FRAMES / 2;
    match frames {
        Value::Array(items) => Ok((items.len() > MAX_STACK_FRAMES).then(|| {
            let tail = items.len() - half;
            let kept = items.iter().take(half).chain(items.iter().skip(tail));
            Value::Array(kept.cloned().collect())
        })),
        Value::String(text) => {
            Ok((wire::utf16_len(text) > MAX_STACK_FRAMES).then(|| spread_ends(text, half)))
        }
        Value::Null | Value::Bool(false) => Ok(None),
        Value::Number(number) if number.as_f64() == Some(0.0) => Ok(None),
        Value::Bool(true) | Value::Number(_) | Value::Object(_) => {
            Err(TruncateError::new(NOT_FRAMES))
        }
    }
}

/// The items of the ends of a string of more than `2 * half` UTF-16 units: a one-character
/// string for each character of its first and of its last `half` units. A character that a cut
/// splits gives one empty string.
fn spread_ends(text: &str, half: usize) -> Value {
    let tail = wire::utf16_len(text) - half;
    let mut items = Vec::new();
    let mut start = 0;
    for c in text.chars() {
        let end = start + c.len_utf16();
        if end <= half || start >= tail {
            items.push(Value::String(c.to_string()));
        } else if start < half || end > tail {
            items.push(Value::String(String::new()));
        }
        start = end;
    }
    Value::Array(items)
}

/// Takes each of [`NORMALIZED_FIELDS`] that is present and not `null` out of `event`, with its
/// key, and leaves `null` in its place until [`normalize_fields`] sets it. So the size guard
/// normalizes the field-limited values at every depth without a copy of the event.
fn take_normalized_fields(event: &mut EventObject) -> Vec<(&'static str, Value)> {
    NORMALIZED_FIELDS
        .into_iter()
        .filter_map(|key| {
            let value = event.get_mut(key).filter(|value| !value.is_null())?;
            Some((key, std::mem::take(value)))
        })
        .collect()
}

/// Sets each field of `limited` (from [`take_normalized_fields`]) in `event` to its value
/// normalized at `depth` with the default breadth and string limits, in the field's place. An
/// array `response` stays an array.
fn normalize_fields(event: &mut EventObject, limited: &[(&str, Value)], depth: usize) {
    for (key, value) in limited {
        if let Some(field) = event.get_mut(*key) {
            *field = normalize(value, depth, MAX_BREADTH, MAX_STRING_LENGTH);
        }
    }
}

/// A `fmt::Write` sink that counts the UTF-8 bytes written and keeps none of them.
#[derive(Debug, Default)]
struct ByteCounter(usize);

impl fmt::Write for ByteCounter {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        self.0 += text.len();
        Ok(())
    }
}

/// The UTF-8 size of the JSON text of `event`: the bytes [`wire::write_stringify`] writes,
/// counted without building the text.
fn byte_size(event: &EventObject) -> usize {
    // `{`, `}`, a `,` between two entries, and `"key":value` for each entry; the order of the
    // entries does not change the size.
    let mut counter = ByteCounter::default();
    for (key, value) in event {
        // A `ByteCounter` never fails.
        let _ = wire::write_quoted(&mut counter, key);
        let _ = wire::write_stringify(value, &mut counter);
    }
    // One `:` per entry, and one `,` fewer than entries.
    2 + counter.0 + event.len() + event.len().saturating_sub(1)
}

/// One step of a path from the event to a string in it.
#[derive(Debug, Clone)]
enum Step {
    Key(String),
    Index(usize),
}

/// A string the last resort can cut: where it is (under the event's `key`, at `path` below it),
/// and its length in UTF-16 units.
#[derive(Debug)]
struct Candidate {
    key: String,
    path: Vec<Step>,
    length: usize,
}

/// The last resort of the size guard. Each of at most [`MAX_ROUNDS`] rounds measures `event` and
/// returns it when it fits. Otherwise it collects every string longer than [`MIN_CUT_LENGTH`]
/// units in walk order ([`collect_long_strings`]), sorts them longest first (a stable sort), and
/// spends a budget of the excess bytes plus [`BUFFER_BYTES`] on them: each takes the smaller of
/// the budget and half its length, unless that is under [`MIN_REDUCTION`] (then it is skipped),
/// and is cut by that many units (a split character is dropped) and gets `"..."`. A round with
/// no string or no cut ends the loop. The result can still be over the limit.
fn cut_longest_strings(mut event: EventObject) -> EventObject {
    for _ in 0..MAX_ROUNDS {
        let size = byte_size(&event);
        if size <= MAX_EVENT_BYTES {
            return event;
        }
        let mut candidates = Vec::new();
        let mut path = Vec::new();
        for (key, value) in wire::own_entries(&event) {
            collect_long_strings(key, value, &mut path, &mut candidates);
        }
        candidates.sort_by_key(|candidate| Reverse(candidate.length));
        let mut remaining = size - MAX_EVENT_BYTES + BUFFER_BYTES;
        let mut cut_any = false;
        for Candidate { key, path, length } in candidates {
            if remaining == 0 {
                break;
            }
            let reduction = remaining.min(length / 2);
            if reduction < MIN_REDUCTION {
                continue;
            }
            if let Some(Value::String(text)) = value_at(&mut event, &key, &path) {
                *text = cut(text, length - reduction);
                remaining -= reduction;
                cut_any = true;
            }
        }
        if !cut_any {
            break;
        }
    }
    event
}

/// Pushes each string longer than [`MIN_CUT_LENGTH`] UTF-16 units in `value` (at `path` under
/// the event's `key`) to `out`, in walk order: array items by index, object entries in
/// [`wire::own_entries`] order.
fn collect_long_strings(key: &str, value: &Value, path: &mut Vec<Step>, out: &mut Vec<Candidate>) {
    match value {
        Value::String(text) => {
            let length = wire::utf16_len(text);
            if length > MIN_CUT_LENGTH {
                out.push(Candidate {
                    key: key.to_owned(),
                    path: path.clone(),
                    length,
                });
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                path.push(Step::Index(index));
                collect_long_strings(key, item, path, out);
                path.pop();
            }
        }
        Value::Object(map) => {
            for (name, item) in wire::own_entries(map) {
                path.push(Step::Key(name.clone()));
                collect_long_strings(key, item, path, out);
                path.pop();
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

/// The value at `path` under the event's `key`, if there is one.
fn value_at<'a>(event: &'a mut EventObject, key: &str, path: &[Step]) -> Option<&'a mut Value> {
    let mut at = event.get_mut(key)?;
    for step in path {
        at = match step {
            Step::Key(name) => at.get_mut(name.as_str())?,
            Step::Index(index) => at.get_mut(*index)?,
        };
    }
    Some(at)
}

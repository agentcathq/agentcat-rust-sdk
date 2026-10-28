//! The event truncation of the TS SDK (`truncateEvent`, B-091): length limits on single fields,
//! the cut of `error.frames`, `normalize` on the customer's JSON fields, and the size guard that
//! keeps an event's JSON at most [`MAX_EVENT_BYTES`] bytes where it can.
//!
//! Lengths are UTF-16 units, and a cut inside a surrogate pair drops the whole character (RD-22).
//! Rust departs from TS in these other recorded places:
//!
//! - A `__proto__` key in `parameters`, `response`, `identifyActorData` or `error` stays, because
//!   `normalize` keeps it (RD-24).
//! - An array `response` stays an array (RD-25).
//! - An object with a small `length` key is an error like any other object (RD-26).
//! - An object with an own `toString` key in a joined array becomes `[object Object]` (RD-27).
//! - An `error` without `message` gets no `message` key, so it does not count toward the 100-key
//!   breadth of `normalize`: an `error` of 100 entries gets no `"..."` marker, and an existing
//!   `"..."` key keeps its value (RD-28).
//!
//! Two steps take a different route to TS's result:
//!
//! - TS cuts each `text` block of `response.content` over 32 768 units before `normalize`, which
//!   then cuts every string over 32 768 units at the same place, so that step changes nothing
//!   `normalize` does not. Rust leaves the cut to `normalize`. A second cut would differ: after
//!   an RD-22 drop the string is still over the limit and would lose one more unit.
//! - The size guard normalizes the fields again at depth 9 down to 1. TS normalizes the depth-10
//!   result; Rust normalizes the field-limited value. For JSON values the two are equal, and the
//!   Rust route cuts each long string once, for the same reason.
use std::cmp::Reverse;
use std::fmt;

use serde_json::{Map, Value};

use super::TsEvent;
use super::normalize::{MAX_BREADTH, MAX_DEPTH, MAX_STRING_LENGTH, normalize};
use crate::js;

#[cfg(test)]
mod tests;

/// The largest event, in UTF-8 bytes of its `JSON.stringify` text, that the size guard keeps
/// whole.
pub(crate) const MAX_EVENT_BYTES: usize = 102_400;

/// The top-level fields with a length limit, in the order TS limits them, and their limits in
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

/// What TS throws when a limited field holds a number, a boolean or an object.
const NOT_A_STRING: &str = "TypeError: str.slice is not a function";

/// What TS throws when `error.frames` is `true`, a number other than 0, or an object.
const NOT_FRAMES: &str =
    "TypeError: frames.slice is not a function or its return value is not iterable";

/// An error TS's `truncateEvent` throws (TS then drops the event), as `String(error)` writes
/// it: the text is the `Display`.
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

/// TS's `truncateEvent`: the field limits, then `normalize` on `parameters`, `response`,
/// `identifyActorData` and `error`, then the size guard.
///
/// The size guard measures the UTF-8 bytes of the event's `JSON.stringify` text. Over
/// [`MAX_EVENT_BYTES`], it normalizes those fields at depth 9, 8, ... 1 and returns the first
/// event that fits; failing that, it cuts the longest strings of the depth-1 event (see
/// [`cut_longest_strings`]). The result can still be over the limit.
///
/// # Errors
///
/// TS's error, with its text, when a limited field or `error.frames` holds a value TS cannot
/// slice (see [`limit_string`] and [`limit_frames`]).
pub(crate) fn truncate_event(mut event: TsEvent) -> Result<TsEvent, TruncateError> {
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
/// and cuts `error.frames`, in TS's order: the first error wins (and leaves earlier limits
/// applied, which the caller drops with the event).
fn limit_fields(event: &mut TsEvent) -> Result<(), TruncateError> {
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

/// TS's `truncateString(value, limit)`: `None` when `value` stays. `null` stays; a string longer
/// than `limit` UTF-16 units is cut there (RD-22) and gets `"..."`; an array of more than `limit`
/// items becomes the string `String()` gives for its first `limit` items, then `"..."`; a shorter
/// string or array stays.
///
/// # Errors
///
/// `TypeError: str.slice is not a function` for a number, a boolean or an object. TS keeps an
/// object whose `length` property is at most `limit` (RD-26).
fn limit_string(value: &Value, limit: usize) -> Result<Option<Value>, TruncateError> {
    match value {
        Value::Null => Ok(None),
        Value::String(text) => {
            Ok((js::utf16_len(text) > limit).then(|| Value::String(cut(text, limit))))
        }
        Value::Array(items) => Ok((items.len() > limit).then(|| {
            // RD-27: an object with an own `toString` key gives "[object Object]".
            let head = Value::Array(items.iter().take(limit).cloned().collect());
            let mut joined = js::to_js_string(Some(&head));
            joined.push_str(TRUNCATION_SUFFIX);
            Value::String(joined)
        })),
        Value::Bool(_) | Value::Number(_) | Value::Object(_) => {
            Err(TruncateError::new(NOT_A_STRING))
        }
    }
}

/// `text` cut at `units` UTF-16 units (RD-22), then `"..."`.
fn cut(text: &str, units: usize) -> String {
    let mut cut = js::cut_utf16(text, units).to_owned();
    cut.push_str(TRUNCATION_SUFFIX);
    cut
}

/// TS's limits on an `error` object: `message` as [`limit_string`] limits it, then `frames` as
/// [`limit_frames`] cuts it. TS spreads an array `error` into an object with index keys
/// (`{...error}`); any other value stays. TS also gives an `error` object without `message` the
/// key `message` with the value `undefined`; Rust adds no key (RD-28).
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

/// TS's `truncateStackFrames(frames)`: `None` when `frames` stays. A falsy value (`null`,
/// `false`, 0, `""`) stays, as does an array or a string of at most [`MAX_STACK_FRAMES`] entries
/// or UTF-16 units. A longer array keeps its first and its last 25 entries; a longer string
/// becomes the array of the characters of its first and its last 25 units, as JavaScript's
/// spread gives them (see [`spread_ends`]).
///
/// # Errors
///
/// `TypeError: frames.slice is not a function or its return value is not iterable` for `true`,
/// another number or an object. TS keeps an object whose `length` property is at most 50
/// (RD-26).
fn limit_frames(frames: &Value) -> Result<Option<Value>, TruncateError> {
    let half = MAX_STACK_FRAMES / 2;
    match frames {
        Value::Array(items) => Ok((items.len() > MAX_STACK_FRAMES).then(|| {
            let tail = items.len() - half;
            let kept = items.iter().take(half).chain(items.iter().skip(tail));
            Value::Array(kept.cloned().collect())
        })),
        Value::String(text) => {
            Ok((js::utf16_len(text) > MAX_STACK_FRAMES).then(|| spread_ends(text, half)))
        }
        Value::Null | Value::Bool(false) => Ok(None),
        Value::Number(number) if number.as_f64() == Some(0.0) => Ok(None),
        Value::Bool(true) | Value::Number(_) | Value::Object(_) => {
            Err(TruncateError::new(NOT_FRAMES))
        }
    }
}

/// `[...text.slice(0, half), ...text.slice(-half)]` for a string of more than `2 * half` UTF-16
/// units: a one-character string for each character of the first and of the last `half` units.
/// Where a cut splits a character, JavaScript's item is half a surrogate pair; here it is the
/// empty string (RD-22).
fn spread_ends(text: &str, half: usize) -> Value {
    let tail = js::utf16_len(text) - half;
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
fn take_normalized_fields(event: &mut TsEvent) -> Vec<(&'static str, Value)> {
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
/// array `response` stays an array (RD-25).
fn normalize_fields(event: &mut TsEvent, limited: &[(&str, Value)], depth: usize) {
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

/// The UTF-8 size of `JSON.stringify(event)`: what TS's `jsonByteSize` measures. It counts the
/// bytes [`js::write_stringify`] writes; no text is built.
fn byte_size(event: &TsEvent) -> usize {
    // `{`, `}`, a `,` between two entries, and `"key":value` for each entry; the order of the
    // entries does not change the size.
    let mut counter = ByteCounter::default();
    for (key, value) in event {
        // A `ByteCounter` never fails.
        let _ = js::write_quoted(&mut counter, key);
        let _ = js::write_stringify(value, &mut counter);
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

/// TS's last resort (`truncateLargestFields`). Each of at most [`MAX_ROUNDS`] rounds measures
/// `event` and returns it when it fits. Otherwise it collects every string longer than
/// [`MIN_CUT_LENGTH`] units in JavaScript order, sorts them longest first (a stable sort), and
/// spends a budget of the excess bytes plus [`BUFFER_BYTES`] on them: each takes the smaller of
/// the budget and half its length, unless that is under [`MIN_REDUCTION`] (then it is skipped),
/// and is cut by that many units (RD-22) and gets `"..."`. A round with no string or no cut
/// ends the loop. The result can still be over the limit.
fn cut_longest_strings(mut event: TsEvent) -> TsEvent {
    for _ in 0..MAX_ROUNDS {
        let size = byte_size(&event);
        if size <= MAX_EVENT_BYTES {
            return event;
        }
        let mut candidates = Vec::new();
        let mut path = Vec::new();
        for (key, value) in js::own_entries(&event) {
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
/// the event's `key`) to `out`, in JavaScript order: array items by index, object entries in
/// [`js::own_entries`] order.
fn collect_long_strings(key: &str, value: &Value, path: &mut Vec<Step>, out: &mut Vec<Candidate>) {
    match value {
        Value::String(text) => {
            let length = js::utf16_len(text);
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
            for (name, item) in js::own_entries(map) {
                path.push(Step::Key(name.clone()));
                collect_long_strings(key, item, path, out);
                path.pop();
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

/// The value at `path` under the event's `key`, if there is one.
fn value_at<'a>(event: &'a mut TsEvent, key: &str, path: &[Step]) -> Option<&'a mut Value> {
    let mut at = event.get_mut(key)?;
    for step in path {
        at = match step {
            Step::Key(name) => at.get_mut(name.as_str())?,
            Step::Index(index) => at.get_mut(*index)?,
        };
    }
    Some(at)
}

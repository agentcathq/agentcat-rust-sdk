//! Truncation: field limits, recursive normalization, and a size budget.
//!
//! Lengths count UTF-16 code units, like the TypeScript SDK, so the same
//! input gives the same output in both SDKs (cuts never split a character).
//! The varchar columns of the AgentCat API are the exception: names are
//! capped at 255 characters and versions at 50, which the API stores without
//! failing the insert.

use serde_json::{Map, Value};

use crate::event::Event;
use crate::util::{cap_chars, truncate_js, utf16_len, utf16_prefix};

pub(crate) const MAX_DEPTH: usize = 10;
pub(crate) const MAX_BREADTH: usize = 100;
pub(crate) const MAX_STRING_LENGTH: usize = 32_768;
pub(crate) const MAX_EVENT_BYTES: usize = 102_400;

const MAX_USER_INTENT_LENGTH: usize = 2_048;
const MAX_ERROR_MESSAGE_LENGTH: usize = 2_048;
const MAX_STACK_FRAMES: usize = 50;
const MAX_CONTENT_TEXT_LENGTH: usize = 32_768;
/// API column limits (varchar sizes, in characters).
const MAX_NAME_COLUMN: usize = 255;
const MAX_VERSION_COLUMN: usize = 50;

const MAX_PROPERTIES_MARKER: &str = "[MaxProperties ~]";

/// Recursively limits depth, breadth and string length.
pub(crate) fn normalize(input: &Value, depth: usize, max_breadth: usize, max_string: usize) -> Value {
    match input {
        Value::String(s) => match truncate_js(s, max_string) {
            Some(cut) => Value::String(cut),
            None => input.clone(),
        },
        Value::Array(items) => {
            if depth == 0 {
                return Value::String("[Array]".into());
            }
            let mut out = Vec::with_capacity(items.len().min(max_breadth + 1));
            for (i, item) in items.iter().enumerate() {
                if i >= max_breadth {
                    out.push(Value::String(MAX_PROPERTIES_MARKER.into()));
                    break;
                }
                out.push(normalize(item, depth - 1, max_breadth, max_string));
            }
            Value::Array(out)
        }
        Value::Object(map) => {
            if depth == 0 {
                return Value::String("[Object]".into());
            }
            let mut out = Map::new();
            for (count, (key, value)) in map.iter().enumerate() {
                if count >= max_breadth {
                    out.insert("...".into(), Value::String(MAX_PROPERTIES_MARKER.into()));
                    break;
                }
                out.insert(key.clone(), normalize(value, depth - 1, max_breadth, max_string));
            }
            Value::Object(out)
        }
        _ => input.clone(),
    }
}

fn normalize_default(v: &Value) -> Value {
    normalize(v, MAX_DEPTH, MAX_BREADTH, MAX_STRING_LENGTH)
}

fn cut_in_place(field: &mut Option<String>, max_units: usize) {
    if let Some(s) = field.as_deref()
        && let Some(cut) = truncate_js(s, max_units)
    {
        *field = Some(cut);
    }
}

fn cap_in_place(field: &mut Option<String>, max_chars: usize) {
    if let Some(s) = field.as_deref()
        && let Some(cut) = cap_chars(s, max_chars)
    {
        *field = Some(cut);
    }
}

fn truncate_error(error: &mut Value) {
    let Some(obj) = error.as_object_mut() else {
        return;
    };
    if let Some(Value::String(message)) = obj.get_mut("message")
        && let Some(cut) = truncate_js(message, MAX_ERROR_MESSAGE_LENGTH)
    {
        *message = cut;
    }
    if let Some(Value::Array(frames)) = obj.get_mut("frames")
        && frames.len() > MAX_STACK_FRAMES
    {
        let half = MAX_STACK_FRAMES / 2;
        let tail = frames.split_off(frames.len() - half);
        frames.truncate(half);
        frames.extend(tail);
    }
}

fn truncate_response_content(response: &mut Value) {
    let Some(Value::Array(content)) = response.get_mut("content") else {
        return;
    };
    for block in content.iter_mut() {
        let Some(obj) = block.as_object_mut() else {
            continue;
        };
        if obj.get("type").and_then(Value::as_str) != Some("text") {
            continue;
        }
        if let Some(Value::String(text)) = obj.get_mut("text")
            && let Some(cut) = truncate_js(text, MAX_CONTENT_TEXT_LENGTH)
        {
            *text = cut;
        }
    }
}

fn json_byte_size<T: serde::Serialize>(value: &T) -> usize {
    serde_json::to_vec(value).map(|v| v.len()).unwrap_or(usize::MAX)
}

/// Normalizes the four free-form fields at a depth.
fn with_depth(event: &Event, depth: usize) -> Event {
    let mut reduced = event.clone();
    let n = |v: &Option<Value>| {
        v.as_ref()
            .map(|v| normalize(v, depth, MAX_BREADTH, MAX_STRING_LENGTH))
    };
    reduced.parameters = n(&event.parameters);
    reduced.response = n(&event.response);
    reduced.identify_actor_data = n(&event.identify_actor_data);
    reduced.error = n(&event.error);
    reduced
}

struct StringPath {
    path: Vec<PathStep>,
    units: usize,
}

#[derive(Clone)]
enum PathStep {
    Key(String),
    Index(usize),
}

fn collect_string_paths(value: &Value, current: &mut Vec<PathStep>, out: &mut Vec<StringPath>) {
    match value {
        Value::String(s) => {
            let units = utf16_len(s);
            if units > 100 {
                out.push(StringPath {
                    path: current.clone(),
                    units,
                });
            }
        }
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                current.push(PathStep::Index(i));
                collect_string_paths(item, current, out);
                current.pop();
            }
        }
        Value::Object(map) => {
            for (key, item) in map {
                current.push(PathStep::Key(key.clone()));
                collect_string_paths(item, current, out);
                current.pop();
            }
        }
        _ => {}
    }
}

fn get_mut<'a>(root: &'a mut Value, path: &[PathStep]) -> Option<&'a mut Value> {
    let mut cur = root;
    for step in path {
        cur = match step {
            PathStep::Key(k) => cur.get_mut(k.as_str())?,
            PathStep::Index(i) => cur.get_mut(*i)?,
        };
    }
    Some(cur)
}

/// Last resort: cuts the largest strings anywhere in the event until it fits
/// or no further progress is possible (at most 10 passes).
fn truncate_largest_fields(mut value: Value, max_bytes: usize) -> Value {
    for _ in 0..10 {
        let size = json_byte_size(&value);
        if size <= max_bytes {
            return value;
        }
        let excess = size - max_bytes;
        let mut paths = Vec::new();
        collect_string_paths(&value, &mut Vec::new(), &mut paths);
        // Stable sort: equal lengths keep traversal order, as in JS.
        paths.sort_by_key(|p| std::cmp::Reverse(p.units));
        if paths.is_empty() {
            break;
        }
        let mut remaining = excess as i64 + 200;
        let mut truncated = false;
        for StringPath { path, units } in paths {
            if remaining <= 0 {
                break;
            }
            let reduction = remaining.min((units / 2) as i64);
            if reduction < 10 {
                continue;
            }
            let new_len = units - reduction as usize;
            if let Some(Value::String(s)) = get_mut(&mut value, &path) {
                *s = format!("{}...", utf16_prefix(s, new_len));
            }
            remaining -= reduction;
            truncated = true;
        }
        if !truncated {
            break;
        }
    }
    value
}

fn truncate_to_size(event: Event) -> Event {
    if json_byte_size(&event) <= MAX_EVENT_BYTES {
        return event;
    }
    for depth in (1..MAX_DEPTH).rev() {
        let reduced = with_depth(&event, depth);
        if json_byte_size(&reduced) <= MAX_EVENT_BYTES {
            return reduced;
        }
    }
    let minimal = with_depth(&event, 1);
    let Ok(value) = serde_json::to_value(&minimal) else {
        return minimal;
    };
    let cut = truncate_largest_fields(value, MAX_EVENT_BYTES);
    serde_json::from_value(cut).unwrap_or(minimal)
}

/// Applies all truncation layers to an event.
pub(crate) fn truncate_event(mut event: Event) -> Event {
    // Layer 1: field limits.
    cut_in_place(&mut event.user_intent, MAX_USER_INTENT_LENGTH);
    cap_in_place(&mut event.resource_name, MAX_NAME_COLUMN);
    cap_in_place(&mut event.server_name, MAX_NAME_COLUMN);
    cap_in_place(&mut event.client_name, MAX_NAME_COLUMN);
    cap_in_place(&mut event.server_version, MAX_VERSION_COLUMN);
    cap_in_place(&mut event.client_version, MAX_VERSION_COLUMN);
    cap_in_place(&mut event.identify_actor_given_id, MAX_NAME_COLUMN);
    cap_in_place(&mut event.identify_actor_name, MAX_NAME_COLUMN);
    cap_in_place(&mut event.actor_id, MAX_NAME_COLUMN);
    if let Some(error) = event.error.as_mut() {
        truncate_error(error);
    }
    if let Some(response) = event.response.as_mut() {
        truncate_response_content(response);
    }

    // Layer 2: normalization of the customer-controlled fields.
    for field in [
        &mut event.parameters,
        &mut event.response,
        &mut event.identify_actor_data,
        &mut event.error,
    ] {
        if let Some(v) = field.as_ref() {
            *field = Some(normalize_default(v));
        }
    }

    // Layer 3: the size budget.
    truncate_to_size(event)
}

//! The sanitization: content blocks of the response that
//! the dashboard cannot show, and large base64 strings, are replaced by a short note.
//!
//! The step makes a new event. Of `response` (when not `null`), an object keeps its keys, with
//! each block of `content` mapped by [`sanitize_block`] and a `structuredContent` that is an
//! object or an array walked like `parameters`; any other `response` stays as it is. Of
//! `parameters` (when not `null`), every string of at least [`SIZE_GATE`] UTF-16 units that matches
//! `^[A-Za-z0-9+/\n\r]+=*$` becomes [`BINARY_DATA_NOTE`]; arrays and objects are walked.
//!
//! Further rules:
//!
//! - A `__proto__` key is an ordinary key: the walk keeps it, and so do the copies of the event,
//!   of the `response` and of a block returned as it is.
//! - An array `response` stays an array.
//! - A block `type` that is an object with a `toString` key (or an array holding one) is written
//!   as `[object Object]`, as any object is, in the message of an unsupported type.
use serde_json::{Map, Value};

use super::EventObject;
use crate::wire;

#[cfg(test)]
mod tests;

/// The shortest string, in UTF-16 units, that is tested for base64.
const SIZE_GATE: usize = 10_240;

/// The note that replaces a base64 string.
const BINARY_DATA_NOTE: &str = "[binary data redacted - not supported by AgentCat]";

/// The note that replaces an embedded resource with a `blob`.
const BINARY_RESOURCE_NOTE: &str = "[binary resource content redacted - not supported by AgentCat]";

/// Sanitizes `event` (module docs).
pub(crate) fn sanitize_event(event: &EventObject) -> EventObject {
    wire::own_entries(event)
        .map(|(key, value)| {
            let value = match key.as_str() {
                "response" if !value.is_null() => sanitize_response(value),
                "parameters" if !value.is_null() => sanitize_tool_values(value),
                _ => value.clone(),
            };
            (key.clone(), value)
        })
        .collect()
}

/// An object `response` with its `content` blocks and `structuredContent` sanitized; any other
/// `response` as it is (an array too).
fn sanitize_response(response: &Value) -> Value {
    let Value::Object(map) = response else {
        return response.clone();
    };
    Value::Object(
        wire::own_entries(map)
            .map(|(key, value)| {
                let value = match (key.as_str(), value) {
                    ("content", Value::Array(blocks)) => {
                        Value::Array(blocks.iter().map(sanitize_block).collect())
                    }
                    ("structuredContent", Value::Object(_) | Value::Array(_)) => {
                        sanitize_tool_values(value)
                    }
                    _ => value.clone(),
                };
                (key.clone(), value)
            })
            .collect(),
    )
}

/// A `text` block with `text`.
fn text_block(text: &str) -> Value {
    let mut block = Map::new();
    block.insert("type".to_owned(), Value::String("text".to_owned()));
    block.insert("text".to_owned(), Value::String(text.to_owned()));
    Value::Object(block)
}

/// One block of `response.content`. A block that is neither an object nor an array stays; an
/// array has no `type`, so it is an unsupported type, written as `undefined`.
fn sanitize_block(block: &Value) -> Value {
    let kind = match block {
        Value::Object(map) => map.get("type"),
        Value::Array(_) => None,
        other => return other.clone(),
    };
    match kind.and_then(Value::as_str) {
        Some("text" | "resource_link") => block.clone(),
        Some("image") => text_block("[image content redacted - not supported by AgentCat]"),
        Some("audio") => text_block("[audio content redacted - not supported by AgentCat]"),
        Some("resource") => sanitize_resource(block),
        _ => text_block(&format!(
            "[unsupported content type \"{}\" redacted - not supported by AgentCat]",
            wire::coerce_to_string(kind)
        )),
    }
}

/// An embedded resource: replaced when its `resource` is an object with a `blob` key of any
/// value, even `null`; kept otherwise (a `resource` of another type has no `blob`).
fn sanitize_resource(block: &Value) -> Value {
    match block.get("resource") {
        Some(Value::Object(resource)) if resource.contains_key("blob") => {
            text_block(BINARY_RESOURCE_NOTE)
        }
        _ => block.clone(),
    }
}

/// `value` (the `parameters`, or the `structuredContent` of a response) with every large base64
/// string replaced; objects in [`wire::own_entries`] order, a `__proto__` key kept.
fn sanitize_tool_values(value: &Value) -> Value {
    match value {
        Value::String(text) if is_binary(text) => Value::String(BINARY_DATA_NOTE.to_owned()),
        Value::Array(items) => Value::Array(items.iter().map(sanitize_tool_values).collect()),
        Value::Object(map) => Value::Object(
            wire::own_entries(map)
                .map(|(key, item)| (key.clone(), sanitize_tool_values(item)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// `true` for a string of at least [`SIZE_GATE`] units that matches `^[A-Za-z0-9+/\n\r]+=*$`.
/// `$` is the end of the string only: a final `\n` after the padding does not match.
fn is_binary(text: &str) -> bool {
    if wire::utf16_len(text) < SIZE_GATE {
        return false;
    }
    let body = text.trim_end_matches('=');
    !body.is_empty()
        && body
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'\n' | b'\r'))
}

//! Sanitization: replaces non-text content blocks in responses and large
//! base64 strings in parameters, which AgentCat does not store.

use serde_json::{Map, Value, json};

use crate::event::Event;

/// Strings shorter than this (in UTF-16 units) are never checked for base64.
const SIZE_GATE: usize = 10_240;

pub(crate) fn sanitize_event(event: &mut Event) {
    if let Some(response) = event.response.as_mut() {
        sanitize_response(response);
    }
    if let Some(parameters) = event.parameters.as_mut() {
        sanitize_parameters(parameters);
    }
}

fn sanitize_response(response: &mut Value) {
    let Some(obj) = response.as_object_mut() else {
        return;
    };
    if let Some(Value::Array(content)) = obj.get_mut("content") {
        for block in content.iter_mut() {
            sanitize_content_block(block);
        }
    }
    if let Some(sc) = obj.get_mut("structuredContent")
        && (sc.is_object() || sc.is_array())
    {
        sanitize_parameters(sc);
    }
}

fn redacted_text(text: String) -> Value {
    json!({"type": "text", "text": text})
}

fn sanitize_content_block(block: &mut Value) {
    if block.is_array() {
        // A JS array is an object with no `type`.
        *block = redacted_text(
            "[unsupported content type \"undefined\" redacted - not supported by AgentCat]".into(),
        );
        return;
    }
    let Some(obj) = block.as_object() else {
        return;
    };
    let replacement = match obj.get("type") {
        Some(Value::String(t)) if t == "text" || t == "resource_link" => return,
        Some(Value::String(t)) if t == "image" => {
            redacted_text("[image content redacted - not supported by AgentCat]".into())
        }
        Some(Value::String(t)) if t == "audio" => {
            redacted_text("[audio content redacted - not supported by AgentCat]".into())
        }
        Some(Value::String(t)) if t == "resource" => {
            // A blob resource (even `blob: null`) is binary; text passes.
            if obj
                .get("resource")
                .and_then(Value::as_object)
                .is_some_and(|r| r.contains_key("blob"))
            {
                redacted_text("[binary resource content redacted - not supported by AgentCat]".into())
            } else {
                return;
            }
        }
        other => redacted_text(format!(
            "[unsupported content type \"{}\" redacted - not supported by AgentCat]",
            js_string(other)
        )),
    };
    *block = replacement;
}

/// `String(value)` for the type discriminator in the redaction message.
fn js_string(value: Option<&Value>) -> String {
    match value {
        None => "undefined".into(),
        Some(Value::Null) => "null".into(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| match v {
                Value::Null => String::new(),
                other => js_string(Some(other)),
            })
            .collect::<Vec<_>>()
            .join(","),
        Some(Value::Object(_)) => "[object Object]".into(),
    }
}

/// Base64 check: `^[A-Za-z0-9+/\n\r]+=*$`.
fn looks_like_base64(s: &str) -> bool {
    let body = s.trim_end_matches('=');
    !body.is_empty()
        && body
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'\n' | b'\r'))
}

fn sanitize_parameters(value: &mut Value) {
    match value {
        Value::String(s) => {
            if crate::util::utf16_len(s) >= SIZE_GATE && looks_like_base64(s) {
                *s = "[binary data redacted - not supported by AgentCat]".into();
            }
        }
        Value::Array(items) => items.iter_mut().for_each(sanitize_parameters),
        Value::Object(map) => sanitize_map(map),
        _ => {}
    }
}

fn sanitize_map(map: &mut Map<String, Value>) {
    for value in map.values_mut() {
        sanitize_parameters(value);
    }
}

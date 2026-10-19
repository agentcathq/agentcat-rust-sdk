//! Token estimates for tool-call events.
//!
//! One divisor, pinned across the AgentCat SDKs: `ceil(utf8_bytes / 3.5)`.
//! The input side counts the raw arguments the model emitted (injected
//! parameters included); the output side counts the content-block text the
//! harness feeds back. Nothing else counts.

use serde_json::Value;

/// Bytes per token.
pub(crate) const TOKEN_ESTIMATE_BYTES_PER_TOKEN: f64 = 3.5;
/// The API's token columns are `int32`.
const MAX_TOKEN_COUNT: u64 = 2_147_483_647;

/// `ceil(bytes / 3.5)`, 0 for nothing, clamped to `i32::MAX`.
pub(crate) fn estimate_tokens(bytes: f64) -> u32 {
    if !(bytes > 0.0) {
        return 0;
    }
    let tokens = (bytes / TOKEN_ESTIMATE_BYTES_PER_TOKEN).ceil();
    if tokens >= MAX_TOKEN_COUNT as f64 {
        MAX_TOKEN_COUNT as u32
    } else {
        tokens as u32
    }
}

/// UTF-8 length of the compact JSON, the same bytes as JavaScript's
/// `JSON.stringify`.
fn compact_json_bytes(value: &Value) -> Option<usize> {
    serde_json::to_vec(value).ok().map(|v| v.len())
}

/// Tokens the model spent emitting the call: the raw arguments object.
/// `None` when the call has no arguments.
pub(crate) fn estimate_input_tokens(args: Option<&Value>) -> Option<u32> {
    match args {
        None | Some(Value::Null) => None,
        Some(v) => compact_json_bytes(v).map(|b| estimate_tokens(b as f64)),
    }
}

/// Tokens the model reads back: the text of the content blocks. Falls back
/// to the whole response when it carries no content list; `None` when there
/// is no response.
pub(crate) fn estimate_output_tokens(response: Option<&Value>) -> Option<u32> {
    let response = match response {
        None | Some(Value::Null) => return None,
        Some(r) => r,
    };
    let content = response.get("content");
    let Some(Value::Array(content)) = content else {
        return compact_json_bytes(response).map(|b| estimate_tokens(b as f64));
    };
    if content.is_empty() {
        return match response.get("structuredContent") {
            Some(s) if !s.is_null() => compact_json_bytes(s).map(|b| estimate_tokens(b as f64)),
            _ => Some(0),
        };
    }
    let bytes: usize = content.iter().map(content_block_bytes).sum();
    Some(estimate_tokens(bytes as f64))
}

fn content_block_bytes(block: &Value) -> usize {
    let Some(obj) = block.as_object() else {
        return 0;
    };
    match obj.get("type").and_then(Value::as_str) {
        Some("text") => obj.get("text").and_then(Value::as_str).map_or(0, str::len),
        Some("resource") => obj
            .get("resource")
            .and_then(|r| r.get("text"))
            .and_then(Value::as_str)
            .map_or(0, str::len),
        _ => 0,
    }
}

//! Token estimates for tool-call events: `ceil(utf8 bytes / 3.5)` of the JSON text
//! ([`wire::stringify`]) of the arguments, or of the text blocks of the result.

use serde_json::Value;

use crate::wire;

#[cfg(test)]
mod tests;

/// `ceil(bytes / 3.5)` for a byte count, at most `i32::MAX` (the server's int32 column).
///
/// `ceil(bytes / 3.5)` is `ceil(2 * bytes / 7)` in integers; `u128` keeps the product from
/// overflowing for any `u64`.
pub(crate) fn estimate_tokens(bytes: u64) -> i32 {
    let tokens = (2 * u128::from(bytes)).div_ceil(7);
    i32::try_from(tokens).unwrap_or(i32::MAX)
}

/// The estimate for the byte length of the JSON text of `value` ([`wire::stringify`]).
fn json_tokens(value: &Value) -> i32 {
    estimate_tokens(byte_count(wire::stringify(value).len()))
}

/// A `usize` byte count as `u64` (saturating, though `usize` is never wider than 64 bits here).
fn byte_count(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX)
}

/// The estimate for the raw arguments of a call: the bytes of their JSON text. `None` where there
/// are no arguments (absent or `null`), so the event has no `inputTokens` field.
pub(crate) fn estimate_input_tokens(arguments: Option<&Value>) -> Option<i32> {
    match arguments {
        None | Some(Value::Null) => None,
        Some(value) => Some(json_tokens(value)),
    }
}

/// The estimate for the result of a call. `None` where there is no response (absent or `null`).
/// A response without a `content` array counts its whole JSON. An empty `content` counts the JSON
/// of `structuredContent`, or `0` when that is absent or `null`. Otherwise the text of `text`
/// blocks and of `resource` blocks with a `resource.text` string is summed, with one rounding.
pub(crate) fn estimate_output_tokens(response: Option<&Value>) -> Option<i32> {
    let response = match response {
        None | Some(Value::Null) => return None,
        Some(value) => value,
    };
    let Some(content) = response.get("content").and_then(Value::as_array) else {
        return Some(json_tokens(response));
    };
    if content.is_empty() {
        return Some(match response.get("structuredContent") {
            None | Some(Value::Null) => 0,
            Some(structured) => json_tokens(structured),
        });
    }
    let bytes: u64 = content.iter().map(block_bytes).sum();
    Some(estimate_tokens(bytes))
}

/// The UTF-8 bytes one content block counts: its `text` for a `text` block, its `resource.text`
/// for a `resource` block, `0` for any other block or a non-string text.
fn block_bytes(block: &Value) -> u64 {
    let text = match block.get("type").and_then(Value::as_str) {
        Some("text") => block.get("text"),
        Some("resource") => block.get("resource").and_then(|r| r.get("text")),
        _ => None,
    };
    text.and_then(Value::as_str)
        .map_or(0, |text| byte_count(text.len()))
}

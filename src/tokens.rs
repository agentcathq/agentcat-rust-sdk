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
const MAX_TOKEN_COUNT: u32 = 2_147_483_647;

/// `ceil(bytes / 3.5)`, 0 for nothing, clamped to `i32::MAX`.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the cast value is a whole number between 1 and i32::MAX"
)]
pub(crate) fn estimate_tokens(bytes: f64) -> u32 {
    if bytes.is_nan() || bytes <= 0.0 {
        return 0;
    }
    let tokens = (bytes / TOKEN_ESTIMATE_BYTES_PER_TOKEN).ceil();
    if tokens >= f64::from(MAX_TOKEN_COUNT) {
        MAX_TOKEN_COUNT
    } else {
        tokens as u32
    }
}

/// UTF-8 length of the compact JSON, the same bytes as JavaScript's
/// `JSON.stringify` (which writes numbers its own way: `10`, not `10.0`;
/// `100000000000000000000`, not `1e20`).
fn compact_json_bytes(value: &Value) -> Option<usize> {
    fn len(value: &Value) -> Option<usize> {
        Some(match value {
            Value::Null | Value::Bool(true) => 4,
            Value::Bool(false) => 5,
            Value::Number(n) => js_number_len(n),
            Value::String(_) => serde_json::to_vec(value).ok()?.len(),
            Value::Array(items) => {
                let inner: usize = items.iter().map(len).sum::<Option<usize>>()?;
                2 + inner + items.len().saturating_sub(1)
            }
            Value::Object(map) => {
                let mut total = 2 + map.len().saturating_sub(1);
                for (k, v) in map {
                    total += serde_json::to_vec(k).ok()?.len() + 1 + len(v)?;
                }
                total
            }
        })
    }
    len(value)
}

/// Length of JavaScript's `Number.prototype.toString` for a JSON number.
fn js_number_len(n: &serde_json::Number) -> usize {
    if n.is_i64() || n.is_u64() {
        return n.to_string().len();
    }
    let Some(f) = n.as_f64() else {
        return n.to_string().len();
    };
    crate::js::number_to_string(f).len()
}

/// Tokens the model spent emitting the call: the raw arguments object.
/// `None` when the call has no arguments.
#[allow(clippy::cast_precision_loss, reason = "byte counts stay far below 2^53")]
pub(crate) fn estimate_input_tokens(args: Option<&Value>) -> Option<u32> {
    match args {
        None | Some(Value::Null) => None,
        Some(v) => compact_json_bytes(v).map(|b| estimate_tokens(b as f64)),
    }
}

/// Tokens the model reads back: the text of the content blocks. Falls back
/// to the whole response when it carries no content list; `None` when there
/// is no response.
#[allow(clippy::cast_precision_loss, reason = "byte counts stay far below 2^53")]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_length_matches_json_stringify() {
        let v: Value = serde_json::from_str(r#"{"a":10.0,"b":[1e20,true,null],"c":"x\"y"}"#).unwrap();
        // JSON.stringify: {"a":10,"b":[100000000000000000000,true,null],"c":"x\"y"}
        assert_eq!(
            compact_json_bytes(&v),
            Some(r#"{"a":10,"b":[100000000000000000000,true,null],"c":"x\"y"}"#.len())
        );
    }
}

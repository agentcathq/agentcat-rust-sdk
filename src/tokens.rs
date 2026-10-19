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
    js_number_string(f).len()
}

/// JavaScript's `Number.prototype.toString` (ECMA-262 `Number::toString`) for a
/// finite double.
pub(crate) fn js_number_string(f: f64) -> String {
    if f == 0.0 {
        return "0".into();
    }
    if !f.is_finite() {
        return "null".into(); // JSON.stringify writes non-finite numbers as null
    }
    // Rust's `{:e}` gives the shortest round-trip digits, as JS requires.
    let sci = format!("{:e}", f.abs());
    let (mantissa, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let exp: i32 = exp.parse().unwrap_or(0);
    let k = i32::try_from(digits.len()).unwrap_or(i32::MAX);
    let n = exp + 1;
    let body = if k <= n && n <= 21 {
        format!("{digits}{}", "0".repeat(usize::try_from(n - k).unwrap_or(0)))
    } else if 0 < n && n <= 21 {
        let n = usize::try_from(n).unwrap_or(0);
        format!("{}.{}", &digits[..n], &digits[n..])
    } else if -6 < n && n <= 0 {
        format!("0.{}{digits}", "0".repeat(usize::try_from(-n).unwrap_or(0)))
    } else {
        let sign = if n > 0 { "+" } else { "-" };
        let e = (n - 1).abs();
        if k == 1 {
            format!("{digits}e{sign}{e}")
        } else {
            format!("{}.{}e{sign}{e}", &digits[..1], &digits[1..])
        }
    };
    if f < 0.0 { format!("-{body}") } else { body }
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
    fn numbers_format_like_javascript() {
        for (f, js) in [
            (10.0, "10"),
            (1.5, "1.5"),
            (-0.25, "-0.25"),
            (1e20, "100000000000000000000"),
            (1e21, "1e+21"),
            (1.5e21, "1.5e+21"),
            (0.000001, "0.000001"),
            (1e-7, "1e-7"),
            (1.2345e-7, "1.2345e-7"),
            (123.456, "123.456"),
            (0.1, "0.1"),
        ] {
            assert_eq!(js_number_string(f), js, "{f}");
        }
    }

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

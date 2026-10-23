//! JavaScript semantics the TS SDK relies on: `JSON.stringify`, the order of an object's own
//! properties, string lengths and cuts in UTF-16 code units, `String.prototype.trim`,
//! `String(value)` and `typeof`. Token estimates and the event-size check count the bytes of
//! `JSON.stringify` output, so these follow JavaScript, not Rust or `serde_json`.
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use serde_json::{Map, Number, Value};

#[cfg(test)]
mod tests;

/// `JSON.stringify(value)`: no whitespace, object keys in [`own_entries`] order, numbers as
/// [`number_to_string`] writes them, and strings escaped as JSON.stringify escapes them.
pub(crate) fn stringify(value: &Value) -> String {
    let mut out = String::new();
    push_json(&mut out, value);
    out
}

fn push_json(out: &mut String, value: &Value) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => push_number(out, n),
        Value::String(s) => push_quoted(out, s),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                push_json(out, item);
            }
            out.push(']');
        }
        Value::Object(object) => {
            out.push('{');
            for (i, (key, item)) in own_entries(object).into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                push_quoted(out, key);
                out.push(':');
                push_json(out, item);
            }
            out.push('}');
        }
    }
}

/// `text` as a JSON string literal, as JSON.stringify writes it (`QuoteJSONString` in
/// ECMA-262): `"` and `\` get a backslash, U+0008, U+0009, U+000A, U+000C and U+000D get
/// `\b`, `\t`, `\n`, `\f` and `\r`, the other characters below U+0020 get `\u00xx` with
/// lowercase hex digits, and every other character stays as it is (U+007F, U+2028 and U+2029
/// too). A `str` holds no lone surrogate, the one other character JavaScript escapes.
fn push_quoted(out: &mut String, text: &str) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    out.reserve(text.len() + 2);
    out.push('"');
    // Every byte to escape is ASCII, so `copied` and `at` are always on character boundaries.
    let mut copied = 0;
    for (at, byte) in text.bytes().enumerate() {
        let escape = match byte {
            b'"' => "\\\"",
            b'\\' => "\\\\",
            0x08 => "\\b",
            b'\t' => "\\t",
            b'\n' => "\\n",
            0x0C => "\\f",
            b'\r' => "\\r",
            0x00..=0x1F => "\\u00",
            _ => continue,
        };
        out.push_str(&text[copied..at]);
        out.push_str(escape);
        if escape == "\\u00" {
            out.push(char::from(HEX[usize::from(byte >> 4)]));
            out.push(char::from(HEX[usize::from(byte & 0x0F)]));
        }
        copied = at + 1;
    }
    out.push_str(&text[copied..]);
    out.push('"');
}

/// `value` as TS sends it (C40): its [`stringify()`] text read back with `serde_json`, so that
/// `serde_json` writes the integers, the `-0` (as `0`), the integers above 2^53 (rounded as
/// `JSON.parse` rounds them) and the order of index-like keys as `JSON.stringify` does. A float
/// (a number whose `JSON.stringify` text is not a 64-bit integer) reads back to the same `f64`, which
/// `serde_json` writes in its own text (`1e-6` for JavaScript's `0.000001`, RD-32). A value nested
/// deeper than `serde_json` reads (128 levels) does not read back and comes back unchanged.
pub(crate) fn to_js_value(value: &Value) -> Value {
    serde_json::from_str(&stringify(value)).unwrap_or_else(|_| value.clone())
}

/// JavaScript's `Number.prototype.toString()` of `number` as an `f64`: the shortest text that
/// reads back as the same `f64`, with an exponent from 1e21 up and below 1e-6 (`1e+21`, `1e-7`),
/// and `"0"` for `-0`. An integer beyond 2^53 is first rounded to the nearest `f64`, as
/// `JSON.parse` rounds it (`18446744073709551615` gives `"18446744073709552000"`).
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "read by the delivery pipeline (plan 4)")
)]
pub(crate) fn number_to_string(number: &Number) -> String {
    let mut out = String::new();
    push_number(&mut out, number);
    out
}

fn push_number(out: &mut String, number: &Number) {
    // `as_f64` is `None` only with serde_json's `arbitrary_precision` feature, which this crate
    // does not turn on; the number's own text is the closest fallback.
    match number.as_f64() {
        Some(f) => out.push_str(ryu_js::Buffer::new().format(f)),
        None => out.push_str(&number.to_string()),
    }
}

/// The entries of `object` in JavaScript's own-property order (`OrdinaryOwnPropertyKeys`): first
/// the keys that are array indices (the canonical decimal text of an integer from 0 to
/// 4 294 967 294, so not `"01"`, `"-1"` or `"4294967295"`) in ascending numeric order, then the
/// other keys in insertion order. `serde_json`'s `preserve_order` map keeps insertion order.
pub(crate) fn own_entries(object: &Map<String, Value>) -> Vec<(&String, &Value)> {
    let mut indices = Vec::new();
    let mut others = Vec::with_capacity(object.len());
    for entry in object {
        match array_index(entry.0) {
            Some(index) => indices.push((index, entry)),
            None => others.push(entry),
        }
    }
    if indices.is_empty() {
        return others;
    }
    // Keys are unique, so no two entries have the same index.
    indices.sort_unstable_by_key(|&(index, _)| index);
    indices
        .into_iter()
        .map(|(_, entry)| entry)
        .chain(others)
        .collect()
}

/// The array index `key` is: `Some` for the canonical decimal text of an integer from 0 to
/// 2^32 - 2 (no sign, no leading zero), `None` for any other key.
fn array_index(key: &str) -> Option<u32> {
    let canonical = match key.as_bytes() {
        [b'0'] => true,
        [b'1'..=b'9', rest @ ..] => rest.len() < 10 && rest.iter().all(u8::is_ascii_digit),
        _ => false,
    };
    if !canonical {
        return None;
    }
    key.parse::<u32>().ok().filter(|&index| index != u32::MAX)
}

/// The length of `text` in UTF-16 code units, JavaScript's `text.length`: a character outside
/// the BMP counts 2.
pub(crate) fn utf16_len(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

/// The longest prefix of `text` of at most `units` UTF-16 code units that ends on a character
/// boundary. Where JavaScript's `text.slice(0, units)` ends inside a surrogate pair (half of a
/// character outside the BMP), this drops the whole character (RD-22).
pub(crate) fn cut_utf16(text: &str, units: usize) -> &str {
    // A character takes at least as many UTF-8 bytes as UTF-16 units.
    if text.len() <= units {
        return text;
    }
    let mut used = 0;
    for (at, c) in text.char_indices() {
        used += c.len_utf16();
        if used > units {
            return &text[..at];
        }
    }
    text
}

/// `text.trim()`: `text` without leading and trailing JavaScript whitespace (see
/// [`is_js_whitespace`]).
pub(crate) fn trim(text: &str) -> &str {
    text.trim_matches(is_js_whitespace)
}

/// Whether `String.prototype.trim` removes `c`: the `WhiteSpace` and `LineTerminator` code points
/// of ECMA-262, which are U+0009 to U+000D, U+0020, U+00A0, U+1680, U+2000 to U+200A, U+2028,
/// U+2029, U+202F, U+205F, U+3000 and U+FEFF. This is not [`char::is_whitespace`], which also
/// takes U+0085 and leaves out U+FEFF.
fn is_js_whitespace(c: char) -> bool {
    matches!(
        c,
        '\u{9}'..='\u{D}'
            | ' '
            | '\u{A0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200A}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202F}'
            | '\u{205F}'
            | '\u{3000}'
            | '\u{FEFF}'
    )
}

/// JavaScript's `String(value)`, where `None` is `undefined`: `"undefined"`, `"null"`,
/// `"true"`, `"false"`, a number as [`number_to_string`] writes it, a string as it is, an array
/// as `Array.prototype.join` gives it (its elements as this function writes them, a `null`
/// element as the empty string, joined with `,`), and `"[object Object]"` for an object.
///
/// RD-27 (owner ruling R23): JavaScript throws `TypeError: Cannot convert object to primitive
/// value` for an object with an own `toString` key (whose value, a JSON value, is never a
/// function), alone or at any depth inside an array; this function gives `"[object Object]"`
/// for it, as for any other object.
pub(crate) fn to_js_string(value: Option<&Value>) -> String {
    let mut out = String::new();
    match value {
        None => out.push_str("undefined"),
        Some(value) => push_js_string(&mut out, value),
    }
    out
}

fn push_js_string(out: &mut String, value: &Value) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => push_number(out, n),
        Value::String(s) => out.push_str(s),
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                if !item.is_null() {
                    push_js_string(out, item);
                }
            }
        }
        Value::Object(_) => out.push_str("[object Object]"),
    }
}

/// JavaScript's `typeof value`, where `None` is `undefined`: `"undefined"`, `"boolean"`,
/// `"number"`, `"string"`, and `"object"` for `null`, an array and an object.
pub(crate) fn type_of(value: Option<&Value>) -> &'static str {
    match value {
        None => "undefined",
        Some(Value::Bool(_)) => "boolean",
        Some(Value::Number(_)) => "number",
        Some(Value::String(_)) => "string",
        Some(Value::Null | Value::Array(_) | Value::Object(_)) => "object",
    }
}

/// `time.toISOString()` of JavaScript: `YYYY-MM-DDTHH:MM:SS.mmmZ` with exactly three millisecond
/// digits, the milliseconds cut, not rounded. A time outside the years 0 to 9999 (where
/// JavaScript writes a signed six-digit year) is written with chrono's year form; the SDK only
/// writes the present. A time outside chrono's range (about 262,000 years either way) gives
/// `1970-01-01T00:00:00.000Z` and never panics: the only such time comes from a customer's
/// `redact_event` hook, and the restore of `timestamp` replaces it with the original.
pub(crate) fn iso_millis(time: SystemTime) -> String {
    let (secs, nanos) = match time.duration_since(UNIX_EPOCH) {
        Ok(after) => (i64::try_from(after.as_secs()).ok(), after.subsec_nanos()),
        Err(error) => {
            let before = error.duration();
            // Seconds round down: 0.25 s before the epoch is -1 s plus 0.75 s.
            match (before.as_secs(), before.subsec_nanos()) {
                (secs, 0) => (i64::try_from(secs).ok().map(|s| -s), 0),
                (secs, nanos) => (
                    i64::try_from(secs).ok().map(|s| -s - 1),
                    1_000_000_000 - nanos,
                ),
            }
        }
    };
    let date = secs
        .and_then(|secs| DateTime::<Utc>::from_timestamp(secs, nanos))
        .unwrap_or(DateTime::UNIX_EPOCH);
    date.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

/// The time of an RFC 3339 text such as `iso_millis` writes, with or without a fraction of a
/// second; `None` when `text` is not one.
pub(crate) fn parse_iso_millis(text: &str) -> Option<SystemTime> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|time| SystemTime::from(time.with_timezone(&Utc)))
}

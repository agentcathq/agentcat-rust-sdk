//! The rules by which the SDK writes and measures the values it sends: the JSON text of a value
//! (no whitespace, array-index keys first), the order of an object's keys, string lengths and cuts
//! in UTF-16 code units, the whitespace that [`trim`] removes, the string form and the type name of
//! a value, and the timestamp text. Token estimates and the event-size check count the bytes of
//! this JSON text, so these rules are the SDK's own, not those of `serde_json` or of `str`.
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use serde_json::{Map, Number, Value};

#[cfg(test)]
mod tests;

/// The JSON text of `value`: no whitespace, object keys in [`own_entries`] order, numbers as
/// [`number_to_string`] writes them, and strings escaped as [`write_quoted`] escapes them.
pub(crate) fn stringify(value: &Value) -> String {
    let mut out = String::new();
    // Writing to a `String` never fails.
    let _ = write_stringify(value, &mut out);
    out
}

/// Writes the [`stringify()`] text of `value` to `out`, so a caller can count the bytes (or send
/// them on) without building the text.
///
/// # Errors
///
/// Only what `out` returns.
pub(crate) fn write_stringify(value: &Value, out: &mut impl fmt::Write) -> fmt::Result {
    match value {
        Value::Null => out.write_str("null"),
        Value::Bool(b) => out.write_str(if *b { "true" } else { "false" }),
        Value::Number(n) => write_number(out, n),
        Value::String(s) => write_quoted(out, s),
        Value::Array(items) => {
            out.write_char('[')?;
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.write_char(',')?;
                }
                write_stringify(item, out)?;
            }
            out.write_char(']')
        }
        Value::Object(object) => {
            out.write_char('{')?;
            for (i, (key, item)) in own_entries(object).enumerate() {
                if i > 0 {
                    out.write_char(',')?;
                }
                write_quoted(out, key)?;
                out.write_char(':')?;
                write_stringify(item, out)?;
            }
            out.write_char('}')
        }
    }
}

/// `text` as a JSON string literal: `"` and `\` get a backslash, U+0008, U+0009, U+000A, U+000C
/// and U+000D get `\b`, `\t`, `\n`, `\f` and `\r`, the other characters below U+0020 get
/// `\u00xx` with lowercase hex digits, and every other character stays as it is (U+007F, U+2028
/// and U+2029 too). A `str` holds no lone surrogate, so no surrogate gets an escape.
pub(crate) fn write_quoted(out: &mut impl fmt::Write, text: &str) -> fmt::Result {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    out.write_char('"')?;
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
        out.write_str(&text[copied..at])?;
        out.write_str(escape)?;
        if escape == "\\u00" {
            out.write_char(char::from(HEX[usize::from(byte >> 4)]))?;
            out.write_char(char::from(HEX[usize::from(byte & 0x0F)]))?;
        }
        copied = at + 1;
    }
    out.write_str(&text[copied..])?;
    out.write_char('"')
}

/// `value` as the SDK sends it in a publish request: its [`stringify()`] text read back with
/// `serde_json`, so that `serde_json` writes the integers, the `-0` (as `0`), the integers above
/// 2^53 (rounded to the nearest `f64`) and the order of array-index keys as [`stringify()`] does. A
/// float (a number whose [`stringify()`] text is not a 64-bit integer) reads back to the same
/// `f64`, which `serde_json` writes in its own text (`1e-6` where [`stringify()`] writes
/// `0.000001`). A value nested deeper than `serde_json` reads (128 levels) does not read back and
/// comes back unchanged.
pub(crate) fn round_trip(value: &Value) -> Value {
    serde_json::from_str(&stringify(value)).unwrap_or_else(|_| value.clone())
}

/// The text of `number` as an `f64`, as the `ryu-js` crate writes it: the shortest text that
/// reads back as the same `f64`, with an exponent for a magnitude from 1e21 up and below 1e-6
/// (`1e+21`, `1e-7`), and `"0"` for `-0`. An integer beyond 2^53 is first rounded to the nearest
/// `f64` (`18446744073709551615` gives `"18446744073709552000"`).
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "only the tests read the number text on its own")
)]
pub(crate) fn number_to_string(number: &Number) -> String {
    let mut out = String::new();
    // Writing to a `String` never fails.
    let _ = write_number(&mut out, number);
    out
}

fn write_number(out: &mut impl fmt::Write, number: &Number) -> fmt::Result {
    // `as_f64` is `None` only with serde_json's `arbitrary_precision` feature, which this crate
    // does not turn on; the number's own text is the closest fallback.
    match number.as_f64() {
        Some(f) => out.write_str(ryu_js::Buffer::new().format(f)),
        None => write!(out, "{number}"),
    }
}

/// The entries of `object` in the order the SDK writes and walks them: first the keys that are
/// array indices (the canonical decimal text of an integer from 0 to 4 294 967 294, so not `"01"`,
/// `"-1"` or `"4294967295"`) in ascending numeric order, then the other keys in insertion order.
/// `serde_json`'s `preserve_order` map keeps insertion order.
pub(crate) fn own_entries(object: &Map<String, Value>) -> OwnEntries<'_> {
    // The common case, an object without an index-like key, is its map's own iterator: no
    // allocation. Each key is checked once: the scan stops at the first index-like key, the
    // entries before it are the first of the other keys, and the loop goes on after it.
    let mut rest = object.iter();
    let mut before = 0;
    let first = loop {
        let Some(entry) = rest.next() else {
            return OwnEntries::Insertion(object.iter());
        };
        if let Some(index) = array_index(entry.0) {
            break (index, entry);
        }
        before += 1;
    };
    let mut indices = vec![first];
    let mut others = Vec::with_capacity(object.len());
    others.extend(object.iter().take(before));
    for entry in rest {
        match array_index(entry.0) {
            Some(index) => indices.push((index, entry)),
            None => others.push(entry),
        }
    }
    // Keys are unique, so no two entries have the same index.
    indices.sort_unstable_by_key(|&(index, _)| index);
    let ordered: Vec<(&String, &Value)> = indices
        .into_iter()
        .map(|(_, entry)| entry)
        .chain(others)
        .collect();
    OwnEntries::Reordered(ordered.into_iter())
}

/// The entries [`own_entries`] gives, in its order.
pub(crate) enum OwnEntries<'a> {
    /// An object without index-like keys: its own iterator.
    Insertion(serde_json::map::Iter<'a>),
    /// An object with index-like keys, reordered.
    Reordered(std::vec::IntoIter<(&'a String, &'a Value)>),
}

impl<'a> Iterator for OwnEntries<'a> {
    type Item = (&'a String, &'a Value);

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Insertion(entries) => entries.next(),
            Self::Reordered(entries) => entries.next(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Self::Insertion(entries) => entries.size_hint(),
            Self::Reordered(entries) => entries.size_hint(),
        }
    }
}

impl ExactSizeIterator for OwnEntries<'_> {}

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

/// The length of `text` in UTF-16 code units, the unit of every length limit of the SDK: a
/// character outside the BMP counts 2.
pub(crate) fn utf16_len(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

/// The longest prefix of `text` of at most `units` UTF-16 code units that ends on a character
/// boundary. Where a cut after `units` units would end inside a surrogate pair (half of a
/// character outside the BMP), the prefix drops the whole character.
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

/// `text` without leading and trailing whitespace, as [`is_trim_whitespace`] defines it.
pub(crate) fn trim(text: &str) -> &str {
    text.trim_matches(is_trim_whitespace)
}

/// Whether [`trim`] removes `c`: the white space and line terminator code points U+0009 to
/// U+000D, U+0020, U+00A0, U+1680, U+2000 to U+200A, U+2028, U+2029, U+202F, U+205F, U+3000 and
/// U+FEFF. This is not [`char::is_whitespace`], which also takes U+0085 and leaves out U+FEFF.
fn is_trim_whitespace(c: char) -> bool {
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

/// The string form of `value`, where `None` is an absent value: `"undefined"` for `None`,
/// `"null"`, `"true"`, `"false"`, a number as [`number_to_string`] writes it, a string as it is,
/// an array as its elements in this form joined with `,` (a `null` element as the empty string),
/// and `"[object Object]"` for an object.
///
/// Every object gives `"[object Object]"`, also an object with a `toString` key (whose value, a
/// JSON value, is never a function), alone or at any depth inside an array.
pub(crate) fn coerce_to_string(value: Option<&Value>) -> String {
    let mut out = String::new();
    match value {
        None => out.push_str("undefined"),
        Some(value) => push_coerced_string(&mut out, value),
    }
    out
}

fn push_coerced_string(out: &mut String, value: &Value) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => {
            // Writing to a `String` never fails.
            let _ = write_number(out, n);
        }
        Value::String(s) => out.push_str(s),
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                if !item.is_null() {
                    push_coerced_string(out, item);
                }
            }
        }
        Value::Object(_) => out.push_str("[object Object]"),
    }
}

/// Whether a JSON value is truthy: `false`, `0`, `""` and `null` are falsy; every other value,
/// an empty array or object too, is truthy.
pub(crate) fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// The type name of `value`, where `None` is an absent value: `"undefined"` for `None`,
/// `"boolean"`, `"number"`, `"string"`, and `"object"` for `null`, an array and an object.
pub(crate) fn type_of(value: Option<&Value>) -> &'static str {
    match value {
        None => "undefined",
        Some(Value::Bool(_)) => "boolean",
        Some(Value::Number(_)) => "number",
        Some(Value::String(_)) => "string",
        Some(Value::Null | Value::Array(_) | Value::Object(_)) => "object",
    }
}

/// `time` as a UTC timestamp text: `YYYY-MM-DDTHH:MM:SS.mmmZ` with exactly three millisecond
/// digits, the milliseconds cut, not rounded. A time outside the years 0 to 9999 is written with
/// chrono's year form; the SDK only writes the present. A time outside chrono's range (about
/// 262,000 years either way) gives `1970-01-01T00:00:00.000Z` and never panics: the only such time
/// comes from a customer's `redact_event` hook, and the restore of `timestamp` replaces it with the
/// original.
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

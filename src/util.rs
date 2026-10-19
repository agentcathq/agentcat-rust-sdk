//! Small helpers shared by the modules: clocks, hashing, JS-compatible text.

use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

pub(crate) fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub(crate) fn unix_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// Formats a time like JavaScript's `Date.prototype.toISOString`:
/// `YYYY-MM-DDTHH:mm:ss.sssZ`.
pub(crate) fn iso_millis(time: &chrono::DateTime<chrono::Utc>) -> String {
    time.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub(crate) fn sha256(input: &[u8]) -> [u8; 32] {
    Sha256::digest(input).into()
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(DIGITS[(b >> 4) as usize] as char);
        out.push(DIGITS[(b & 0xf) as usize] as char);
    }
    out
}

/// Length in UTF-16 code units, the unit JavaScript's `String.length` counts.
pub(crate) fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

/// The longest prefix of `s` that is at most `max_units` UTF-16 code units,
/// cut on a `char` boundary. Equal to JavaScript's `s.slice(0, max_units)`
/// except that it never splits a surrogate pair (JS would leave a lone
/// surrogate, which is not valid UTF-8).
pub(crate) fn utf16_prefix(s: &str, max_units: usize) -> &str {
    let mut units = 0;
    for (idx, ch) in s.char_indices() {
        let next = units + ch.len_utf16();
        if next > max_units {
            return &s[..idx];
        }
        units = next;
    }
    s
}

/// JavaScript-style truncation: when `s` is longer than `max_units` UTF-16
/// code units, keep the first `max_units` and append `...`.
pub(crate) fn truncate_js(s: &str, max_units: usize) -> Option<String> {
    if utf16_len(s) <= max_units {
        None
    } else {
        Some(format!("{}...", utf16_prefix(s, max_units)))
    }
}

/// Caps `s` to at most `max_chars` Unicode scalar values in total, ending in
/// `...` when cut. Used for the server's varchar columns, which count code
/// points.
pub(crate) fn cap_chars(s: &str, max_chars: usize) -> Option<String> {
    if s.chars().count() <= max_chars {
        return None;
    }
    let keep = max_chars.saturating_sub(3);
    let end = s.char_indices().nth(keep).map(|(i, _)| i).unwrap_or(s.len());
    Some(format!("{}...", &s[..end]))
}

/// JavaScript's `String.prototype.trim` whitespace set: Rust's `char::is_whitespace`
/// without U+0085 (NEL), plus U+FEFF (BOM).
pub(crate) fn js_trim(s: &str) -> &str {
    s.trim_matches(|c: char| (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}')
}

/// Runs `f`, turning a panic into `None`. AgentCat code on the request path
/// uses this so that its own bugs never reach the customer's server.
pub(crate) fn catch<T>(f: impl FnOnce() -> T) -> Option<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).ok()
}

/// The message of a panic payload.
pub(crate) fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        (*s).to_owned()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "Box<dyn Any>".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_helpers_match_js() {
        assert_eq!(utf16_len("abc"), 3);
        assert_eq!(utf16_len("é"), 1);
        assert_eq!(utf16_len("😀"), 2);
        assert_eq!(utf16_prefix("a😀b", 2), "a");
        assert_eq!(utf16_prefix("a😀b", 3), "a😀");
        assert_eq!(truncate_js("abcdef", 3).as_deref(), Some("abc..."));
        assert_eq!(truncate_js("abc", 3), None);
    }

    #[test]
    fn cap_chars_counts_code_points() {
        assert_eq!(cap_chars("abcdef", 6), None);
        assert_eq!(cap_chars("abcdefg", 6).as_deref(), Some("abc..."));
        assert_eq!(cap_chars("ééééééé", 6).as_deref(), Some("ééé..."));
    }

    #[test]
    fn js_trim_matches_js() {
        assert_eq!(js_trim("\u{feff} x \u{a0}\n"), "x");
    }

    #[test]
    fn iso_matches_to_iso_string() {
        let t = chrono::DateTime::from_timestamp_millis(1_760_000_000_123).unwrap();
        assert_eq!(iso_millis(&t), "2025-10-09T08:53:20.123Z");
    }
}

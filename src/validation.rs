//! Customer tag validation, with the TypeScript SDK's rules.

use crate::js::type_of;
use crate::logging::write_to_log;
use crate::util::utf16_len;
use serde_json::Value;

const MAX_TAG_KEY_LENGTH: usize = 32;
const MAX_TAG_VALUE_LENGTH: usize = 200;
const MAX_TAG_ENTRIES: usize = 50;

/// `/^[a-zA-Z0-9$_.:\- ]+$/`
fn valid_key_chars(key: &str) -> bool {
    !key.is_empty()
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'$' | b'_' | b'.' | b':' | b'-' | b' '))
}

fn drop_tag(key: &str, reason: &str) {
    write_to_log(&format!("Dropping invalid tag: \"{key}\" — {reason}"));
}

/// Validates customer tags in their given order. Invalid entries are dropped
/// with a log line; valid entries past the 50th are dropped. A repeated key
/// keeps its first position and its last value. `None` when nothing valid
/// remains.
pub(crate) fn validate_tags(
    tags: impl IntoIterator<Item = (String, Value)>,
) -> Option<Vec<(String, String)>> {
    let mut valid: Vec<(String, String)> = Vec::new();
    for (key, value) in tags {
        if !valid_key_chars(&key) {
            drop_tag(&key, "key contains invalid characters or is empty");
            continue;
        }
        if utf16_len(&key) > MAX_TAG_KEY_LENGTH {
            drop_tag(&key, &format!("key exceeds max length of {MAX_TAG_KEY_LENGTH}"));
            continue;
        }
        let Value::String(value) = value else {
            drop_tag(&key, &format!("non-string value (got {})", type_of(&value)));
            continue;
        };
        if utf16_len(&value) > MAX_TAG_VALUE_LENGTH {
            drop_tag(
                &key,
                &format!("value exceeds max length of {MAX_TAG_VALUE_LENGTH}"),
            );
            continue;
        }
        if value.contains('\n') {
            drop_tag(&key, "value contains newline character");
            continue;
        }
        valid.push((key, value));
    }
    if valid.is_empty() {
        return None;
    }
    if valid.len() > MAX_TAG_ENTRIES {
        let dropped = valid.len() - MAX_TAG_ENTRIES;
        write_to_log(&format!(
            "Dropping {dropped} tag(s) — exceeds maximum of {MAX_TAG_ENTRIES} entries per event"
        ));
        valid.truncate(MAX_TAG_ENTRIES);
    }
    let mut merged: Vec<(String, String)> = Vec::with_capacity(valid.len());
    for (key, value) in valid {
        match merged.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = value,
            None => merged.push((key, value)),
        }
    }
    Some(merged)
}

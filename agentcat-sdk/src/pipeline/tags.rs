//! Tag validation (`validateTags` of the TS SDK, B-093).
use serde_json::{Map, Value};

use crate::js;

#[cfg(test)]
mod tests;

/// The longest tag key, in UTF-16 units.
const MAX_TAG_KEY_LENGTH: usize = 32;
/// The longest tag value, in UTF-16 units.
const MAX_TAG_VALUE_LENGTH: usize = 200;
/// The most tags one event keeps.
const MAX_TAG_ENTRIES: usize = 50;

/// `true` when `key` matches TS's `/^[a-zA-Z0-9$_.:\- ]+$/`: one or more of those characters.
fn is_valid_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '$' | '_' | '.' | ':' | '-' | ' '))
}

/// Validates the customer tags of an event, as TS's `validateTags` does.
///
/// It walks `tags` in JavaScript's own-property order ([`js::own_entries`]). An entry whose key
/// does not match the pattern, whose key is over 32 UTF-16 units, whose value is not a string,
/// whose value is over 200 units or whose value holds `\n` is dropped, and the TS log message for
/// it is pushed to `log` (the text after the log head). Of more than 50 survivors the first 50
/// stay, and one more message counts the rest. The result is `None` for an empty input or when no
/// entry survives.
pub(crate) fn validate_tags(
    tags: &Map<String, Value>,
    log: &mut Vec<String>,
) -> Option<Map<String, Value>> {
    if tags.is_empty() {
        return None;
    }
    let mut valid: Vec<(&String, &String)> = Vec::new();
    for (key, value) in js::own_entries(tags) {
        let reason = if !is_valid_key(key) {
            "key contains invalid characters or is empty".to_owned()
        } else if js::utf16_len(key) > MAX_TAG_KEY_LENGTH {
            format!("key exceeds max length of {MAX_TAG_KEY_LENGTH}")
        } else if let Value::String(text) = value {
            if js::utf16_len(text) > MAX_TAG_VALUE_LENGTH {
                format!("value exceeds max length of {MAX_TAG_VALUE_LENGTH}")
            } else if text.contains('\n') {
                "value contains newline character".to_owned()
            } else {
                valid.push((key, text));
                continue;
            }
        } else {
            format!("non-string value (got {})", js::type_of(Some(value)))
        };
        log.push(format!("Dropping invalid tag: \"{key}\" \u{2014} {reason}"));
    }
    if valid.is_empty() {
        return None;
    }
    if valid.len() > MAX_TAG_ENTRIES {
        let dropped = valid.len() - MAX_TAG_ENTRIES;
        log.push(format!(
            "Dropping {dropped} tag(s) \u{2014} exceeds maximum of {MAX_TAG_ENTRIES} entries per event"
        ));
        valid.truncate(MAX_TAG_ENTRIES);
    }
    Some(
        valid
            .into_iter()
            .map(|(key, text)| (key.clone(), Value::String(text.clone())))
            .collect(),
    )
}

//! Customer tag validation, with the `AgentCat` API's own rules.
//!
//! The TypeScript SDK also accepts a space in keys and the `agentcat:` key
//! prefix; the API drops both after answering 202, so this SDK drops them up
//! front, where the drop is logged.

use crate::logging::write_to_log;

const MAX_TAG_KEY_LENGTH: usize = 32;
const MAX_TAG_VALUE_LENGTH: usize = 200;
const MAX_TAG_ENTRIES: usize = 50;
const RESERVED_PREFIX: &str = "agentcat:";

fn valid_key_chars(key: &str) -> bool {
    !key.is_empty()
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'$' | b'_' | b'.' | b':' | b'-'))
}

/// Validates customer tags in their given order. Invalid entries are dropped
/// with a log line; entries past the 50th are dropped. `None` when nothing
/// valid remains.
pub(crate) fn validate_tags(
    tags: impl IntoIterator<Item = (String, String)>,
) -> Option<Vec<(String, String)>> {
    let mut valid = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (key, value) in tags {
        if !valid_key_chars(&key) {
            write_to_log(&format!(
                "Dropping invalid tag: \"{key}\" — key contains invalid characters or is empty"
            ));
            continue;
        }
        if key.chars().count() > MAX_TAG_KEY_LENGTH {
            write_to_log(&format!(
                "Dropping invalid tag: \"{key}\" — key exceeds max length of {MAX_TAG_KEY_LENGTH}"
            ));
            continue;
        }
        if key.starts_with(RESERVED_PREFIX) {
            write_to_log(&format!(
                "Dropping invalid tag: \"{key}\" — the \"{RESERVED_PREFIX}\" key prefix is reserved"
            ));
            continue;
        }
        if value.chars().count() > MAX_TAG_VALUE_LENGTH {
            write_to_log(&format!(
                "Dropping invalid tag: \"{key}\" — value exceeds max length of {MAX_TAG_VALUE_LENGTH}"
            ));
            continue;
        }
        if value.contains('\n') {
            write_to_log(&format!(
                "Dropping invalid tag: \"{key}\" — value contains newline character"
            ));
            continue;
        }
        // A later duplicate key overwrites the earlier one, as in a JS object.
        if !seen.insert(key.clone()) {
            valid.retain(|(k, _): &(String, String)| *k != key);
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
    Some(valid)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(k: &str, v: &str) -> (String, String) {
        (k.to_owned(), v.to_owned())
    }

    #[test]
    fn keeps_valid_tags_in_order() {
        let out = validate_tags([t("env", "prod"), t("trace.id", "abc"), t("$a:b-c_d", "x")]).unwrap();
        assert_eq!(
            out,
            vec![t("env", "prod"), t("trace.id", "abc"), t("$a:b-c_d", "x")]
        );
    }

    #[test]
    fn drops_invalid_entries() {
        assert_eq!(validate_tags([t("", "x")]), None);
        assert_eq!(validate_tags([t("has space", "x")]), None);
        assert_eq!(validate_tags([t("bad/char", "x")]), None);
        assert_eq!(validate_tags([t(&"k".repeat(33), "x")]), None);
        assert_eq!(validate_tags([t("agentcat:x", "x")]), None);
        assert_eq!(validate_tags([t("k", &"v".repeat(201))]), None);
        assert_eq!(validate_tags([t("k", "a\nb")]), None);
        // CR alone and exactly-at-limit values are fine.
        assert!(validate_tags([t("k", "a\rb")]).is_some());
        assert!(validate_tags([t(&"k".repeat(32), &"é".repeat(200))]).is_some());
        // agentcat_ underscore tags are ordinary tags.
        assert!(validate_tags([t("agentcat_x", "y")]).is_some());
    }

    #[test]
    fn caps_at_fifty_in_order() {
        let tags: Vec<_> = (0..60).map(|i| t(&format!("k{i}"), "v")).collect();
        let out = validate_tags(tags).unwrap();
        assert_eq!(out.len(), 50);
        assert_eq!(out[0].0, "k0");
        assert_eq!(out[49].0, "k49");
    }

    #[test]
    fn empty_input_is_none() {
        assert_eq!(validate_tags(Vec::new()), None);
    }
}

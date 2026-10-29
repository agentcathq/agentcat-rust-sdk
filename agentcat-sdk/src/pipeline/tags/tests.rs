//! `validate_tags`: the cases of `tests/vectors/tags.json` and `tests/vectors/tags_cases.json`,
//! and a property test of which tags stay and which log lines the drops give.
use proptest::prelude::*;
use serde_json::{Map, Number, Value};

use super::validate_tags;
use crate::test_support::json::index_keys_first;
use crate::test_support::strategies::{js_string, number, object_key};
use crate::test_support::{assert_js_eq, config, vectors};

/// The tags a case holds: `input`, or `input_text` parsed in insertion order.
fn input_of(case: &Map<String, Value>) -> Map<String, Value> {
    let name = vectors::name(case);
    let value = match (case.get("input"), case.get("input_text")) {
        (Some(input), None) => input.clone(),
        (None, Some(Value::String(text))) => serde_json::from_str(text).expect("input_text"),
        _ => panic!("{name}: needs exactly one of input and input_text"),
    };
    match value {
        Value::Object(map) => map,
        other => panic!("{name}: tags are not an object: {other}"),
    }
}

/// Checks one case: the tags (`null` for none) and the log lines.
fn check_case(case: &Map<String, Value>) {
    let name = vectors::name(case);
    let mut log = Vec::new();
    let actual = validate_tags(&input_of(case), &mut log);
    let actual = actual.map_or(Value::Null, Value::Object);
    assert_js_eq(&actual, &case["output"], name);
    let expected: Vec<&str> = case["logs"]
        .as_array()
        .expect("logs")
        .iter()
        .map(|line| line.as_str().expect("log line"))
        .collect();
    assert_eq!(log, expected, "{name}: logs");
}

#[test]
fn vectors_match_ts() {
    let cases = vectors::cases("tags");
    assert!(cases.len() > 50, "the file has its cases");
    cases.iter().for_each(check_case);
}

/// The cases of `tags_cases.json`: random tags, half of them with 48 to 69 entries, around the
/// 50-tag cap.
#[test]
fn validate_tags_matches_tags_cases() {
    let cases = vectors::cases("tags_cases");
    assert!(cases.len() >= 30, "tags_cases.json has its cases");
    cases.iter().for_each(check_case);
}

/// Whether a tag stays, from the rule: a key of 1 to 32 characters from `a-z`, `A-Z`, `0-9`,
/// `$`, `_`, `.`, `:`, `-` and space, and a string value of at most 200 UTF-16 units without
/// `\n`.
fn is_valid_tag(key: &str, value: &Value) -> bool {
    let key_ok = (1..=32).contains(&key.len())
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "$_.:- ".contains(c));
    let value_ok = value
        .as_str()
        .is_some_and(|text| text.encode_utf16().count() <= 200 && !text.contains('\n'));
    key_ok && value_ok
}

/// A key from the allowed characters, or any object key.
fn tag_key() -> impl Strategy<Value = String> + Clone {
    prop_oneof![
        "[a-zA-Z0-9$_.: -]{0,40}",
        "[0-9]{1,3}",
        // Always a valid key, so enough entries survive to reach the 50-tag cap.
        "[a-z]{3,12}",
        object_key().prop_map(|key| key.chars().take(40).collect::<String>()),
    ]
}

/// A tag value: mostly strings (some with newlines), also numbers and `null`.
fn tag_value() -> impl Strategy<Value = Value> + Clone {
    prop_oneof![
        6 => js_string(120).prop_map(Value::String),
        2 => "[a-z]{190,210}".prop_map(Value::String),
        // Astral characters are 2 UTF-16 units each: 190 to 213 units, the 200-unit limit inside.
        2 => (95_usize..=105, 0_usize..=3)
            .prop_map(|(astral, ascii)| Value::String("😀".repeat(astral) + &"a".repeat(ascii))),
        1 => "[a-z]{0,5}\n[a-z]{0,5}".prop_map(Value::String),
        1 => number().prop_map(Value::Number),
        1 => Just(Value::Null),
        1 => any::<bool>().prop_map(Value::Bool),
        1 => any::<u8>().prop_map(|n| Value::Number(Number::from(n))),
    ]
}

proptest! {
    #![proptest_config(config())]

    /// The valid tags stay, in the key order of [`index_keys_first`], up to 50 of them; none
    /// stay for no valid tag. Each invalid tag gives a log line that names its key, in that
    /// order, and the valid tags past the 50th give one line that counts them.
    #[test]
    fn validate_tags_keeps_the_first_50_valid_tags_and_logs_each_drop(
        // Half the cases hold 48 to 69 entries, around the 50-tag cap.
        entries in prop_oneof![
            proptest::collection::vec((tag_key(), tag_value()), 0..70),
            proptest::collection::vec((tag_key(), tag_value()), 48..70),
        ],
    ) {
        let tags: Map<String, Value> = entries.into_iter().collect();
        let mut log = Vec::new();
        let actual = validate_tags(&tags, &mut log);
        let order = index_keys_first(tags.keys().map(String::as_str));
        let (valid, invalid): (Vec<&str>, Vec<&str>) =
            order.into_iter().partition(|key| is_valid_tag(key, &tags[*key]));
        let kept: Vec<(&str, &Value)> = valid.iter().take(50).map(|key| (*key, &tags[*key])).collect();
        match actual {
            None => prop_assert!(kept.is_empty()),
            Some(actual) => {
                let actual: Vec<(&str, &Value)> = actual.iter().map(|(key, value)| (key.as_str(), value)).collect();
                prop_assert_eq!(actual, kept);
            }
        }
        let extra = valid.len().saturating_sub(50);
        prop_assert_eq!(log.len(), invalid.len() + usize::from(extra > 0));
        for (line, key) in log.iter().zip(&invalid) {
            let head = format!("Dropping invalid tag: \"{key}\" \u{2014} ");
            prop_assert!(line.starts_with(&head), "{}", line);
        }
        if extra > 0 {
            let line = format!("Dropping {extra} tag(s) \u{2014} exceeds maximum of 50 entries per event");
            prop_assert_eq!(log.last(), Some(&line));
        }
    }
}

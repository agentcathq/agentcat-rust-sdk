//! `validate_tags` against TS: the vectors of `tests/vectors/tags.json`, and a property test that
//! runs the same inputs through the JS oracle.
use proptest::prelude::*;
use serde_json::{Map, Number, Value};

use super::validate_tags;
use crate::test_support::strategies::{js_string, number, object_key};
use crate::test_support::{assert_js_eq, config, oracle, vectors};

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

#[test]
fn vectors_match_ts() {
    let cases = vectors::cases("tags");
    assert!(cases.len() > 50, "the file has its cases");
    for case in &cases {
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
}

/// A key from the allowed characters, or any object key.
fn tag_key() -> impl Strategy<Value = String> + Clone {
    prop_oneof![
        "[a-zA-Z0-9$_.: -]{0,40}",
        "[0-9]{1,3}",
        object_key().prop_map(|key| key.chars().take(40).collect::<String>()),
    ]
}

/// A tag value: mostly strings (some with newlines), also numbers and `null`.
fn tag_value() -> impl Strategy<Value = Value> + Clone {
    prop_oneof![
        6 => js_string(120).prop_map(Value::String),
        2 => "[a-z]{190,210}".prop_map(Value::String),
        1 => "[a-z]{0,5}\n[a-z]{0,5}".prop_map(Value::String),
        1 => number().prop_map(Value::Number),
        1 => Just(Value::Null),
        1 => any::<bool>().prop_map(Value::Bool),
        1 => any::<u8>().prop_map(|n| Value::Number(Number::from(n))),
    ]
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn validate_tags_matches_ts(
        entries in proptest::collection::vec((tag_key(), tag_value()), 0..70),
    ) {
        let tags: Map<String, Value> = entries.into_iter().collect();
        let mut log = Vec::new();
        let actual = validate_tags(&tags, &mut log);
        let ts = oracle::call("validation.validateTags", &[Value::Object(tags)])
            .unwrap()
            .expect("an object");
        let expected = ts["value"].clone();
        assert_js_eq(&actual.map_or(Value::Null, Value::Object), &expected, "tags");
        let ts_logs: Vec<String> = ts["logs"]
            .as_array()
            .expect("logs")
            .iter()
            .map(|line| line.as_str().expect("log line").to_owned())
            .collect();
        prop_assert_eq!(log, ts_logs);
    }
}

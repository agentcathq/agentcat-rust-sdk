//! `normalize` against TS: the vectors of `tests/vectors/normalize.json`, and property tests
//! that run the same inputs through the JS oracle.
use proptest::prelude::*;
use serde_json::{Map, Value};

use super::{MAX_BREADTH, MAX_DEPTH, MAX_STRING_LENGTH, normalize};
use crate::test_support::strategies::json_value;
use crate::test_support::{assert_js_eq, config_capped, oracle, vectors};

/// The value a case holds: `input`, or `input_text` parsed in insertion order.
fn input_of(case: &Map<String, Value>) -> Value {
    let name = vectors::name(case);
    match (case.get("input"), case.get("input_text")) {
        (Some(input), None) => input.clone(),
        (None, Some(Value::String(text))) => serde_json::from_str(text).expect("input_text"),
        _ => panic!("{name}: needs exactly one of input and input_text"),
    }
}

/// The `[depth, breadth, max_string]` a case holds, or the defaults.
fn limits_of(case: &Map<String, Value>) -> (usize, usize, usize) {
    let mut limits = [MAX_DEPTH, MAX_BREADTH, MAX_STRING_LENGTH];
    if let Some(Value::Array(given)) = case.get("limits") {
        for (slot, value) in limits.iter_mut().zip(given) {
            *slot = usize::try_from(value.as_u64().expect("limit")).expect("limit fits");
        }
    }
    (limits[0], limits[1], limits[2])
}

/// `true` when `value` has a `__proto__` key anywhere.
fn has_proto_key(value: &Value) -> bool {
    match value {
        Value::Object(map) => map.contains_key("__proto__") || map.values().any(has_proto_key),
        Value::Array(items) => items.iter().any(has_proto_key),
        _ => false,
    }
}

/// `value` without any `__proto__` key.
fn without_proto(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(key, _)| key.as_str() != "__proto__")
                .map(|(key, item)| (key.clone(), without_proto(item)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(without_proto).collect()),
        other => other.clone(),
    }
}

#[test]
fn vectors_match_ts() {
    let cases = vectors::cases("normalize");
    assert!(cases.len() > 60, "the file has its cases");
    for case in &cases {
        let name = vectors::name(case);
        let (depth, breadth, max_string) = limits_of(case);
        let actual = normalize(&input_of(case), depth, breadth, max_string);
        if case.get("deviation").and_then(Value::as_str) == Some("RD-24") {
            // RD-24: Rust keeps the `__proto__` key, with a normalized value.
            assert!(
                has_proto_key(&actual),
                "{name}: Rust keeps the __proto__ key"
            );
            let Value::Object(top) = input_of(case) else {
                panic!("{name}: an object input");
            };
            let Some(proto) = top.get("__proto__") else {
                panic!("{name}: a top-level __proto__ key");
            };
            let kept = normalize(proto, depth.saturating_sub(1), breadth, max_string);
            assert_eq!(
                actual["__proto__"], kept,
                "{name}: normalized __proto__ value"
            );
            assert_js_eq(&without_proto(&actual), &case["output"], name);
        } else {
            assert_js_eq(&actual, &case["output"], name);
        }
    }
}

/// Runs `normalize` in Rust and in TS on `input` and compares them.
fn compare(input: &Value, depth: usize, breadth: usize, max_string: usize) {
    let actual = normalize(input, depth, breadth, max_string);
    let args = [
        input.clone(),
        depth.into(),
        breadth.into(),
        max_string.into(),
    ];
    let ts = oracle::call("truncation.normalize", &args)
        .unwrap()
        .expect("a value");
    assert_js_eq(&actual, &ts, "normalize");
}

proptest! {
    #![proptest_config(config_capped(256))]

    #[test]
    fn normalize_matches_ts_small_limits(
        input in json_value(6, 6),
        depth in 0usize..6,
        breadth in 0usize..6,
        max_string in 0usize..20,
    ) {
        compare(&input, depth, breadth, max_string);
    }

    #[test]
    fn normalize_matches_ts_default_limits(input in json_value(12, 4)) {
        compare(&input, MAX_DEPTH, MAX_BREADTH, MAX_STRING_LENGTH);
    }
}

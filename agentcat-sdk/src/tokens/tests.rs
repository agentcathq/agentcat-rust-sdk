//! `tokens` against TS: the vectors of `tests/vectors/tokens.json`, and property tests that run
//! the same inputs through the JS oracle.
use std::collections::BTreeSet;

use proptest::prelude::*;
use serde_json::{Map, Value, json};

use super::{estimate_input_tokens, estimate_output_tokens, estimate_tokens};
use crate::test_support::strategies::{js_string, json_value, object_key};
use crate::test_support::{config, oracle, vectors};

/// The case name prefixes of `tokens.json`, and `divisor` (a case without a prefix).
const KINDS: [&str; 5] = ["bytes", "utf8", "input", "output", "divisor"];

/// What a case expects: a token count, or `None` where TS gives `undefined` (the file has `null`).
fn expected(case: &Map<String, Value>) -> Option<i32> {
    let output = &case["output"];
    if output.is_null() {
        return None;
    }
    let tokens = output
        .as_i64()
        .unwrap_or_else(|| panic!("{}: output is not an integer", vectors::name(case)));
    Some(i32::try_from(tokens).unwrap())
}

/// Checks one `tokens.json` case against its TS output; returns its kind (the name prefix).
fn check_case(case: &Map<String, Value>) -> &'static str {
    let name = vectors::name(case);
    let kind = KINDS
        .into_iter()
        .find(|kind| name == *kind || name.strip_prefix(kind).is_some_and(|r| r.starts_with(':')))
        .unwrap_or_else(|| panic!("{name}: unknown kind"));
    let input = case.get("input");
    match kind {
        "bytes" => {
            let bytes = input.and_then(Value::as_u64).expect("bytes input");
            assert_eq!(Some(estimate_tokens(bytes)), expected(case), "{name}");
        }
        "utf8" => {
            let text = input.and_then(Value::as_str).expect("utf8 input");
            let bytes = case["output"].as_u64().expect("utf8 output");
            assert_eq!(u64::try_from(text.len()).unwrap(), bytes, "{name}");
        }
        "input" => assert_eq!(estimate_input_tokens(input), expected(case), "{name}"),
        "output" => assert_eq!(estimate_output_tokens(input), expected(case), "{name}"),
        "divisor" => {
            // The divisor TS pins, applied to every byte count up to 2000, in float arithmetic.
            let divisor = case["output"].as_f64().expect("divisor");
            for bytes in 0_u32..=2000 {
                #[expect(clippy::cast_possible_truncation, reason = "at most 572 tokens")]
                let tokens = (f64::from(bytes) / divisor).ceil() as i32;
                assert_eq!(estimate_tokens(u64::from(bytes)), tokens, "{bytes} bytes");
            }
        }
        _ => unreachable!("{name}: kind {kind:?} has no check"),
    }
    kind
}

#[test]
fn vectors_match_ts() {
    let seen: BTreeSet<&str> = vectors::cases("tokens").iter().map(check_case).collect();
    assert_eq!(seen, BTreeSet::from(KINDS), "every kind has a case");
    // TS maps absent arguments and an absent response to undefined, as null (a vector case).
    assert_eq!(estimate_input_tokens(None), None);
    assert_eq!(estimate_output_tokens(None), None);
}

#[test]
fn estimate_tokens_never_overflows() {
    assert_eq!(estimate_tokens(u64::MAX), i32::MAX);
    assert_eq!(estimate_tokens(0), 0);
}

/// A content block: text, resource, image, other JSON, or a block with a wrong-typed field.
fn block() -> impl Strategy<Value = Value> + Clone {
    prop_oneof![
        js_string(40).prop_map(|t| json!({"type": "text", "text": t})),
        js_string(40)
            .prop_map(|t| json!({"type": "resource", "resource": {"uri": "u", "text": t}})),
        Just(json!({"type": "image", "data": "QUJD", "mimeType": "image/png"})),
        Just(json!({"type": "resource", "resource": {"uri": "u", "blob": "QUJD"}})),
        json_value(1, 3),
        (json_value(1, 2), json_value(1, 2))
            .prop_map(|(t, r)| json!({"type": "text", "text": t, "resource": r})),
        json_value(1, 2).prop_map(|r| json!({"type": "resource", "resource": r})),
    ]
}

/// A tool result: `content` blocks (possibly none), an optional `structuredContent`, extra keys.
fn tool_result() -> impl Strategy<Value = Value> + Clone {
    (
        proptest::collection::vec(block(), 0..6),
        proptest::option::of(json_value(2, 3)),
        proptest::collection::vec((object_key(), json_value(1, 2)), 0..3),
    )
        .prop_map(|(content, structured, extra)| {
            let mut result = Map::new();
            for (key, value) in extra {
                result.insert(key, value);
            }
            result.insert("content".to_owned(), Value::Array(content));
            if let Some(structured) = structured {
                result.insert("structuredContent".to_owned(), structured);
            }
            Value::Object(result)
        })
}

/// What the oracle gives as an `Option<i32>`: `undefined` is `None`.
fn ts_tokens(func: &str, value: &Value) -> Option<i64> {
    oracle::call(func, std::slice::from_ref(value))
        .unwrap()
        .map(|v| v.as_i64().expect("an integer"))
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn input_tokens_match_ts(
        entries in proptest::collection::vec((object_key(), json_value(5, 6)), 0..6),
    ) {
        let value = Value::Object(entries.into_iter().collect());
        let ts = ts_tokens("tokenEstimate.estimateInputTokens", &value);
        prop_assert_eq!(estimate_input_tokens(Some(&value)).map(i64::from), ts);
    }

    #[test]
    fn output_tokens_match_ts(value in tool_result()) {
        let ts = ts_tokens("tokenEstimate.estimateOutputTokens", &value);
        prop_assert_eq!(estimate_output_tokens(Some(&value)).map(i64::from), ts);
    }

    #[test]
    fn any_response_matches_ts(value in json_value(4, 5)) {
        let ts = ts_tokens("tokenEstimate.estimateOutputTokens", &value);
        prop_assert_eq!(estimate_output_tokens(Some(&value)).map(i64::from), ts);
    }

    #[test]
    fn estimate_tokens_matches_ts(bytes in prop_oneof![0_u64..100_000, 0_u64..(1 << 53)]) {
        let ts = oracle::call("tokenEstimate.estimateTokens", &[json!(bytes)]).unwrap();
        prop_assert_eq!(Some(json!(estimate_tokens(bytes))), ts);
    }
}

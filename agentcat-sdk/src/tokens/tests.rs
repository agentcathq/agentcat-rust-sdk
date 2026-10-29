//! `tokens`: the cases of `tests/vectors/tokens.json` and `tests/vectors/tokens_cases.json`, and
//! property tests of the bound and of what each estimate counts.
use std::collections::BTreeSet;

use proptest::prelude::*;
use serde_json::{Map, Value, json};

use super::{estimate_input_tokens, estimate_output_tokens, estimate_tokens};
use crate::js;
use crate::test_support::strategies::{js_string, json_value, object_key};
use crate::test_support::{config, vectors};

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
            assert_eq!(divisor, 3.5, "TS pins the divisor 3.5");
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

/// The cases of `tokens_cases.json`: random arguments (`input:`), random tool results and other
/// responses (`output:`), and random byte counts (`bytes:`).
#[test]
fn estimates_match_tokens_cases() {
    let seen: BTreeSet<&str> = vectors::cases("tokens_cases")
        .iter()
        .map(check_case)
        .collect();
    assert_eq!(
        seen,
        BTreeSet::from(["bytes", "input", "output"]),
        "tokens_cases.json has cases of each kind"
    );
}

/// The estimate of the byte length of the `stringify` text of `value`.
fn stringify_tokens(value: &Value) -> i32 {
    estimate_tokens(u64::try_from(js::stringify(value).len()).unwrap())
}

/// The UTF-8 bytes of the text a content block counts, from the rule: the `text` string of a
/// `text` block and the `resource.text` string of a `resource` block; 0 for any other block.
fn counted_bytes(block: &Value) -> u64 {
    let text = match block["type"].as_str() {
        Some("text") => &block["text"],
        Some("resource") => &block["resource"]["text"],
        _ => &Value::Null,
    };
    text.as_str()
        .map_or(0, |text| u64::try_from(text.len()).unwrap())
}

/// The output estimate from the rule: a response without a `content` array counts its JSON; an
/// empty `content` counts the JSON of `structuredContent`, or 0 without one; any other `content`
/// counts the text of its blocks, rounded once.
fn expected_output(response: &Value) -> i32 {
    match response.get("content").and_then(Value::as_array) {
        None => stringify_tokens(response),
        Some(content) if content.is_empty() => match response.get("structuredContent") {
            None | Some(Value::Null) => 0,
            Some(structured) => stringify_tokens(structured),
        },
        Some(content) => estimate_tokens(content.iter().map(counted_bytes).sum()),
    }
}

proptest! {
    #![proptest_config(config())]

    /// The estimate is the fewest tokens whose 3.5 bytes each cover the bytes (`7 * t >= 2 * b`)
    /// up to `i32::MAX`, and it never falls as the bytes grow.
    #[test]
    fn estimate_tokens_is_the_fewest_tokens_of_3_5_bytes_that_cover_the_bytes(
        bytes in prop_oneof![0_u64..100_000, 0_u64..(1 << 53)],
        more in 0_u64..1_000,
    ) {
        let tokens = estimate_tokens(bytes);
        let wide = |n: i32| 7 * u128::try_from(n).unwrap();
        prop_assert!(tokens == i32::MAX || wide(tokens) >= 2 * u128::from(bytes));
        if tokens > 0 {
            prop_assert!(wide(tokens - 1) < 2 * u128::from(bytes));
        }
        prop_assert!(estimate_tokens(bytes + more) >= tokens);
    }

    /// The input estimate counts the bytes of the arguments' `stringify` text.
    #[test]
    fn input_tokens_count_the_stringify_bytes(
        entries in proptest::collection::vec((object_key(), json_value(5, 6)), 0..6),
    ) {
        let value = Value::Object(entries.into_iter().collect());
        prop_assert_eq!(estimate_input_tokens(Some(&value)), Some(stringify_tokens(&value)));
    }

    /// The output estimate of a tool result counts the text of its text and resource blocks,
    /// or its structured content when it has no block.
    #[test]
    fn output_tokens_count_the_block_texts(value in tool_result()) {
        prop_assert_eq!(estimate_output_tokens(Some(&value)), Some(expected_output(&value)));
    }

    /// The output estimate of any other response counts its JSON when it has no `content` array.
    #[test]
    fn output_tokens_of_any_response_follow_the_same_rule(value in json_value(4, 5)) {
        let expected = (!value.is_null()).then(|| expected_output(&value));
        prop_assert_eq!(estimate_output_tokens(Some(&value)), expected);
    }
}

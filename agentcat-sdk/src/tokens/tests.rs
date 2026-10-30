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

/// The UTF-8 length of `text`.
fn utf8_len(text: &str) -> u64 {
    u64::try_from(text.len()).unwrap()
}

/// Any [`json_value`] up to 2 levels deep that `keep` holds for.
fn json_but(keep: fn(&Value) -> bool) -> impl Strategy<Value = Value> + Clone {
    json_value(1, 2).prop_filter("a value of the wrong kind", keep)
}

/// A content block and the UTF-8 bytes it counts, known from how the block is built: the `text`
/// string of a `text` block and the `resource.text` string of a `resource` block. A `text` key
/// elsewhere in the block, a text that is not a string, a resource that is not an object, a block
/// of another type or of a type that is not a string, a block without a type, and a block that is
/// not an object count 0.
fn counted_block() -> impl Strategy<Value = (Value, u64)> {
    let not_string: fn(&Value) -> bool = |value| !value.is_string();
    let not_object: fn(&Value) -> bool = |value| !value.is_object();
    let other_type = prop::sample::select(vec!["image", "audio", "resource_link", "Text", ""]);
    prop_oneof![
        js_string(40).prop_map(|t| (json!({"type": "text", "text": t}), utf8_len(&t))),
        (js_string(40), js_string(8)).prop_map(|(t, other)| {
            let block = json!({"type": "text", "text": t, "resource": {"text": other}});
            (block, utf8_len(&t))
        }),
        js_string(40).prop_map(|t| {
            let block = json!({"type": "resource", "resource": {"uri": "u", "text": t}});
            (block, utf8_len(&t))
        }),
        (js_string(40), js_string(8)).prop_map(|(t, other)| {
            let block =
                json!({"type": "resource", "text": other, "resource": {"uri": "u", "text": t}});
            (block, utf8_len(&t))
        }),
        Just((
            json!({"type": "image", "data": "QUJD", "mimeType": "image/png"}),
            0
        )),
        Just((
            json!({"type": "resource", "resource": {"uri": "u", "blob": "QUJD"}}),
            0
        )),
        json_but(not_string).prop_map(|t| (json!({"type": "text", "text": t}), 0)),
        json_but(not_string).prop_map(|t| (
            json!({"type": "resource", "resource": {"uri": "u", "text": t}}),
            0
        )),
        json_but(not_object).prop_map(|r| (json!({"type": "resource", "resource": r}), 0)),
        (other_type, js_string(16)).prop_map(|(kind, t)| {
            (json!({"type": kind, "text": t, "resource": {"text": t}}), 0)
        }),
        (json_but(not_string), js_string(16)).prop_map(|(kind, t)| {
            (json!({"type": kind, "text": t, "resource": {"text": t}}), 0)
        }),
        js_string(16).prop_map(|t| (json!({"text": t, "resource": {"text": t}}), 0)),
        json_but(not_object).prop_map(|block| (block, 0)),
    ]
}

/// A tool result without blocks: an empty `content`, an optional `structuredContent` (maybe
/// `null`), and other keys.
fn blockless_result() -> impl Strategy<Value = Value> {
    (
        proptest::option::of(json_value(2, 3)),
        proptest::collection::vec((object_key(), json_value(1, 2)), 0..3),
    )
        .prop_map(|(structured, extra)| {
            let mut result: Map<String, Value> = extra.into_iter().collect();
            result.insert("content".to_owned(), json!([]));
            if let Some(structured) = structured {
                result.insert("structuredContent".to_owned(), structured);
            }
            Value::Object(result)
        })
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

    /// The input estimate is the estimate of the byte length of the arguments' `stringify` text.
    /// This is only a composition check of `stringify` and [`estimate_tokens`]; the frozen cases
    /// of `tokens.json` and `tokens_cases.json` pin the values.
    #[test]
    fn input_tokens_count_the_stringify_bytes(
        entries in proptest::collection::vec((object_key(), json_value(5, 6)), 0..6),
    ) {
        let value = Value::Object(entries.into_iter().collect());
        prop_assert_eq!(estimate_input_tokens(Some(&value)), Some(stringify_tokens(&value)));
    }

    /// The output estimate of a tool result with blocks is `ceil(bytes / 3.5)` of the bytes its
    /// blocks count (see [`counted_block`]), summed before the one rounding; `structuredContent`
    /// and the other keys do not count.
    #[test]
    fn output_tokens_count_the_block_texts(
        blocks in proptest::collection::vec(counted_block(), 1..6),
        structured in proptest::option::of(json_value(2, 3)),
        extra in proptest::collection::vec((object_key(), json_value(1, 2)), 0..3),
    ) {
        let bytes: u64 = blocks.iter().map(|(_, bytes)| bytes).sum();
        let mut result: Map<String, Value> = extra.into_iter().collect();
        let content = blocks.into_iter().map(|(block, _)| block).collect();
        result.insert("content".to_owned(), Value::Array(content));
        if let Some(structured) = structured {
            result.insert("structuredContent".to_owned(), structured);
        }
        let tokens = i32::try_from((2 * bytes).div_ceil(7)).unwrap();
        prop_assert_eq!(estimate_output_tokens(Some(&Value::Object(result))), Some(tokens));
    }

    /// The output estimate of a response without blocks: without a `content` array it counts the
    /// `stringify` text of the response; with an empty one, that of `structuredContent`, or 0
    /// without one; `null` has no estimate. This is only a composition check of `stringify` and
    /// [`estimate_tokens`]; the frozen cases of `tokens_cases.json` pin the values.
    #[test]
    fn output_tokens_without_blocks_count_the_stringify_bytes(
        value in prop_oneof![blockless_result(), json_value(4, 5)],
    ) {
        let content = value.get("content").and_then(Value::as_array);
        prop_assume!(content.is_none_or(Vec::is_empty));
        let expected = match (content, value.get("structuredContent")) {
            _ if value.is_null() => None,
            (None, _) => Some(stringify_tokens(&value)),
            (Some(_), None | Some(Value::Null)) => Some(0),
            (Some(_), Some(structured)) => Some(stringify_tokens(structured)),
        };
        prop_assert_eq!(estimate_output_tokens(Some(&value)), expected);
    }
}

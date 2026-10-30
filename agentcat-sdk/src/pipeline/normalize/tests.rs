//! `normalize`: the cases of `tests/vectors/normalize.json` and
//! `tests/vectors/normalize_cases.json`, and property tests of the limits.
use proptest::prelude::*;
use serde_json::{Map, Value};

use super::{MAX_BREADTH, MAX_DEPTH, MAX_STRING_LENGTH, normalize};
use crate::test_support::json::index_keys_first;
use crate::test_support::rules::check_cut;
use crate::test_support::strategies::json_value;
use crate::test_support::{assert_js_eq, config_capped, vectors};

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

/// The `proto:` cases pin that a `__proto__` key is an ordinary key: it keeps its normalized
/// value and counts toward the breadth. Each has a `-twin` case that spells the key `_proto_`.
#[test]
fn vectors_match_ts() {
    let cases = vectors::cases("normalize");
    assert!(cases.len() > 60, "the file has its cases");
    for case in &cases {
        let name = vectors::name(case);
        let (depth, breadth, max_string) = limits_of(case);
        let actual = normalize(&input_of(case), depth, breadth, max_string);
        assert_js_eq(&actual, &case["output"], name);
    }
}

/// The cases of `normalize_cases.json`: random values under small random limits
/// (`small-limits:`) and under the default limits (`default-limits:`).
#[test]
fn normalize_matches_normalize_cases() {
    let cases = vectors::cases("normalize_cases");
    assert!(cases.len() >= 100, "normalize_cases.json has its cases");
    for case in &cases {
        let name = vectors::name(case);
        let (depth, breadth, max_string) = limits_of(case);
        let actual = normalize(&input_of(case), depth, breadth, max_string);
        assert_js_eq(&actual, &case["output"], name);
    }
}

/// The value that replaces the items past the breadth limit.
const MAX_PROPERTIES: &str = "[MaxProperties ~]";

/// Checks that `output` is `input` normalized with `depth` levels left, `breadth` items per
/// container and strings of at most `max_string` UTF-16 units.
fn check_normalized(
    input: &Value,
    output: &Value,
    depth: usize,
    breadth: usize,
    max_string: usize,
) -> Result<(), TestCaseError> {
    match (input, output) {
        (Value::String(text), Value::String(out)) if text.encode_utf16().count() > max_string => {
            let kept = out.strip_suffix("...").expect("a cut string ends with ...");
            check_cut(text, kept, max_string)?;
        }
        (Value::Array(_), _) if depth == 0 => prop_assert_eq!(output, "[Array]"),
        (Value::Object(_), _) if depth == 0 => prop_assert_eq!(output, "[Object]"),
        (Value::Array(items), Value::Array(out)) => {
            let kept = items.len().min(breadth);
            let cut = items.len() > breadth;
            prop_assert_eq!(out.len(), kept + usize::from(cut));
            for (item, out) in items.iter().zip(out).take(kept) {
                check_normalized(item, out, depth - 1, breadth, max_string)?;
            }
            if cut {
                prop_assert_eq!(&out[kept], MAX_PROPERTIES);
            }
        }
        (Value::Object(map), Value::Object(out)) => {
            let cut = map.len() > breadth;
            let mut keys: Vec<&str> = index_keys_first(map.keys().map(String::as_str));
            keys.truncate(breadth);
            for key in &keys {
                if cut && *key == "..." {
                    prop_assert_eq!(&out[*key], MAX_PROPERTIES);
                } else {
                    check_normalized(&map[*key], &out[*key], depth - 1, breadth, max_string)?;
                }
            }
            if cut && !keys.contains(&"...") {
                prop_assert_eq!(&out["..."], MAX_PROPERTIES);
                keys.push("...");
            }
            let out_keys: Vec<&str> = out.keys().map(String::as_str).collect();
            prop_assert_eq!(out_keys, keys);
        }
        _ => prop_assert_eq!(output, input),
    }
    Ok(())
}

proptest! {
    #![proptest_config(config_capped(256))]

    /// Each container past the depth limit is `[Array]` or `[Object]`; each one within it keeps
    /// its first items (an object's keys in the order of [`index_keys_first`]) up to the breadth
    /// limit and marks the rest; each string past the length limit keeps its longest prefix
    /// within the limit and ends with `...`.
    #[test]
    fn normalize_cuts_each_level_at_the_limits(
        input in json_value(6, 6),
        depth in 0usize..6,
        breadth in 0usize..6,
        max_string in 0usize..20,
    ) {
        let output = normalize(&input, depth, breadth, max_string);
        check_normalized(&input, &output, depth, breadth, max_string)?;
    }

    /// A value within the default limits comes back equal, in the key order of
    /// [`index_keys_first`].
    #[test]
    fn values_within_the_default_limits_come_back_unchanged(input in json_value(9, 6)) {
        let output = normalize(&input, MAX_DEPTH, MAX_BREADTH, MAX_STRING_LENGTH);
        check_normalized(&input, &output, MAX_DEPTH, MAX_BREADTH, MAX_STRING_LENGTH)?;
        prop_assert_eq!(&output, &input);
    }
}

//! `js` against JavaScript: the TS-generated vectors of `tests/vectors/js.json`, and property
//! tests that run the same inputs through the JS oracle.
use std::collections::BTreeSet;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use proptest::prelude::*;
use serde_json::{Map, Value, json};

use super::{
    cut_utf16, iso_millis, number_to_string, own_entries, parse_iso_millis, stringify,
    to_js_string, to_js_value, trim, type_of, utf16_len, write_stringify,
};
use crate::test_support::strategies::{js_string, json_value, object_key};
use crate::test_support::{assert_js_eq, config, oracle, vectors};

/// The case name prefixes of `js.json`.
const KINDS: [&str; 7] = [
    "stringify",
    "stringify-text",
    "cut",
    "trim",
    "utf16-len",
    "string",
    "typeof",
];

/// The `field` string of `case`.
fn text<'a>(case: &'a Map<String, Value>, field: &str) -> &'a str {
    case.get(field)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{}: no string {field:?}", vectors::name(case)))
}

/// The `value` of a `{value?}` input: `None` when the key is absent (JS `undefined`).
fn value_of(case: &Map<String, Value>) -> Option<&Value> {
    case["input"]
        .as_object()
        .unwrap_or_else(|| panic!("{}: input is not an object", vectors::name(case)))
        .get("value")
}

/// Checks one `js.json` case against its TS output; returns its kind (the name prefix).
fn check_case(case: &Map<String, Value>) -> &'static str {
    let name = vectors::name(case);
    let kind = KINDS
        .into_iter()
        .find(|kind| {
            name.strip_prefix(kind)
                .is_some_and(|rest| rest.starts_with(':'))
        })
        .unwrap_or_else(|| panic!("{name}: unknown kind"));
    let output = &case["output"];
    match kind {
        "stringify" => assert_eq!(stringify(&case["input"]), text(case, "output"), "{name}"),
        "stringify-text" => {
            let value: Value = serde_json::from_str(text(case, "input_text"))
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(stringify(&value), text(case, "output"), "{name}");
        }
        "cut" => {
            let (Some(s), Some(units)) = (case["input"][0].as_str(), case["input"][1].as_u64())
            else {
                panic!("{name}: input is not [text, units]");
            };
            let units = usize::try_from(units).unwrap();
            assert_js_eq(&json!(cut_utf16(s, units)), output, name);
        }
        "trim" => assert_eq!(trim(text(case, "input")), text(case, "output"), "{name}"),
        "utf16-len" => {
            let len = u64::try_from(utf16_len(text(case, "input"))).unwrap();
            assert_eq!(Some(len), output.as_u64(), "{name}");
        }
        "string" => assert_eq!(to_js_string(value_of(case)), text(case, "output"), "{name}"),
        "typeof" => assert_eq!(type_of(value_of(case)), text(case, "output"), "{name}"),
        _ => unreachable!("{name}: kind {kind:?} has no check"),
    }
    kind
}

/// Whether `name` is an `iso:` or `parse-iso:` case, which the two time tests check.
fn is_time_case(name: &str) -> bool {
    name.starts_with("iso:") || name.starts_with("parse-iso:")
}

/// The time `ms` milliseconds after the epoch (before it when negative).
fn at_millis(ms: i64) -> SystemTime {
    let magnitude = Duration::from_millis(ms.unsigned_abs());
    if ms < 0 {
        UNIX_EPOCH - magnitude
    } else {
        UNIX_EPOCH + magnitude
    }
}

/// `iso_millis` writes what `Date.prototype.toISOString` writes, for epoch 0, 7 ms, 999 ms and
/// the other `iso:` cases of `js.json`.
#[test]
fn iso_millis_matches_to_iso_string() {
    let mut seen = 0;
    for case in vectors::cases("js") {
        let name = vectors::name(&case);
        if !name.starts_with("iso:") {
            continue;
        }
        let ms = case["input"].as_i64().unwrap();
        assert_eq!(iso_millis(at_millis(ms)), text(&case, "output"), "{name}");
        seen += 1;
    }
    assert!(seen >= 3, "js.json must have iso: cases");
    // Sub-millisecond parts are cut, not rounded.
    assert_eq!(
        iso_millis(UNIX_EPOCH + Duration::new(1, 999_999_999)),
        "1970-01-01T00:00:01.999Z"
    );
}

/// A time outside chrono's range gives the epoch text, not a panic; a time before the epoch with
/// a fraction cuts the milliseconds as `toISOString` does.
///
/// The far times exist only where `SystemTime` can hold them: on Windows it counts 100 ns ticks
/// in a `u64`, so no `SystemTime` lies outside chrono's range and the far cases are skipped.
#[test]
fn iso_millis_out_of_range_does_not_panic() {
    let epoch = "1970-01-01T00:00:00.000Z";
    let far = Duration::from_secs(u64::MAX / 4);
    if let Some(late) = UNIX_EPOCH.checked_add(far) {
        assert_eq!(iso_millis(late), epoch);
    }
    if let Some(early) = UNIX_EPOCH.checked_sub(far) {
        assert_eq!(iso_millis(early), epoch);
    }
    assert_eq!(
        iso_millis(UNIX_EPOCH - Duration::from_micros(1500)),
        "1969-12-31T23:59:59.998Z"
    );
}

/// `parse_iso_millis` reads the `parse-iso:` texts of `js.json` to the instant `Date.parse`
/// gives, and the `iso:` outputs back to their instants; text that is not a date gives `None`.
#[test]
fn parse_iso_millis_reads_both_forms() {
    let mut seen = 0;
    for case in vectors::cases("js") {
        let name = vectors::name(&case);
        if name.starts_with("parse-iso:") {
            let ms = case["output"].as_i64().unwrap();
            assert_eq!(
                parse_iso_millis(text(&case, "input_text")),
                Some(at_millis(ms)),
                "{name}"
            );
            seen += 1;
        } else if name.starts_with("iso:") {
            let ms = case["input"].as_i64().unwrap();
            assert_eq!(
                parse_iso_millis(text(&case, "output")),
                Some(at_millis(ms)),
                "{name}"
            );
            seen += 1;
        }
    }
    assert!(seen >= 8, "js.json must have iso: and parse-iso: cases");
    assert_eq!(parse_iso_millis("yesterday"), None);
    assert_eq!(parse_iso_millis(""), None);
}

#[test]
fn vectors_match_ts() {
    let seen: BTreeSet<&str> = vectors::cases("js")
        .iter()
        .filter(|case| !is_time_case(vectors::name(case)))
        .map(check_case)
        .collect();
    assert_eq!(
        seen,
        BTreeSet::from(KINDS),
        "js.json must have cases of every kind"
    );
}

/// `-0` is `"0"` in JavaScript, as `stringify:negative-zero` and `string:-0` show; `ryu_js`
/// writes it so, and this pins it for `number_to_string` itself.
#[test]
fn negative_zero_matches_js() {
    let zero = serde_json::Number::from_f64(-0.0).unwrap();
    let js = oracle::call("oracle.String", &[json!({"value": -0.0})]).unwrap();
    assert_eq!(Some(json!(number_to_string(&zero))), js);
}

/// C40: `to_js_value` is the `JSON.stringify` text read back, so `serde_json` writes it as
/// JavaScript does: `1.0` as `1`, `-0.0` as `0`, an integer above 2^53 rounded to the nearest
/// `f64`, and index-like keys first in ascending order, at every depth.
#[test]
fn to_js_value_writes_stringify_text() {
    let value = json!({
        "10": 1.0,
        "2": [-0.0, 9_007_199_254_740_993_u64, {"b": 2.0, "1": 0.5}],
        "text": "a\u{2028}b",
    });
    let read_back = to_js_value(&value);
    assert_eq!(
        serde_json::to_string(&read_back).unwrap(),
        stringify(&value)
    );
    assert_eq!(
        stringify(&value),
        "{\"2\":[0,9007199254740992,{\"1\":0.5,\"b\":2}],\"10\":1,\"text\":\"a\u{2028}b\"}"
    );
}

/// A value nested deeper than `serde_json` reads (128 levels) cannot be read back, so
/// `to_js_value` gives it unchanged.
#[test]
fn to_js_value_past_the_nesting_limit_is_a_copy() {
    let mut value = json!(1.0);
    for _ in 0..200 {
        value = json!([value]);
    }
    assert_eq!(to_js_value(&value), value);
}

/// RD-27 (owner ruling R23): JavaScript's `String()` throws for an object with an own
/// `toString` key, alone or at any depth inside an array; `to_js_string` writes it as
/// JavaScript writes the same object without that key.
#[test]
fn to_js_string_keeps_objects_with_a_to_string_key_rd27() {
    let cases = [
        (json!({"toString": 1}), json!({"other": 1})),
        (json!([[{"toString": null}]]), json!([[{"other": null}]])),
        (
            json!([1, [{"toString": "f"}, null, "a"]]),
            json!([1, [{"other": "f"}, null, "a"]]),
        ),
    ];
    for (value, renamed) in &cases {
        let err = oracle::call("oracle.String", &[json!({"value": value})]).unwrap_err();
        assert!(
            err.message
                .contains("Cannot convert object to primitive value"),
            "{value}: {err}"
        );
        let js = oracle::call("oracle.String", &[json!({"value": renamed})]).unwrap();
        assert_eq!(Some(json!(to_js_string(Some(value)))), js, "{value}");
    }
    assert_eq!(to_js_string(Some(&cases[0].0)), "[object Object]");
    assert_eq!(to_js_string(Some(&cases[1].0)), "[object Object]");
}

/// The plain JSON text of a number: a safe integer, or at most 15 digits times 10^e with
/// |e| <= 22, written with or without a decimal point and an exponent (Ruling P3). `serde_json`
/// without `float_roundtrip` (R22) parses such a text to the `f64` `JSON.parse` gives: an exact
/// significand times or divided by an exact power of ten, rounded once.
fn plain_number_text() -> impl Strategy<Value = String> + Clone {
    const SAFE: i64 = (1 << 53) - 1;
    let decimal = (0u64..1_000_000_000_000_000, -22i32..=22, 0u8..4).prop_map(|(m, e, form)| {
        let digits = m.to_string();
        let len = i32::try_from(digits.len()).unwrap();
        match form {
            // 12.345: the point leaves -e fraction digits.
            0 if e < 0 && -e < len => {
                let (int, frac) = digits.split_at(usize::try_from(len + e).unwrap());
                format!("{int}.{frac}")
            }
            // 0.0012345
            0 if e < 0 => format!(
                "0.{}{digits}",
                "0".repeat(usize::try_from(-e - len).unwrap())
            ),
            // 1.2345e3: one integer digit, so the exponent grows by the fraction digits.
            1 if len > 1 => format!("{}.{}e{}", &digits[..1], &digits[1..], e + len - 1),
            2 => format!("{digits}E{e:+}"),
            _ => format!("{digits}e{e}"),
        }
    });
    prop_oneof![
        (-SAFE..=SAFE).prop_map(|n| n.to_string()),
        (any::<bool>(), decimal).prop_map(|(negative, text)| if negative {
            format!("-{text}")
        } else {
            text
        }),
    ]
}

/// A JSON text of `json_value`'s shape with [`plain_number_text`] numbers, whose objects list
/// their keys in shuffled order and often repeat one (a repeated key keeps its first position
/// and takes its last value, in JavaScript and in `serde_json`'s `preserve_order` map).
fn json_text() -> impl Strategy<Value = String> + Clone {
    let leaf = prop_oneof![
        Just("null".to_owned()),
        any::<bool>().prop_map(|b| b.to_string()),
        plain_number_text(),
        js_string(16).prop_map(|s| Value::String(s).to_string()),
    ];
    leaf.prop_recursive(5, 64, 8, |inner| {
        let key = prop_oneof![
            2 => object_key(),
            1 => prop::sample::select(vec!["a", "b", "0", "1", "10"]).prop_map(str::to_owned),
        ];
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..=8)
                .prop_map(|items| format!("[{}]", items.join(","))),
            prop::collection::vec((key, inner), 0..=8)
                .prop_shuffle()
                .prop_map(|entries| {
                    let fields: Vec<String> = entries
                        .into_iter()
                        .map(|(key, value)| format!("{}:{value}", Value::String(key)))
                        .collect();
                    format!("{{{}}}", fields.join(","))
                }),
        ]
    })
}

/// A string that often starts and ends with JavaScript whitespace or its neighbors U+0085,
/// U+180E and U+200B, which `trim` keeps.
fn padded_string() -> impl Strategy<Value = String> + Clone {
    let edge = prop::collection::vec(
        prop::sample::select(vec![
            '\t', '\n', '\u{B}', '\u{C}', '\r', ' ', '\u{A0}', '\u{1680}', '\u{2000}', '\u{200A}',
            '\u{2028}', '\u{2029}', '\u{202F}', '\u{205F}', '\u{3000}', '\u{FEFF}', '\u{85}',
            '\u{180E}', '\u{200B}', 'x',
        ]),
        0..4,
    )
    .prop_map(String::from_iter);
    (edge.clone(), js_string(20), edge).prop_map(|(head, body, tail)| head + &body + &tail)
}

/// The `{value?}` argument of `oracle.String` and `oracle.typeof`.
fn value_arg(value: Option<&Value>) -> Value {
    value.map_or_else(|| json!({}), |value| json!({"value": value}))
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn stringify_matches_js(value in json_value(6, 8)) {
        let js = oracle::call("JSON.stringify", std::slice::from_ref(&value)).unwrap();
        prop_assert_eq!(Some(json!(stringify(&value))), js);
    }

    #[test]
    fn reformat_matches_js(text in json_text()) {
        let value: Value = serde_json::from_str(&text).unwrap();
        let js = oracle::call("oracle.reformat", &[json!(text)]).unwrap();
        prop_assert_eq!(Some(json!(stringify(&value))), js, "{}", text);
    }

    #[test]
    fn trim_matches_js(text in padded_string()) {
        let js = oracle::call("String.prototype.trim", &[json!(text)]).unwrap();
        prop_assert_eq!(Some(json!(trim(&text))), js);
    }

    #[test]
    fn cut_matches_js((text, units) in (js_string(30), 0..40usize)) {
        let js = oracle::call("String.prototype.slice", &[json!(text), json!(units)])
            .unwrap()
            .unwrap();
        assert_js_eq(&json!(cut_utf16(&text, units)), &js, &format!("{text:?} at {units}"));
    }

    #[test]
    fn utf16_len_matches_js(text in js_string(30)) {
        let js = oracle::call("String.length", &[json!(text)]).unwrap();
        prop_assert_eq!(Some(json!(utf16_len(&text))), js);
    }

    #[test]
    fn to_js_string_matches_js(value in proptest::option::of(json_value(3, 4))) {
        let js = oracle::call("oracle.String", &[value_arg(value.as_ref())]).unwrap();
        prop_assert_eq!(Some(json!(to_js_string(value.as_ref()))), js);
    }

    #[test]
    fn type_of_matches_js(value in proptest::option::of(json_value(2, 3))) {
        let js = oracle::call("oracle.typeof", &[value_arg(value.as_ref())]).unwrap();
        prop_assert_eq!(Some(json!(type_of(value.as_ref()))), js);
    }
}

/// `write_stringify` writes the text `stringify` returns, for every `stringify` case of
/// `js.json` (the TS text) and every `stringify-text` case.
#[test]
fn write_stringify_equals_stringify() {
    let mut seen = 0;
    for case in vectors::cases("js") {
        let name = vectors::name(&case);
        let value: Value = if name.starts_with("stringify-text:") {
            serde_json::from_str(text(&case, "input_text")).unwrap()
        } else if name.starts_with("stringify:") {
            case["input"].clone()
        } else {
            continue;
        };
        let mut written = String::new();
        write_stringify(&value, &mut written).unwrap();
        assert_eq!(written, stringify(&value), "{name}");
        assert_eq!(written, text(&case, "output"), "{name}");
        seen += 1;
    }
    assert!(seen >= 10, "js.json must have stringify cases");
}

/// The fast path of `own_entries` (no index-like key) keeps insertion order, and the index path
/// still puts the indices first in ascending order, then the other keys in insertion order.
#[test]
fn own_entries_fast_path_keeps_order() {
    let plain: Value =
        serde_json::from_str(r#"{"b":1,"a":2,"01":3,"-1":4,"4294967295":5}"#).unwrap();
    let keys: Vec<&str> = own_entries(plain.as_object().unwrap())
        .map(|(key, _)| key.as_str())
        .collect();
    assert_eq!(keys, ["b", "a", "01", "-1", "4294967295"]);
    let mixed: Value =
        serde_json::from_str(r#"{"b":1,"10":2,"a":3,"2":4,"4294967294":5}"#).unwrap();
    let keys: Vec<&str> = own_entries(mixed.as_object().unwrap())
        .map(|(key, _)| key.as_str())
        .collect();
    assert_eq!(keys, ["2", "10", "4294967294", "b", "a"]);
    assert_eq!(own_entries(&Map::new()).count(), 0);
}

/// No decision: this pins a Rust contract, `ExactSizeIterator` (its `size_hint` is exact and `len`
/// is the number of entries left), which `OwnEntries`, the iterator of C04's key order, implements.
/// It holds on both paths, before and while the iterator is consumed.
#[test]
fn own_entries_len_is_exact() {
    for text in [r#"{"b":1,"a":2,"c":3}"#, r#"{"b":1,"10":2,"2":3}"#] {
        let object: Value = serde_json::from_str(text).unwrap();
        let mut entries = own_entries(object.as_object().unwrap());
        for left in (0..=3).rev() {
            assert_eq!(entries.size_hint(), (left, Some(left)), "{text}");
            assert_eq!(entries.len(), left, "{text}");
            if left > 0 {
                assert!(entries.next().is_some(), "{text}");
            }
        }
        assert_eq!(entries.next(), None, "{text}");
    }
}

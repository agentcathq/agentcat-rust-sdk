//! `sanitize_event` against TS: the vectors of `tests/vectors/sanitize.json` and a property test
//! that runs the same events through the JS oracle.
use proptest::prelude::*;
use serde_json::{Map, Value, json};

use super::sanitize_event;
use crate::pipeline::TsEvent;
use crate::test_support::strategies::{js_string, ts_event};
use crate::test_support::{assert_js_eq, config_capped, markers, oracle, vectors};

/// Each RD-24 and RD-27 case and its twin, a case of the same file. RD-24: the same event with
/// `_proto_` for each `__proto__` key. RD-27: the same blocks with `{"a": 1}` for the `type`
/// that has an own `toString` key.
const TWINS: [(&str, &str); 7] = [
    ("proto:parameters", "proto:parameters-twin"),
    ("proto:parameters-string", "proto:parameters-string-twin"),
    ("proto:parameters-null", "proto:parameters-null-twin"),
    (
        "proto:parameters-array-value",
        "proto:parameters-array-value-twin",
    ),
    ("proto:parameters-nested", "proto:parameters-nested-twin"),
    ("proto:structured-content", "proto:structured-content-twin"),
    ("block:type-tostring-key", "block:type-object"),
];

/// The event a case holds: `input`, or `input_text` parsed in insertion order; `$repeat`
/// macros expanded.
fn input_of(case: &Map<String, Value>, context: &str) -> TsEvent {
    let value = match (case.get("input"), case.get("input_text")) {
        (Some(input), None) => input.clone(),
        (None, Some(Value::String(text))) => {
            markers::expand(serde_json::from_str(text).expect("input_text"))
        }
        _ => panic!("{context}: needs exactly one of input and input_text"),
    };
    match value {
        Value::Object(event) => event,
        other => panic!("{context}: the input is not an event: {other}"),
    }
}

/// `value` with each `from` key renamed to `to`, at any depth, at the same place in the order.
fn rename_key(value: &Value, from: &str, to: &str) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, item)| {
                    let key = if key == from { to } else { key };
                    (key.to_owned(), rename_key(item, from, to))
                })
                .collect(),
        ),
        Value::Array(items) => {
            Value::Array(items.iter().map(|i| rename_key(i, from, to)).collect())
        }
        other => other.clone(),
    }
}

/// `true` when `value` has a `__proto__` key anywhere.
fn has_proto_key(value: &Value) -> bool {
    match value {
        Value::Object(map) => map.contains_key("__proto__") || map.values().any(has_proto_key),
        Value::Array(items) => items.iter().any(has_proto_key),
        _ => false,
    }
}

/// The twin of the case `name` in [`TWINS`]: its name and the case.
fn twin_of<'a>(
    cases: &'a [Map<String, Value>],
    name: &str,
) -> (&'static str, &'a Map<String, Value>) {
    let (_, twin_name) = TWINS
        .iter()
        .find(|(case, _)| *case == name)
        .unwrap_or_else(|| panic!("{name} has a twin in TWINS"));
    let twin = cases
        .iter()
        .find(|case| vectors::name(case) == *twin_name)
        .unwrap_or_else(|| panic!("the twin {twin_name} of {name} is in the file"));
    (twin_name, twin)
}

#[test]
fn vectors_match_ts() {
    let cases = vectors::cases("sanitize");
    assert!(cases.len() >= 100, "sanitize.json has its cases");
    for case in &cases {
        let name = vectors::name(case);
        let context = format!("sanitize.json {name}");
        let input = input_of(case, &context);
        let actual = Value::Object(sanitize_event(&input));
        let deviation = case.get("deviation").and_then(Value::as_str);
        if deviation != Some("RD-27") {
            assert!(
                case.get("error").is_none(),
                "{context}: only the RD-27 case throws in TS"
            );
        }
        match deviation {
            None => assert_js_eq(&actual, &case["output"], &context),
            Some("RD-24") => check_rd24(&cases, name, &input, &actual, &context),
            Some("RD-25") => check_rd25(&actual, &case["output"], &context),
            Some("RD-27") => check_rd27(&cases, name, &input, &actual, case, &context),
            Some(other) => panic!("{context}: unknown deviation {other}"),
        }
    }
}

/// RD-24: TS drops a `__proto__` key from the walked parts (a prototype assignment), Rust keeps
/// it. The twin has `_proto_` for the key: Rust's event with the key renamed is TS's output for
/// the twin. TS's output for the case itself is the twin's without the key.
fn check_rd24(
    cases: &[Map<String, Value>],
    name: &str,
    input: &TsEvent,
    actual: &Value,
    context: &str,
) {
    let (twin_name, twin) = twin_of(cases, name);
    let renamed = rename_key(&Value::Object(input.clone()), "__proto__", "_proto_");
    assert_eq!(
        renamed,
        Value::Object(input_of(twin, twin_name)),
        "{context}: the twin {twin_name} has `_proto_` for `__proto__`"
    );
    assert!(
        has_proto_key(actual),
        "{context}: RD-24: Rust keeps the key"
    );
    assert_js_eq(
        &rename_key(actual, "__proto__", "_proto_"),
        &twin["output"],
        &format!("{context}: RD-24, against the TS output of {twin_name}"),
    );
}

/// RD-25: Rust keeps an array `response` an array; TS spreads it into an object with index keys.
/// The array holds TS's values in index order, and every other field is TS's.
fn check_rd25(actual: &Value, expected: &Value, context: &str) {
    let mut event = actual.as_object().expect("an event").clone();
    let Some(Value::Array(items)) = event.remove("response") else {
        panic!("{context}: RD-25: the response stays an array");
    };
    let mut ts = expected.as_object().expect("TS's event").clone();
    let Some(Value::Object(spread)) = ts.remove("response") else {
        panic!("{context}: TS spreads the response into an object");
    };
    let keys: Vec<String> = (0..items.len()).map(|i| i.to_string()).collect();
    assert_eq!(
        spread.keys().cloned().collect::<Vec<_>>(),
        keys,
        "{context}"
    );
    let values = Value::Array(spread.into_iter().map(|(_, value)| value).collect());
    assert_js_eq(&Value::Array(items), &values, context);
    assert_js_eq(&Value::Object(event), &Value::Object(ts), context);
}

/// RD-27: TS throws when it writes the message for a `type` with an own `toString` key. Rust
/// writes `[object Object]`: it gives TS's output for the twin, whose block has `{"a": 1}` for
/// the `type` and is the same elsewhere.
fn check_rd27(
    cases: &[Map<String, Value>],
    name: &str,
    input: &TsEvent,
    actual: &Value,
    case: &Map<String, Value>,
    context: &str,
) {
    assert_eq!(
        case.get("error"),
        Some(&json!(
            "TypeError: Cannot convert object to primitive value"
        )),
        "{context}: TS throws"
    );
    let (twin_name, twin) = twin_of(cases, name);
    let mut swapped = input.clone();
    let block = &mut swapped["response"]["content"][0]["type"];
    assert!(
        block.get("toString").is_some(),
        "{context}: the type has a toString key"
    );
    *block = json!({ "a": 1 });
    assert_eq!(
        Value::Object(swapped),
        Value::Object(input_of(twin, twin_name)),
        "{context}: the twin {twin_name} differs only in the type"
    );
    assert_js_eq(
        actual,
        &twin["output"],
        &format!("{context}: RD-27, against the TS output of {twin_name}"),
    );
}

/// Runs `event` through Rust and through TS (the oracle) and compares the results.
fn compare(event: &TsEvent) {
    let actual = Value::Object(sanitize_event(event));
    let ts = oracle::call(
        "sanitization.sanitizeEvent",
        &[Value::Object(event.clone())],
    )
    .unwrap()
    .expect("a reply");
    assert_js_eq(&actual, &ts, "sanitizeEvent");
}

/// A string of 10 230 to 10 262 characters from the base64 alphabet with line breaks, a repeated
/// seed of 1 to 64 characters, then 0 to 3 `=`, and sometimes one character that the pattern
/// rejects or a line break after the padding.
fn base64_run() -> impl Strategy<Value = String> + Clone {
    let alphabet: Vec<char> = ('A'..='Z')
        .chain('a'..='z')
        .chain('0'..='9')
        .chain(['+', '/', '\n', '\r'])
        .collect();
    (
        prop::collection::vec(prop::sample::select(alphabet), 1..=64),
        10_230usize..=10_262,
        0usize..=3,
        prop::sample::select(vec!["", "", "", "-", "_", " ", "é", "x", "=\n", "\n"]),
    )
        .prop_map(|(seed, len, padding, tail)| {
            let mut text: String = seed.iter().cycle().take(len).collect();
            text.push_str(&"=".repeat(padding));
            text.push_str(tail);
            text
        })
}

/// Strings of an event: mostly short, sometimes a [`base64_run`].
fn text() -> impl Strategy<Value = String> + Clone {
    prop_oneof![30 => js_string(8), 1 => base64_run()]
}

/// A content block that [`ts_event`] does not make: a `type` that is a number, `null`, a boolean,
/// absent, an array or an object without a `toString` key (never RD-27); a block that is not an
/// object; a `resource` of each shape.
fn odd_block() -> impl Strategy<Value = Value> {
    prop_oneof![
        prop::sample::select(vec![
            json!({"type": 5, "text": "t"}),
            json!({"type": 1.5}),
            json!({"type": null}),
            json!({"type": true}),
            json!({"text": "no type"}),
            json!({"type": ["a", null, [1]]}),
            json!({"type": {"a": 1}}),
            json!({"type": ""}),
            json!("text"),
            json!(3),
            Value::Null,
            json!([]),
            json!([{"type": "image"}]),
            json!({"type": "resource"}),
            json!({"type": "resource", "resource": "blob"}),
            json!({"type": "resource", "resource": [{"blob": "x"}]}),
            json!({"type": "resource", "resource": null}),
            json!({"type": "resource", "resource": {}}),
            json!({"type": "resource", "resource": {"blob": null}}),
            json!({"type": "resource", "resource": {"blob": 0, "text": "t"}}),
        ]),
        text().prop_map(|data| json!({"type": "resource", "resource": {"blob": data}})),
        text().prop_map(|data| json!({"type": "text", "text": data})),
    ]
}

/// A [`ts_event`] with up to 3 more content blocks ([`odd_block`]) in its `response`; its
/// `response` sometimes replaced by a string, a number or a boolean, and its `parameters` and
/// `structuredContent` sometimes by a [`base64_run`] or an array holding one.
fn sanitize_event_input() -> impl Strategy<Value = TsEvent> {
    (
        ts_event(text()),
        prop::collection::vec(odd_block(), 0..=3),
        0u8..10,
        prop::option::of(base64_run()),
    )
        .prop_map(|(mut event, blocks, replace, run)| {
            if let Some(Value::Object(response)) = event.get_mut("response")
                && let Some(Value::Array(content)) = response.get_mut("content")
            {
                content.extend(blocks);
            }
            match (replace, run) {
                (0, _) => {
                    event.insert("response".to_owned(), json!("done"));
                }
                (1, _) => {
                    event.insert("response".to_owned(), json!(7));
                }
                (2, Some(run)) => {
                    event.insert("parameters".to_owned(), Value::String(run));
                }
                (3, Some(run)) => {
                    event.insert("parameters".to_owned(), json!([run, "x", [null]]));
                }
                (4, Some(run)) => {
                    event.insert(
                        "response".to_owned(),
                        json!({"content": [], "structuredContent": {"k": [run]}}),
                    );
                }
                (5, Some(run)) => {
                    event.insert(
                        "response".to_owned(),
                        json!({"content": [], "structuredContent": run}),
                    );
                }
                _ => {}
            }
            event
        })
}

proptest! {
    #![proptest_config(config_capped(300))]

    #[test]
    fn sanitize_matches_ts(event in sanitize_event_input()) {
        compare(&event);
    }
}

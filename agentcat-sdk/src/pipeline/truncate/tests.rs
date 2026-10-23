//! `truncate_event` against TS: the vectors of `tests/vectors/truncate.json` and
//! `tests/vectors/truncate_size.json`, and property tests that run the same events through the JS
//! oracle.
use proptest::prelude::*;
use serde_json::{Map, Value, json};

use super::{TruncateError, truncate_event};
use crate::js;
use crate::pipeline::TsEvent;
use crate::test_support::strategies::{bmp_string, js_string, number, ts_event};
use crate::test_support::{assert_js_eq, config, config_capped, markers, oracle, vectors};

/// The vector sets: the field limits and the TS tests, then the size guard.
const SETS: [&str; 2] = ["truncate", "truncate_size"];

/// Each RD-26 and RD-27 case and its twin, where Rust gives the twin's TS result. RD-26: the same
/// field with an object that has no `length` key. RD-27: the same array with `{"a": 1}` in place
/// of the object with an own `toString` key.
const TWINS: [(&str, &str); 3] = [
    ("intent:length-object", "intent:object"),
    ("frames:object-length-10", "frames:object"),
    ("intent:array-with-tostring-key", "intent:array-with-object"),
];

/// The marker `normalize` writes at the `"..."` key of an object over its breadth.
const MAX_PROPERTIES: &str = "[MaxProperties ~]";

/// The largest event, in bytes, of the field-limit property test: far enough under the 102 400
/// bytes of the size guard that a cut character (RD-22) never meets it.
const FIELD_TEST_MAX_BYTES: usize = 90_000;

/// The UTF-8 size of `JSON.stringify(event)`.
fn size_of(event: &TsEvent) -> usize {
    js::stringify(&Value::Object(event.clone())).len()
}

/// Asserts that `actual` is TS's `expected` reply of `oracle.truncateEvent`: `{"value": event}`,
/// or `{"error": text}` for the text of the error TS throws.
fn assert_matches_ts(actual: &Result<TsEvent, TruncateError>, expected: &Value, context: &str) {
    match (actual, expected.get("value"), expected.get("error")) {
        (Ok(event), Some(value), None) => {
            assert_js_eq(&Value::Object(event.clone()), value, context);
        }
        (Err(error), None, Some(Value::String(message))) => {
            assert_eq!(error.to_string(), *message, "{context}");
        }
        _ => {
            let rust = match actual {
                Ok(event) => format!("an event of {} bytes", size_of(event)),
                Err(error) => format!("the error {error}"),
            };
            let ts = expected
                .get("error")
                .map_or_else(|| "an event".to_owned(), Value::to_string);
            panic!("{context}: Rust gives {rust}, TS gives {ts}");
        }
    }
}

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

/// The TS result a case holds, in the shape of an `oracle.truncateEvent` reply: `{"value":
/// output}`, or `{"error": error}` when TS throws.
fn reply_of(case: &Map<String, Value>, context: &str) -> Value {
    match (case.get("output"), case.get("error")) {
        (Some(output), None) => json!({ "value": output }),
        (None, Some(error)) => json!({ "error": error }),
        _ => panic!("{context}: needs exactly one of output and error"),
    }
}

#[test]
fn truncate_event_by_value_matches_vectors() {
    for set in SETS {
        let cases = vectors::cases(set);
        assert!(cases.len() >= 10, "{set}.json has its cases");
        for case in &cases {
            let name = vectors::name(case);
            let context = format!("{set}.json {name}");
            let input = &input_of(case, &context);
            let actual = truncate_event(input.clone());
            let expected = &reply_of(case, &context);
            match case.get("deviation").and_then(Value::as_str) {
                None => assert_matches_ts(&actual, expected, &context),
                Some("RD-25") => check_rd25(&actual, expected, &context),
                Some("RD-26") => check_rd26(&cases, name, &actual, expected, &context),
                Some("RD-27") => check_rd27(&cases, name, input, &actual, expected, &context),
                Some("RD-28") => check_rd28(name, input, &actual, expected, &context),
                Some(other) => panic!("{context}: unknown deviation {other}"),
            }
        }
    }
}

/// RD-25: Rust keeps an array `response` an array; TS spreads it into an object with index keys.
/// The array holds TS's values in index order, and every other field is TS's.
fn check_rd25(actual: &Result<TsEvent, TruncateError>, expected: &Value, context: &str) {
    let mut event = actual.clone().expect("RD-25: an event");
    let Some(Value::Array(items)) = event.remove("response") else {
        panic!("{context}: RD-25: the response stays an array");
    };
    let mut ts = expected["value"].as_object().expect("TS's event").clone();
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

/// RD-26: TS keeps an object whose `length` is at most the limit; Rust gives the error TS gives
/// for an object without `length` (the twin case's).
fn check_rd26(
    cases: &[Map<String, Value>],
    name: &str,
    actual: &Result<TsEvent, TruncateError>,
    expected: &Value,
    context: &str,
) {
    assert!(
        expected.get("value").is_some(),
        "{context}: TS keeps the object"
    );
    let (twin_name, twin) = twin_of(cases, name);
    let twin_error = twin
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{context}: the twin {twin_name} has TS's error"));
    let error = actual.as_ref().expect_err("RD-26: Rust gives the error");
    assert_eq!(
        error.to_string(),
        twin_error,
        "{context}: RD-26, against the TS error of {twin_name}"
    );
}

/// RD-27: TS throws on the object with an own `toString` key in the joined `userIntent` array.
/// Rust joins it as `[object Object]`: it gives TS's output for the twin, whose array has
/// `{"a": 1}` at that index and is the same elsewhere.
fn check_rd27(
    cases: &[Map<String, Value>],
    name: &str,
    input: &TsEvent,
    actual: &Result<TsEvent, TruncateError>,
    expected: &Value,
    context: &str,
) {
    assert!(expected.get("error").is_some(), "{context}: TS throws");
    let items = input["userIntent"].as_array().expect("an array userIntent");
    let at = items
        .iter()
        .position(|item| item.get("toString").is_some())
        .expect("an item with a toString key");
    assert_eq!(
        items.iter().filter(|item| item.is_object()).count(),
        1,
        "{context}: the object with the toString key is the only object of the array"
    );
    let (twin_name, twin) = twin_of(cases, name);
    let mut swapped = input.clone();
    swapped["userIntent"][at] = json!({ "a": 1 });
    assert_eq!(
        Value::Object(swapped),
        Value::Object(input_of(twin, twin_name)),
        "{context}: the twin {twin_name} differs only at index {at}"
    );
    let event = actual.as_ref().expect("RD-27: Rust truncates the event");
    assert_js_eq(
        &Value::Object(event.clone()),
        &reply_of(twin, twin_name)["value"],
        &format!("{context}: RD-27, against the TS output of {twin_name}"),
    );
}

/// RD-28: TS gives an `error` without `message` an own `message: undefined` property, and
/// `normalize` counts it toward the 100-key breadth before it skips it. With 100 entries, TS adds
/// the `"..."` marker; with 99 keys and a `"..."` key, TS writes the marker into that key. Rust
/// gives TS's event with the marker undone: the `"..."` key removed, or given back its value.
fn check_rd28(
    name: &str,
    input: &TsEvent,
    actual: &Result<TsEvent, TruncateError>,
    expected: &Value,
    context: &str,
) {
    let mut ts = expected["value"].as_object().expect("TS's event").clone();
    let Some(Value::Object(error)) = ts.get_mut("error") else {
        panic!("{context}: TS's error is an object");
    };
    assert_eq!(
        error.get("..."),
        Some(&json!(MAX_PROPERTIES)),
        "{context}: TS writes the marker"
    );
    match name {
        "error:100-keys-no-message" | "error:array-100" => {
            // A filter, not `Map::remove`: with `preserve_order`, `remove` can move the last key.
            *error = error
                .iter()
                .filter(|(key, _)| key.as_str() != "...")
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
        }
        "error:99-keys-plus-dots" => {
            let original = input["error"]["..."].clone();
            assert!(
                original.is_string(),
                "{context}: the input has a \"...\" key"
            );
            error.insert("...".to_owned(), original);
        }
        other => panic!("{context}: no RD-28 rule for {other}"),
    }
    let event = actual.as_ref().expect("RD-28: Rust truncates the event");
    assert_js_eq(
        &Value::Object(event.clone()),
        &Value::Object(ts),
        &format!("{context}: RD-28, against TS's event with the marker undone"),
    );
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

/// Runs `event` through Rust and through TS (the oracle) and compares the results.
fn compare(event: &TsEvent) {
    let actual = truncate_event(event.clone());
    let ts = oracle::call("oracle.truncateEvent", &[Value::Object(event.clone())])
        .unwrap()
        .expect("a reply");
    assert_matches_ts(&actual, &ts, "truncateEvent");
}

/// The object at `key` of `map`, made an empty object first when it is absent or not an object.
fn object_at<'a>(map: &'a mut Map<String, Value>, key: &str) -> &'a mut Map<String, Value> {
    let slot = map.entry(key).or_insert_with(|| json!({}));
    if !slot.is_object() {
        *slot = json!({});
    }
    slot.as_object_mut().expect("an object")
}

/// `block` appended to `response.content` of `event` (both made when absent).
fn push_content(event: &mut TsEvent, block: Value) {
    let content = object_at(event, "response")
        .entry("content")
        .or_insert_with(|| json!([]));
    if !content.is_array() {
        *content = json!([]);
    }
    content.as_array_mut().expect("an array").push(block);
}

/// A string of `limit - 4` to `limit + 4` UTF-16 units: `x`s, then up to 6 characters of
/// [`js_string`], so that a character outside the BMP can straddle the limit.
fn near(limit: usize) -> impl Strategy<Value = String> + Clone {
    (limit - 4..=limit + 4, js_string(6)).prop_map(|(units, tail)| {
        let mut text = "x".repeat(units - js::utf16_len(&tail));
        text.push_str(&tail);
        text
    })
}

/// An item of a joined `userIntent` array: a string, a number, `null`, a boolean, a nested
/// array, or `{"a": 1}` (no `toString` key: RD-27).
fn intent_item() -> impl Strategy<Value = Value> + Clone {
    prop_oneof![
        js_string(4).prop_map(Value::String),
        number().prop_map(Value::Number),
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        Just(json!([1, [null, "a"], {"a": 1}])),
        Just(json!({"a": 1})),
    ]
}

/// A field of an event set near its limit.
#[derive(Debug, Clone)]
enum Near {
    /// A top-level field and its value.
    Field(&'static str, Value),
    /// `error.message`.
    Message(String),
    /// One more `text` block in `response.content`.
    Content(String),
    /// `error.frames` with this many frames.
    Frames(usize),
}

impl Near {
    fn apply(self, event: &mut TsEvent) {
        match self {
            Self::Field(key, value) => {
                event.insert(key.to_owned(), value);
            }
            Self::Message(message) => {
                object_at(event, "error").insert("message".to_owned(), Value::String(message));
            }
            Self::Content(text) => push_content(event, json!({"type": "text", "text": text})),
            Self::Frames(count) => {
                let frames = (0..count)
                    .map(|i| json!({"filename": format!("src/f{i}.rs"), "function": "f", "lineno": i, "in_app": true}))
                    .collect();
                object_at(event, "error").insert("frames".to_owned(), Value::Array(frames));
            }
        }
    }
}

/// `userIntent` (2048), a server, client or resource field (256), `error.message` (2048) or a
/// content text block (32 768) near its limit; `error.frames` near 50 entries; a `userIntent`
/// array near 2048 items; a `userIntent` that is not a string.
fn near_limit() -> impl Strategy<Value = Near> {
    let metadata = prop::sample::select(vec![
        "resourceName",
        "serverName",
        "serverVersion",
        "clientName",
        "clientVersion",
    ]);
    let not_a_string =
        prop::sample::select(vec![json!(5), json!(false), json!({"a": 1}), Value::Null]);
    prop_oneof![
        near(2048).prop_map(|text| Near::Field("userIntent", Value::String(text))),
        (metadata, near(256)).prop_map(|(key, text)| Near::Field(key, Value::String(text))),
        near(2048).prop_map(Near::Message),
        near(32_768).prop_map(Near::Content),
        (45usize..=56).prop_map(Near::Frames),
        prop::collection::vec(intent_item(), 2046..=2050)
            .prop_map(|items| Near::Field("userIntent", Value::Array(items))),
        not_a_string.prop_map(|value| Near::Field("userIntent", value)),
    ]
}

/// A [`ts_event`] with up to 3 fields near their limits, under [`FIELD_TEST_MAX_BYTES`].
fn near_limit_event() -> impl Strategy<Value = TsEvent> {
    (
        ts_event(js_string(40)),
        prop::collection::vec(near_limit(), 0..=3),
    )
        .prop_map(|(mut event, changes)| {
            for change in changes {
                change.apply(&mut event);
            }
            event
        })
        .prop_filter("well under the size limit", |event| {
            size_of(event) < FIELD_TEST_MAX_BYTES
        })
}

/// Up to 6 long strings of BMP characters, 45 000 to 185 000 characters together, each with a
/// place for [`place`] (0 to 9) and a nesting depth (0 to 10). Their lengths share the total in
/// parts of 1 to 3, so two strings often have the same length (the last resort's stable order
/// decides between them). A string repeats a [`bmp_string`] of 64 to 1 023 characters: drawing
/// every character of 185 000 takes most of a test's time.
fn bulk() -> impl Strategy<Value = Vec<(u8, usize, String)>> {
    (
        45_000usize..185_000,
        prop::collection::vec((0u8..10, 0usize..=10, 1usize..=3), 1..=6),
    )
        .prop_flat_map(|(total, pieces)| {
            let weights: usize = pieces.iter().map(|&(_, _, weight)| weight).sum();
            pieces
                .into_iter()
                .map(|(spot, depth, weight)| {
                    let len = total * weight / weights;
                    let text = bmp_string(64..1024)
                        .prop_map(move |seed| seed.chars().cycle().take(len).collect::<String>());
                    (Just(spot), Just(depth), text)
                })
                .collect::<Vec<_>>()
        })
}

/// Puts the `i`th bulk string `text` into `event` at `spot`: inside `depth` objects `{"n": ...}`
/// under a key of `parameters`, `response.structuredContent`, `identifyActorData`, `error` or
/// `properties`; as a content text block; or as `error.message`, a tag, `userIntent` or
/// `resourceName`. The keys are `9`, `bulk1`, `7`, `bulk3`, ...: JavaScript puts the index keys
/// first.
fn place(event: &mut TsEvent, i: usize, spot: u8, depth: usize, text: String) {
    let key = if i.is_multiple_of(2) {
        (9 - i).to_string()
    } else {
        format!("bulk{i}")
    };
    let mut nested = Value::String(text.clone());
    for _ in 0..depth {
        nested = json!({ "n": nested });
    }
    let text = Value::String(text);
    match spot {
        0 => object_at(event, "parameters").insert(key, nested),
        1 => object_at(object_at(event, "response"), "structuredContent").insert(key, nested),
        2 => {
            push_content(event, json!({"type": "text", "text": text}));
            None
        }
        3 => object_at(event, "identifyActorData").insert(key, nested),
        4 => object_at(event, "error").insert(key, nested),
        5 => object_at(event, "error").insert("message".to_owned(), text),
        6 => object_at(event, "properties").insert(key, nested),
        7 => object_at(event, "tags").insert(key, text),
        8 => event.insert("userIntent".to_owned(), text),
        _ => event.insert("resourceName".to_owned(), text),
    };
}

/// A [`ts_event`] of BMP strings with [`bulk`] strings placed in it, 90 to 400 KB.
fn large_event() -> impl Strategy<Value = TsEvent> {
    (ts_event(bmp_string(0..24)), bulk())
        .prop_map(|(mut event, pieces)| {
            for (i, (spot, depth, text)) in pieces.into_iter().enumerate() {
                place(&mut event, i, spot, depth, text);
            }
            event
        })
        .prop_filter("90 to 400 KB", |event| {
            (90_000..=400_000).contains(&size_of(event))
        })
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn truncate_matches_ts_field_limits(event in near_limit_event()) {
        compare(&event);
    }
}

proptest! {
    #![proptest_config(config_capped(1_000))]

    #[test]
    fn truncate_matches_ts_size_guard(event in large_event()) {
        compare(&event);
    }
}

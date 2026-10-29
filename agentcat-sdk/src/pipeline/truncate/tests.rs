//! `truncate_event` against TS: the vectors of `tests/vectors/truncate.json` and
//! `tests/vectors/truncate_size.json` and `tests/vectors/truncate_cut.json`, and property tests that run the same events through the JS
//! oracle.
use proptest::prelude::*;
use serde_json::{Map, Value, json};

use super::{TruncateError, truncate_event};
use crate::js;
use crate::pipeline::TsEvent;
use crate::test_support::strategies::{bmp_string, js_string, number, ts_event};
use crate::test_support::{assert_js_eq, config, config_capped, markers, oracle, vectors};

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

/// The Rust result a case holds: `{"value": output}` for the truncated event, or `{"error":
/// error}` with the error's text when `truncate_event` returns an error.
fn reply_of(case: &Map<String, Value>, context: &str) -> Value {
    match (case.get("output"), case.get("error")) {
        (Some(output), None) => json!({ "value": output }),
        (None, Some(error)) => json!({ "error": error }),
        _ => panic!("{context}: needs exactly one of output and error"),
    }
}

/// Every case of the three fixture files: the field limits and the other rules (`truncate`), the
/// size guard (`truncate_size`) and a last-resort cut through an emoji (`truncate_cut`). Cases
/// that pin a rule by name: the `proto:` cases, that a `__proto__` key is an ordinary key and
/// stays (each `-twin` case spells it `_proto_`); `response:array`, that an array `response`
/// stays an array; `intent:length-object` and `frames:object-length-10`, that an object with a
/// `length` key is an error like any other object; `intent:array-with-tostring-key`, that an
/// object with a `toString` key is joined as `[object Object]`; and `error:100-keys-no-message`,
/// `error:array-100` and `error:99-keys-plus-dots`, that an `error` without `message` gets no
/// `message` key, so only its own keys count toward the breadth.
#[test]
fn truncate_event_by_value_matches_vectors() {
    let sets = [
        ("truncate", vectors::cases("truncate"), 10),
        ("truncate_size", vectors::cases("truncate_size"), 10),
        ("truncate_cut", vectors::cases("truncate_cut"), 1),
    ];
    for (set, cases, fewest) in sets {
        assert!(cases.len() >= fewest, "{set}.json has its cases");
        for case in &cases {
            let name = vectors::name(case);
            let context = format!("{set}.json {name}");
            let actual = truncate_event(input_of(case, &context));
            assert_matches_ts(&actual, &reply_of(case, &context), &context);
        }
    }
}

/// Runs `event` through Rust and through TS (the oracle) and compares the results.
fn compare(event: &TsEvent) {
    let actual = truncate_event(event.clone());
    let ts = oracle::call("oracle.truncateEvent", &[Value::Object(event.clone())])
        .unwrap()
        .expect("a reply");
    assert_matches_ts(&actual, &ts, "truncateEvent");
}

/// The last resort cuts strings inside an array as TS does. `properties` is not normalized, so a
/// list of long strings there is still whole when the size guard runs: the budget reaches all
/// three strings, each ends in `"..."`, and the event fits.
#[test]
fn size_guard_cuts_long_strings_inside_an_array() {
    let list: Vec<String> = ["a", "b", "c"].iter().map(|c| c.repeat(60_000)).collect();
    let event = json!({
        "id": "evt_1",
        "sessionId": "ses_1",
        "eventType": "mcp:tools/call",
        "timestamp": "2025-01-15T12:00:00.000Z",
        "properties": {"list": list},
    });
    let event = event.as_object().cloned().unwrap();
    compare(&event);
    let truncated = truncate_event(event).unwrap();
    let items = truncated["properties"]["list"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    for item in items {
        let text = item.as_str().unwrap();
        assert!(
            text.len() < 60_000 && text.ends_with("..."),
            "{}",
            text.len()
        );
    }
    assert!(
        size_of(&truncated) <= super::MAX_EVENT_BYTES,
        "{}",
        size_of(&truncated)
    );
}

/// The last resort cuts a string whose reduction is exactly [`MIN_REDUCTION`](super::MIN_REDUCTION)
/// units, as TS does: the cut of the long string leaves a budget of that many bytes, which the
/// short string spends, and the event fits.
#[test]
fn size_guard_cuts_at_the_minimum_reduction() {
    let event_with = |long: usize, short: usize| {
        let event = json!({
            "id": "evt_1",
            "sessionId": "ses_1",
            "eventType": "mcp:tools/call",
            "timestamp": "2025-01-15T12:00:00.000Z",
            "properties": {"long": "a".repeat(long), "short": "b".repeat(short)},
        });
        event.as_object().cloned().unwrap()
    };
    let short = 200;
    let base = size_of(&event_with(0, 0));
    // The excess plus the buffer is half the long string plus the minimum reduction.
    let long =
        2 * (super::MAX_EVENT_BYTES - super::BUFFER_BYTES + super::MIN_REDUCTION - base - short);
    let event = event_with(long, short);
    compare(&event);
    let truncated = truncate_event(event).unwrap();
    assert_eq!(
        truncated["properties"]["short"],
        format!("{}...", "b".repeat(short - super::MIN_REDUCTION))
    );
    assert!(size_of(&truncated) <= super::MAX_EVENT_BYTES);
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

proptest! {
    #![proptest_config(config())]

    /// The size guard measures an event with `byte_size`, which counts the UTF-8 bytes of
    /// `JSON.stringify(event)` (TS's `jsonByteSize`) without building the text: over random
    /// events, it is the length of the text `js::stringify` writes.
    #[test]
    fn byte_size_matches_stringify_len(event in ts_event(js_string(40))) {
        prop_assert_eq!(super::byte_size(&event), size_of(&event));
    }
}

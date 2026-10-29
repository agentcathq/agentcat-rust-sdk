//! `sanitize_event` against TS: the vectors of `tests/vectors/sanitize.json` and a property test
//! that runs the same events through the JS oracle.
use proptest::prelude::*;
use serde_json::{Map, Value, json};

use super::sanitize_event;
use crate::pipeline::TsEvent;
use crate::test_support::strategies::{js_string, ts_event};
use crate::test_support::{assert_js_eq, config_capped, markers, oracle, vectors};

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

/// Cases that pin a rule by name: the `proto:` cases, that a `__proto__` key is an ordinary key
/// and stays (each `-twin` case spells it `_proto_`); the `response:array` cases, that an array
/// `response` stays an array; `block:type-tostring-key`, that a `type` object with a `toString`
/// key is written `[object Object]` in the message, as any other object is (`block:type-object`).
#[test]
fn vectors_match_ts() {
    let cases = vectors::cases("sanitize");
    assert!(cases.len() >= 100, "sanitize.json has its cases");
    for case in &cases {
        let name = vectors::name(case);
        let context = format!("sanitize.json {name}");
        assert!(
            !case.contains_key("error"),
            "{context}: sanitize never fails, so a case holds no error"
        );
        let actual = Value::Object(sanitize_event(&input_of(case, &context)));
        assert_js_eq(&actual, &case["output"], &context);
    }
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

// The property test leaves out the inputs where Rust differs from TS on purpose, which only the
// vector cases of `sanitize.json` check against TS (with twins that rename the key or the type):
// a `__proto__` key in the parameters, in `structuredContent` or in a block (RD-24); an array
// `response` (RD-25); a block `type` with an own `toString` key (RD-27, where TS throws). It also
// leaves out events with other shapes than a tool-call event (no `response`, a `parameters`
// that is not `{request, extra}`): those are vector cases too. `ts_event` never makes a `toString`
// or `__proto__` key, and `odd_block` has no `type` with a `toString` key.
proptest! {
    #![proptest_config(config_capped(300))]

    #[test]
    fn sanitize_matches_ts(event in sanitize_event_input()) {
        compare(&event);
    }
}

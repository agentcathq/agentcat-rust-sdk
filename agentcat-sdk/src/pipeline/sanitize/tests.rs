//! `sanitize_event`: the cases of `tests/vectors/sanitize.json` and
//! `tests/vectors/sanitize_cases.json`, and property tests of what the step replaces and keeps.
use proptest::prelude::*;
use serde_json::{Map, Value, json};

use super::{BINARY_DATA_NOTE, sanitize_event};
use crate::pipeline::TsEvent;
use crate::test_support::json::index_keys_first;
use crate::test_support::strategies::{js_string, ts_event};
use crate::test_support::{assert_js_eq, config_capped, markers, vectors};

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

/// The cases of `sanitize_cases.json`: random tool-call events with large base64 strings and
/// content blocks of every kind. The check compares the JSON text, so the key order counts.
#[test]
fn sanitize_matches_sanitize_cases() {
    let cases = vectors::cases("sanitize_cases");
    assert!(cases.len() >= 20, "sanitize_cases.json has its cases");
    for case in &cases {
        let name = vectors::name(case);
        let actual = Value::Object(sanitize_event(&input_of(case, name)));
        assert_eq!(
            serde_json::to_string(&actual).unwrap(),
            serde_json::to_string(&case["output"]).unwrap(),
            "{name}"
        );
    }
}

/// Whether the step replaces `text`: at least 10 240 UTF-16 units of `A-Z`, `a-z`, `0-9`, `+`,
/// `/`, `\n` and `\r` (at least one), then only `=`.
fn is_large_base64(text: &str) -> bool {
    let body = text.trim_end_matches('=');
    text.encode_utf16().count() >= 10_240
        && !body.is_empty()
        && body
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+/\n\r".contains(c))
}

/// Checks a walked value (the `parameters`, or the `structuredContent` of a response): each
/// large base64 string is the note, every other value stays, and objects list their keys with
/// the array-index keys first.
fn check_walked(input: &Value, output: &Value) -> Result<(), TestCaseError> {
    match (input, output) {
        (Value::String(text), _) if is_large_base64(text) => {
            prop_assert_eq!(output, BINARY_DATA_NOTE);
        }
        (Value::Array(items), Value::Array(out)) => {
            prop_assert_eq!(items.len(), out.len());
            for (item, out) in items.iter().zip(out) {
                check_walked(item, out)?;
            }
        }
        (Value::Object(map), Value::Object(out)) => {
            let keys: Vec<&str> = out.keys().map(String::as_str).collect();
            prop_assert_eq!(keys, index_keys_first(map.keys().map(String::as_str)));
            for (key, item) in map {
                check_walked(item, &out[key])?;
            }
        }
        _ => prop_assert_eq!(output, input),
    }
    Ok(())
}

/// Whether the step keeps the content block `block` as it is: a `text` or `resource_link` block,
/// a `resource` block without a `blob` in an object `resource`, and a block that is neither an
/// object nor an array.
fn keeps_block(block: &Value) -> bool {
    let Value::Object(map) = block else {
        return !block.is_array();
    };
    match map.get("type").and_then(Value::as_str) {
        Some("text" | "resource_link") => true,
        Some("resource") => !map
            .get("resource")
            .and_then(Value::as_object)
            .is_some_and(|resource| resource.contains_key("blob")),
        _ => false,
    }
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

// The strategy makes tool-call events ([`ts_event`]) with more content blocks and other
// `response` and `parameters` values. None has a `__proto__` key, a `toString` key or an array
// `response`: the cases of `sanitize.json` hold those.
proptest! {
    #![proptest_config(config_capped(300))]

    /// A sanitized event comes back unchanged from a second run.
    #[test]
    fn sanitize_is_idempotent(event in sanitize_event_input()) {
        let once = sanitize_event(&event);
        let twice = sanitize_event(&once);
        prop_assert_eq!(serde_json::to_string(&twice).unwrap(), serde_json::to_string(&once).unwrap());
    }

    /// The step replaces only the large base64 strings of the `parameters` and of an object or
    /// array `structuredContent`. Every other field stays, and the keys of the event and of an
    /// object `response` come with the array-index keys first.
    #[test]
    fn sanitize_replaces_only_large_base64_strings_in_the_tool_values(
        event in sanitize_event_input(),
    ) {
        let sanitized = sanitize_event(&event);
        let keys: Vec<&str> = sanitized.keys().map(String::as_str).collect();
        prop_assert_eq!(keys, index_keys_first(event.keys().map(String::as_str)));
        for (key, value) in &event {
            let out = &sanitized[key];
            match (key.as_str(), value) {
                ("parameters", _) => check_walked(value, out)?,
                ("response", Value::Object(response)) => {
                    let out = out.as_object().unwrap();
                    let keys: Vec<&str> = out.keys().map(String::as_str).collect();
                    prop_assert_eq!(keys, index_keys_first(response.keys().map(String::as_str)));
                    for (key, value) in response {
                        match (key.as_str(), value) {
                            ("structuredContent", Value::Object(_) | Value::Array(_)) => {
                                check_walked(value, &out[key])?;
                            }
                            ("content", Value::Array(_)) => {}
                            _ => prop_assert_eq!(&out[key], value),
                        }
                    }
                }
                _ => prop_assert_eq!(out, value),
            }
        }
    }

    /// Each content block of an object `response` is kept when the dashboard can show it, and
    /// replaced by a text block with a note otherwise.
    #[test]
    fn sanitize_keeps_the_blocks_it_can_show_and_replaces_the_others_with_a_note(
        event in sanitize_event_input(),
    ) {
        let sanitized = sanitize_event(&event);
        let Some(Value::Array(blocks)) = event.get("response").and_then(|response| response.get("content")) else {
            return Ok(());
        };
        let out = sanitized["response"]["content"].as_array().unwrap();
        prop_assert_eq!(blocks.len(), out.len());
        for (block, out) in blocks.iter().zip(out) {
            if keeps_block(block) {
                prop_assert_eq!(out, block);
            } else {
                let note = out["text"].as_str().unwrap();
                prop_assert_eq!(out.as_object().unwrap().len(), 2);
                prop_assert_eq!(&out["type"], "text");
                prop_assert!(note.starts_with('[') && note.ends_with(" redacted - not supported by AgentCat]"), "{}", note);
            }
        }
    }
}

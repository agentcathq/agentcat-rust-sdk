//! `redact_strings` and `apply_event_redaction` against TS: the vectors of
//! `tests/vectors/redact.json` and property tests that run the same events through the JS oracle.
//!
//! The redactor of every test is the oracle's: it records each string it gets, gives
//! `"[R:" + s + "]"`, and fails on call number `throw_at` (1-based) when a case has one.
use proptest::prelude::*;
use serde_json::{Map, Value, json};

use super::{
    EVENT_HOOK_FIELD, PROTECTED_FIELDS, RESTORED_FIELDS, STRING_HOOK_FIELD, apply_event_redaction,
    redact_strings, visit_strings,
};
use crate::pipeline::TsEvent;
use crate::test_support::strategies::{js_string, json_value, object_key, ts_event};
use crate::test_support::{assert_js_eq, config, markers, oracle, vectors};

/// TS's fields for the two hooks, which the restore treats in its own way.
const HOOK_FIELDS: [&str; 2] = [STRING_HOOK_FIELD, EVENT_HOOK_FIELD];

/// What [`redact`] gives: the event, or the number of the call that failed.
type Redacted = Result<TsEvent, usize>;

/// Runs [`redact_strings`] on `event` with the oracle's redactor: the result and every string
/// the redactor got, in order.
fn redact(event: &TsEvent, throw_at: Option<usize>) -> (Redacted, Vec<String>) {
    let mut calls = Vec::new();
    let result = redact_strings(event, |text| {
        calls.push(text.to_owned());
        if Some(calls.len()) == throw_at {
            Err(calls.len())
        } else {
            Ok(format!("[R:{text}]"))
        }
    });
    (result, calls)
}

/// The event a walk case holds: `input`, or `input_text` parsed in insertion order.
fn input_of(case: &Map<String, Value>, context: &str) -> TsEvent {
    let value = match (case.get("input"), case.get("input_text")) {
        (Some(input), None) => input.clone(),
        (None, Some(Value::String(text))) => {
            markers::expand(serde_json::from_str(text).expect("input_text"))
        }
        _ => panic!("{context}: needs exactly one of input and input_text"),
    };
    event_of(value, context)
}

/// `value` as an event.
fn event_of(value: Value, context: &str) -> TsEvent {
    match value {
        Value::Object(event) => event,
        other => panic!("{context}: not an event: {other}"),
    }
}

/// The calls as a JSON array, to compare with TS's through [`assert_js_eq`].
fn calls_value(calls: Vec<String>) -> Value {
    Value::Array(calls.into_iter().map(Value::String).collect())
}

/// The `proto:` and `apply:proto-` cases pin that a `__proto__` key is an ordinary key: the
/// redactor gets the strings under it, and the key stays, also at the top level of a hook's
/// result.
#[test]
fn vectors_match_ts() {
    let cases = vectors::cases("redact");
    assert!(cases.len() >= 100, "redact.json has its cases");
    let (restore, walk): (Vec<_>, Vec<_>) =
        cases.iter().partition(|case| case.contains_key("original"));
    assert!(restore.len() >= 40, "redact.json has its restore cases");
    for case in walk {
        check_walk_case(case);
    }
    for case in restore {
        check_restore_case(case);
    }
}

/// A walk case: the calls are TS's exactly, and the event (or the error after those calls).
fn check_walk_case(case: &Map<String, Value>) {
    let name = vectors::name(case);
    let context = format!("redact.json {name}");
    let input = input_of(case, &context);
    let throw_at = case.get("throw_at").map(|n| {
        let n = n.as_u64().expect("throw_at is a count");
        usize::try_from(n).expect("throw_at fits a usize")
    });
    let (result, calls) = redact(&input, throw_at);
    assert_js_eq(
        &calls_value(calls),
        &case["calls"],
        &format!("{context}: calls"),
    );
    match (result, case.get("output"), case.get("error")) {
        (Ok(event), Some(output), None) => assert_js_eq(&Value::Object(event), output, &context),
        (Err(call), None, Some(_)) => {
            assert_eq!(Some(call), throw_at, "{context}: the failing call's error");
        }
        (result, output, error) => {
            panic!("{context}: Rust gives {result:?}; TS gives output {output:?}, error {error:?}")
        }
    }
}

/// A restore case: the event the hook's result becomes, or `None` for a drop.
fn check_restore_case(case: &Map<String, Value>) {
    let name = vectors::name(case);
    let context = format!("redact.json {name}");
    let original = event_of(case["original"].clone(), &context);
    let replacement = match case.get("replacement") {
        None | Some(Value::Null) => None,
        Some(value) => Some(event_of(value.clone(), &context)),
    };
    let actual = apply_event_redaction(&original, replacement).map(Value::Object);
    let expected = &case["output"];
    match actual {
        None => assert_eq!(expected, &Value::Null, "{context}: the event is dropped"),
        Some(actual) => assert_js_eq(&actual, expected, &context),
    }
}

/// Runs `event` through Rust and through TS (the oracle) and compares the calls and the results.
fn compare_redact(event: &TsEvent, throw_at: Option<usize>) {
    let (result, calls) = redact(event, throw_at);
    let mut request = Map::new();
    request.insert("event".to_owned(), Value::Object(event.clone()));
    if let Some(n) = throw_at {
        request.insert("throwAt".to_owned(), json!(n));
    }
    let ts = oracle::call("oracle.redactEvent", &[Value::Object(request)])
        .unwrap()
        .expect("a reply");
    assert_js_eq(&calls_value(calls), &ts["calls"], "redactEvent: calls");
    match (result, ts.get("value"), ts.get("error")) {
        (Ok(event), Some(value), None) => {
            assert_js_eq(&Value::Object(event), value, "redactEvent");
        }
        (Err(call), None, Some(_)) => assert_eq!(Some(call), throw_at, "redactEvent: error"),
        (result, value, error) => {
            panic!("Rust gives {result:?}; TS gives value {value:?}, error {error:?}")
        }
    }
}

/// Runs the restore through Rust and through TS (the oracle) and compares the results.
fn compare_restore(original: &TsEvent, replacement: Option<&TsEvent>) {
    let actual = apply_event_redaction(original, replacement.cloned()).map(Value::Object);
    let request = json!({
        "original": original,
        "replacement": replacement,
    });
    let ts = oracle::call("oracle.applyEventRedaction", &[request])
        .unwrap()
        .expect("a reply");
    match actual {
        Some(actual) => assert_js_eq(&actual, &ts, "applyEventRedaction"),
        None => assert_eq!(ts, Value::Null, "applyEventRedaction drops the event"),
    }
}

/// A key the walk treats in a special way somewhere: a protected name, `timestamp`, `""`, or an
/// [`object_key`].
fn walk_key() -> impl Strategy<Value = String> + Clone {
    prop_oneof![
        2 => prop::sample::select(PROTECTED_FIELDS.map(str::to_owned).to_vec()),
        1 => prop::sample::select(vec!["timestamp".to_owned(), String::new()]),
        2 => object_key(),
    ]
}

/// A JSON value of up to 3 levels whose object keys are [`walk_key`]s and whose strings are
/// [`js_string`]s.
fn walk_value() -> impl Strategy<Value = Value> + Clone {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i32>().prop_map(Value::from),
        js_string(20).prop_map(Value::String),
    ];
    leaf.prop_recursive(3, 24, 4, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..=4).prop_map(Value::Array),
            prop::collection::vec((walk_key(), inner), 0..=4)
                .prop_map(|entries| Value::Object(entries.into_iter().collect())),
        ]
    })
}

/// A [`ts_event`], sometimes with a `""` key (whose objects TS treats as the top level) and with
/// [`walk_value`]s in `parameters` or in place of the `timestamp`.
fn redact_input() -> impl Strategy<Value = TsEvent> {
    (
        ts_event(js_string(20)),
        prop::option::weighted(0.3, walk_value()),
        prop::option::weighted(0.3, walk_value()),
        prop::option::weighted(0.1, walk_value()),
    )
        .prop_map(|(mut event, empty_key, parameters, timestamp)| {
            if let Some(value) = empty_key {
                event.insert(String::new(), value);
            }
            if let Some(value) = parameters {
                event.insert("parameters".to_owned(), value);
            }
            if let Some(value) = timestamp {
                event.insert("timestamp".to_owned(), value);
            }
            event
        })
}

/// An event of up to 6 [`walk_key`]s with [`walk_value`]s: protected names, `timestamp` and `""`
/// at the top level and below it, nested objects and arrays, and every JSON type.
fn walk_event() -> impl Strategy<Value = TsEvent> {
    prop::collection::vec((walk_key(), walk_value()), 0..=6)
        .prop_map(|entries| entries.into_iter().collect())
}

/// A map of up to 8 entries whose keys are mostly the event's keys (the restored ones, the
/// hook fields, other fields) and whose values are [`json_value`]s or timestamps.
fn restore_map() -> impl Strategy<Value = TsEvent> + Clone {
    let key = prop_oneof![
        4 => prop::sample::select(RESTORED_FIELDS.map(str::to_owned).to_vec()),
        1 => prop::sample::select(HOOK_FIELDS.map(str::to_owned).to_vec()),
        2 => prop::sample::select(
            ["parameters", "response", "userIntent", "tags", "resourceName"]
                .map(str::to_owned)
                .to_vec()
        ),
        1 => object_key(),
    ];
    let value = prop_oneof![
        3 => json_value(2, 3),
        1 => Just(json!("2025-01-15T12:00:00.000Z")),
    ];
    prop::collection::vec((key, value), 0..=8).prop_map(|entries| entries.into_iter().collect())
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn redact_strings_matches_ts(
        event in redact_input(),
        throw_at in prop::option::weighted(0.2, 1usize..=12),
    ) {
        compare_redact(&event, throw_at);
    }

    /// C38: the async string hook gets the strings of [`visit_strings`] and gives its results to
    /// [`redact_strings`] in that order, so the two walks must visit the same strings in the
    /// same order. Fed those results in visit order, `redact_strings` builds the same event as
    /// with the redactor called inside the walk, and uses every result.
    #[test]
    fn visit_strings_matches_redact_strings_call_order(
        event in prop_oneof![redact_input(), walk_event()],
    ) {
        let mut visited = Vec::new();
        visit_strings(&event, |text| visited.push(text.to_owned()));
        let (direct, calls) = redact(&event, None);
        prop_assert_eq!(&visited, &calls);
        let mut results = visited.iter().map(|text| format!("[R:{text}]"));
        let replayed = redact_strings(&event, |_| results.next().ok_or(0_usize));
        prop_assert_eq!(replayed, direct);
        prop_assert_eq!(results.next(), None);
    }

    #[test]
    fn apply_event_redaction_matches_ts(
        original in restore_map(),
        replacement in prop::option::weighted(0.9, restore_map()),
    ) {
        compare_restore(&original, replacement.as_ref());
    }
}

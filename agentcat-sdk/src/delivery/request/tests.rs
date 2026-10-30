//! The `PublishEventRequest` of an event: its JSON values in the `JSON.stringify` text, the session
//! as `null` without one, `actor_id` from the identity, a `userIntent` that is not a string as
//! text, and the timestamp's milliseconds. The queue vectors pin every field against the expected
//! request (`pipeline::process::tests::queue_fixture_cases_hold`).
use serde_json::{Map, Value, json};

use super::publish_request;
use crate::pipeline::EventObject;
use crate::wire;

/// A tool-call event with `fields` set over the base fields.
fn event(fields: Value) -> EventObject {
    let Value::Object(mut event) = json!({
        "id": "evt_000000000000000000000000000",
        "sessionId": "ses_wire00000000000000000000000",
        "projectId": "proj_wire",
        "eventType": "mcp:tools/call",
        "timestamp": "2026-10-23T11:59:59.250Z",
        "duration": 750,
        "resourceName": "echo",
    }) else {
        unreachable!("a JSON object");
    };
    let Value::Object(fields) = fields else {
        panic!("fields must be an object");
    };
    event.extend(fields);
    event
}

/// The `serde_json` text of the request of `event`: the body `agentcat-api` posts.
fn body(event: &EventObject) -> String {
    serde_json::to_string(&publish_request(event)).unwrap()
}

/// Each JSON value of the request (`parameters`, `response`, `error`, `identify_data`,
/// `properties`) is written as `JSON.stringify` writes the event's value: `1.0` as `1`, `-0.0` as
/// `0`, 2^53 + 1 rounded to 2^53, index-like keys first in ascending order. A float is the one
/// listed exception: `serde_json` may write it in another text of the same number.
#[test]
fn wire_values_follow_the_stringify_text() {
    let value = json!({
        "10": 1.0,
        "2": -0.0,
        "big": 9_007_199_254_740_993_u64,
        "list": [1.0, {"10": "a", "2": "b"}],
    });
    let fields = [
        "parameters",
        "response",
        "error",
        "identifyActorData",
        "properties",
    ];
    let mut values = Map::new();
    for field in fields {
        values.insert(field.to_owned(), value.clone());
    }
    let sent = body(&event(Value::Object(values)));
    let text = wire::stringify(&value);
    assert_eq!(
        text,
        r#"{"2":0,"10":1,"big":9007199254740992,"list":[1,{"2":"b","10":"a"}]}"#
    );
    for key in [
        "parameters",
        "response",
        "error",
        "identify_data",
        "properties",
    ] {
        assert!(
            sent.contains(&format!(r#""{key}":{text}"#)),
            "{key}: {sent}"
        );
    }

    // A number whose `JSON.stringify` text is not a 64-bit integer reads back as an `f64`, which
    // serde_json writes in its own text: the same number, here `1e-6` for `0.000001`. (serde_json
    // 1.0.154 writes `1e+21` like `JSON.stringify`; older releases wrote `1e21`.)
    let floats = json!({"small": 0.000_001, "large": 1e21});
    assert_eq!(
        wire::stringify(&floats),
        r#"{"small":0.000001,"large":1e+21}"#
    );
    let properties = publish_request(&event(json!({"properties": floats})))
        .properties
        .unwrap();
    assert_eq!(serde_json::to_string(&properties["small"]).unwrap(), "1e-6");
    assert_eq!(properties, floats, "the same numbers");
}

/// A session of `""` or no key sends `session_id: null`: an event without a session sends
/// `session_id: null`; a session is sent as it is.
#[test]
fn sessionless_event_sends_null_session() {
    assert!(body(&event(json!({"sessionId": ""}))).contains(r#""session_id":null"#));
    let mut no_key = event(json!({}));
    no_key.shift_remove("sessionId");
    assert!(body(&no_key).contains(r#""session_id":null"#));
    assert!(body(&event(json!({}))).contains(r#""session_id":"ses_wire00000000000000000000000""#));
}

/// A Rust event has no `actorId`, so `actor_id` is the actor the `identify` hook gave, and absent
/// without one.
#[test]
fn actor_id_follows_identity() {
    let request = publish_request(&event(json!({"identifyActorGivenId": "user-42"})));
    assert_eq!(request.actor_id.as_deref(), Some("user-42"));
    assert_eq!(request.identify_actor_given_id.as_deref(), Some("user-42"));
    let anonymous = publish_request(&event(json!({})));
    assert_eq!(anonymous.actor_id, None);
    assert!(!body(&event(json!({}))).contains("actor_id"));
}

/// `agentcat-api` types `user_intent` as a string, so a `userIntent` that is not a string goes as
/// its `JSON.stringify` text; a string goes as it is, and a `null` as no value.
#[test]
fn non_string_user_intent_is_text() {
    let intent = |value: Value| publish_request(&event(json!({"userIntent": value}))).user_intent;
    assert_eq!(
        intent(json!(["plan", "act"])).as_deref(),
        Some(r#"["plan","act"]"#)
    );
    assert_eq!(intent(json!(42)).as_deref(), Some("42"));
    assert_eq!(intent(json!({"a": 1.0})).as_deref(), Some(r#"{"a":1}"#));
    assert_eq!(intent(json!("why")).as_deref(), Some("why"));
    assert_eq!(intent(Value::Null), None);
}

/// The timestamp keeps the event's milliseconds on the wire, as RFC 3339; a zero fraction is left
/// out, as chrono writes it.
#[test]
fn timestamp_keeps_milliseconds() {
    assert!(
        body(&event(json!({}))).contains(r#""timestamp":"2026-10-23T11:59:59.250Z""#),
        "{}",
        body(&event(json!({})))
    );
    let whole = event(json!({"timestamp": "2026-10-23T12:00:00.000Z"}));
    assert!(
        body(&whole).contains(r#""timestamp":"2026-10-23T12:00:00Z""#),
        "{}",
        body(&whole)
    );
}

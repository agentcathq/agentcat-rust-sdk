//! Golden vectors computed by the TypeScript SDK's own functions
//! (`scripts/ts-fixtures/generate.sh`), checked against this crate.
//!
//! Where the Rust SDK deliberately fixes a TypeScript defect, the test says
//! so and asserts the fixed behavior.

use agentcat::__private as p;
use serde_json::{Map, Value, json};

fn vectors() -> Value {
    serde_json::from_str(include_str!("fixtures/ts-vectors.json")).expect("valid fixture")
}

fn obj(v: &Value) -> Map<String, Value> {
    v.as_object().cloned().unwrap_or_default()
}

#[test]
fn sanitization_matches() {
    for case in vectors()["sanitize"].as_array().unwrap() {
        let input = &case["input"];
        let (response, parameters) =
            p::sanitize(input.get("response").cloned(), input.get("parameters").cloned());
        assert_eq!(
            response.as_ref(),
            case["output"].get("response"),
            "response of {input}"
        );
        assert_eq!(
            parameters.as_ref(),
            case["output"].get("parameters"),
            "parameters"
        );
    }
}

#[test]
fn normalize_matches() {
    for case in vectors()["normalize"].as_array().unwrap() {
        let max_string = case.get("max_string").and_then(Value::as_u64).unwrap_or(32768) as usize;
        let out = p::normalize(
            &case["input"],
            case["depth"].as_u64().unwrap() as usize,
            max_string,
        );
        assert_eq!(out, case["output"], "{case}");
    }
}

/// Maps a TS camelCase event onto the Rust event and back.
fn to_rust_event(ts: &Value) -> agentcat::Event {
    let mut e: agentcat::Event = serde_json::from_value(json!({
        "id": "", "session_id": "", "event_type": "", "timestamp": "2026-10-19T00:00:00Z"
    }))
    .unwrap();
    let s = |k: &str| ts.get(k).and_then(Value::as_str).map(str::to_owned);
    e.user_intent = s("userIntent");
    e.resource_name = s("resourceName");
    e.server_name = s("serverName");
    e.server_version = s("serverVersion");
    e.client_name = s("clientName");
    e.client_version = s("clientVersion");
    e.error = ts.get("error").cloned();
    e.parameters = ts.get("parameters").cloned();
    e.response = ts.get("response").cloned();
    e.tags = ts.get("tags").map(|t| serde_json::from_value(t.clone()).unwrap());
    e.properties = ts.get("properties").map(obj);
    e
}

#[test]
fn truncation_matches() {
    for case in vectors()["truncate"].as_array().unwrap() {
        let label = case["label"].as_str().unwrap();
        let out = p::truncate(to_rust_event(&case["input"]));
        let want = &case["output"];
        if label == "size_last_resort" {
            // The last-resort pass cuts by the byte excess of the whole event,
            // and the Rust event carries different system fields (for example
            // `sdk_language: "Rust"`), so cut lengths cannot match TS byte for
            // byte. Assert the rule instead: the event fits, and every cut
            // string ends in "...".
            let size = serde_json::to_vec(&out).unwrap().len();
            assert!(size <= 102_400, "{size}");
            let params = out.parameters.as_ref().unwrap().as_object().unwrap();
            for key in ["a", "b", "c", "d"] {
                let v = params[key].as_str().unwrap();
                let ts_len = want["parameters"][key].as_str().unwrap().len();
                assert!(v.ends_with("..."), "{key}");
                assert!(v.len().abs_diff(ts_len) < 200, "{key}: {} vs {ts_len}", v.len());
            }
            continue;
        }
        assert_eq!(
            out.user_intent.as_deref(),
            want.get("userIntent").and_then(Value::as_str),
            "{label}"
        );
        assert_eq!(
            out.parameters.as_ref(),
            want.get("parameters"),
            "{label} parameters"
        );
        assert_eq!(out.response.as_ref(), want.get("response"), "{label} response");
        assert_eq!(
            out.properties.map(Value::Object).as_ref(),
            want.get("properties"),
            "{label} properties"
        );
        if label == "fields" {
            // Fixed TS gap: names cap at the API's 255-character columns and
            // versions at 50 (TS sent 259 characters, which the API rejects).
            assert_eq!(out.resource_name.as_ref().unwrap().chars().count(), 255);
            assert!(out.resource_name.unwrap().ends_with("..."));
            assert_eq!(out.server_name.unwrap().chars().count(), 255);
            assert_eq!(out.client_name.unwrap().chars().count(), 255);
            assert_eq!(out.server_version.unwrap().chars().count(), 50);
            assert_eq!(out.client_version.unwrap().chars().count(), 50);
            assert_eq!(out.error.as_ref().unwrap()["message"], want["error"]["message"]);
            assert_eq!(out.error.as_ref().unwrap()["frames"], want["error"]["frames"]);
        }
    }
}

#[test]
fn tool_result_error_capture_matches() {
    for case in vectors()["capture_tool_result"].as_array().unwrap() {
        let e = p::capture_tool_result_error(&case["result"]);
        assert_eq!(e.message, case["error"]["message"].as_str().unwrap());
        assert_eq!(e.error_type, None);
        // Platform is "rust" here; the API groups non-JS platforms by module.
        assert_eq!(e.platform.as_deref(), Some("rust"));
    }
}

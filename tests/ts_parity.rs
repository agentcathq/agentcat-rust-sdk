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
fn derived_session_ids_match() {
    for case in vectors()["derive"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let project = case["project_id"].as_str();
        assert_eq!(
            agentcat::derive_session_id(id, project),
            case["session_id"].as_str().unwrap(),
            "{case}"
        );
    }
}

#[test]
fn ksuid_encoding_matches() {
    let v = vectors();
    for case in v["ksuid_from_parts"].as_array().unwrap() {
        let payload: Vec<u8> = case["payload"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b.as_u64().unwrap() as u8)
            .collect();
        let encoded = p::ksuid_from_parts(case["time_ms"].as_u64().unwrap(), payload.try_into().unwrap());
        assert_eq!(encoded.as_deref(), case["encoded"].as_str(), "{case}");
    }
    for case in v["ksuid_parse"].as_array().unwrap() {
        let (ts, payload) = p::ksuid_parse(case["encoded"].as_str().unwrap()).unwrap();
        assert_eq!(u64::from(ts), case["timestamp"].as_u64().unwrap());
        let expected: Vec<u8> = case["payload"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b.as_u64().unwrap() as u8)
            .collect();
        assert_eq!(payload, expected);
    }
}

#[cfg(feature = "posthog")]
#[test]
fn posthog_uuid_v7_matches() {
    for case in vectors()["uuidv7"].as_array().unwrap() {
        assert_eq!(
            p::to_uuid_v7(case["id"].as_str().unwrap()),
            case["uuid"].as_str().unwrap(),
            "{case}"
        );
    }
}

#[test]
fn trace_context_matches() {
    for case in vectors()["trace_context"].as_array().unwrap() {
        let (trace, span, dd_trace, dd_span) = p::trace_context(case["id"].as_str().unwrap());
        assert_eq!(trace, case["trace_id"].as_str().unwrap());
        assert_eq!(span, case["span_id"].as_str().unwrap());
        assert_eq!(dd_trace, case["dd_trace_id"].as_str().unwrap());
        assert_eq!(dd_span, case["dd_span_id"].as_str().unwrap());
    }
}

#[test]
fn session_id_validation_matches() {
    for case in vectors()["is_valid_session_id"].as_array().unwrap() {
        assert_eq!(
            p::is_valid_session_id(case["value"].as_str().unwrap()),
            case["valid"].as_bool().unwrap(),
            "{case}"
        );
    }
}

#[test]
fn handle_extraction_matches() {
    for case in vectors()["extract_handle"].as_array().unwrap() {
        let args = obj(&case["args"]);
        let got = p::extract_handle(Some(&args), case["name"].as_str().unwrap());
        assert_eq!(got.as_deref(), case["value"].as_str(), "{case}");
    }
}

#[test]
fn resolution_mint_back_and_tags_match() {
    for case in vectors()["resolutions"].as_array().unwrap() {
        let label = case["label"].as_str().unwrap();
        let (hook_mode, agent_tracking, args, ours) = match label {
            "omitted" => (false, false, json!({"q": 1}), true),
            "start" => (false, false, json!({"session_id": " START "}), true),
            "supplied" => (
                false,
                false,
                json!({"session_id": "ses_2cOHEO0LYGADMzRvWTXXVbbgxgm"}),
                true,
            ),
            "invalid" => (false, false, json!({"session_id": "ses_invented"}), true),
            "foreign" => (false, false, json!({"session_id": "customer-owned"}), false),
            "hook" => (
                true,
                false,
                json!({"session_id": "ses_2cOHEO0LYGADMzRvWTXXVbbgxgm"}),
                true,
            ),
            "agent" => (
                false,
                true,
                json!({"session_id": "ses_2cOHEO0LYGADMzRvWTXXVbbgxgm", "agent_id": " opus-4.80-1m|claude-code|k3n9x "}),
                true,
            ),
            "agent_off" => (false, false, json!({"agent_id": "opus"}), true),
            "agent_foreign" => (
                false,
                true,
                json!({"session_id": "x", "agent_id": format!("line1\nline2\r{}", "z".repeat(300))}),
                false,
            ),
            other => panic!("unknown case {other}"),
        };
        let args = obj(&args);
        let (source, session_id, agent_id, text, structured, tags) =
            p::resolve_for_test(hook_mode, agent_tracking, Some(&args), ours, Some("2026-07-28"));
        assert_eq!(source, case["session_source"].as_str().unwrap(), "{label}");
        let minted = source == "minted";
        if minted {
            assert!(p::is_valid_session_id(&session_id));
        } else {
            assert_eq!(session_id, case["session_id"].as_str().unwrap(), "{label}");
        }
        assert_eq!(agent_id.as_deref(), case["agent_id"].as_str(), "{label}");
        let text = text.map(|t| {
            if minted {
                t.replace(&session_id, "<minted>")
            } else {
                t
            }
        });
        assert_eq!(text.as_deref(), case["mint_back_text"].as_str(), "{label}");
        let structured = structured.map(|mut m| {
            if minted {
                m.insert("session_id".into(), json!("<minted>"));
            }
            Value::Object(m)
        });
        assert_eq!(structured.unwrap_or(Value::Null), case["structured"], "{label}");
        assert_eq!(serde_json::to_value(&tags).unwrap(), case["tags"], "{label}");
    }
}

#[test]
fn structured_mirror_matches() {
    let v = vectors();
    for case in v["mirror"].as_array().unwrap() {
        let mut sc = case["result"].get("structuredContent").cloned();
        p::mirror_for_test(&mut sc, obj(&case["mint"]));
        let expected = case["out"].get("structuredContent").cloned();
        // Byte equality checks that mcp_session is the first key.
        assert_eq!(
            serde_json::to_string(&sc).unwrap(),
            serde_json::to_string(&expected).unwrap(),
            "{case}"
        );
    }
}

#[test]
fn argument_stripping_matches() {
    for case in vectors()["strip"].as_array().unwrap() {
        let tool = case["tool"].as_str().unwrap();
        let mut args = obj(&case["args"]);
        let entry = case["registry"].get(tool).and_then(Value::as_array).map(|names| {
            let has = |n: &str| names.iter().any(|v| v == n);
            (has("session_id"), has("agent_id"), has("context"))
        });
        p::strip_for_test(&mut args, tool, entry);
        assert_eq!(Value::Object(args), case["out"], "{case}");
    }
}

#[test]
fn report_missing_descriptor_matches() {
    let v = vectors();
    let ts: Value = serde_json::from_str(v["report_missing"]["descriptor_json"].as_str().unwrap()).unwrap();
    assert_eq!(
        serde_json::to_string(&p::report_missing_descriptor()).unwrap(),
        serde_json::to_string(&ts).unwrap()
    );
}

#[test]
fn token_estimates_match() {
    let v = vectors();
    for case in v["tokens_input"].as_array().unwrap() {
        let args = (!case["args"].is_null()).then(|| case["args"].clone());
        assert_eq!(
            p::estimate_input_tokens(args.as_ref()).map(u64::from),
            case["tokens"].as_u64(),
            "{case}"
        );
    }
    for case in v["tokens_output"].as_array().unwrap() {
        let response = (!case["response"].is_null()).then(|| case["response"].clone());
        assert_eq!(
            p::estimate_output_tokens(response.as_ref()).map(u64::from),
            case["tokens"].as_u64(),
            "{case}"
        );
    }
    for case in v["tokens_bytes"].as_array().unwrap() {
        assert_eq!(
            u64::from(p::estimate_tokens(case["bytes"].as_f64().unwrap())),
            case["tokens"].as_u64().unwrap(),
            "{case}"
        );
    }
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

#[test]
fn constants_match_ts_source() {
    let ts: Value = serde_json::from_str(include_str!("fixtures/ts-constants.json")).unwrap();
    use agentcat::constants as c;
    let pairs: &[(&str, &str)] = &[
        (
            "DEFAULT_CONTEXT_PARAMETER_DESCRIPTION",
            c::DEFAULT_CONTEXT_PARAMETER_DESCRIPTION,
        ),
        ("AGENTCAT_CUSTOM_EVENT_TYPE", c::AGENTCAT_CUSTOM_EVENT_TYPE),
        ("AGENTCAT_SOURCE", c::AGENTCAT_SOURCE),
        ("SESSION_ID_PARAM_DESCRIPTION", c::SESSION_ID_PARAM_DESCRIPTION),
        ("SESSION_ID_PARAM_PATTERN", c::SESSION_ID_PARAM_PATTERN),
        ("SESSION_START_SENTINEL", c::SESSION_START_SENTINEL),
        ("AGENT_ID_PARAM_DESCRIPTION", c::AGENT_ID_PARAM_DESCRIPTION),
        ("MINT_BACK_HEADER_ISSUED", c::MINT_BACK_HEADER_ISSUED),
        ("MINT_BACK_ISSUED_BODY", c::MINT_BACK_ISSUED_BODY),
        ("MINT_BACK_HEADER_UNRECOGNIZED", c::MINT_BACK_HEADER_UNRECOGNIZED),
        ("MINT_BACK_UNRECOGNIZED_BODY", c::MINT_BACK_UNRECOGNIZED_BODY),
        ("MCP_SESSION_KEY", c::MCP_SESSION_KEY),
        ("MCP_SESSION_FIELD_DESCRIPTION", c::MCP_SESSION_FIELD_DESCRIPTION),
        (
            "MCP_SESSION_FIELD_DESCRIPTION_HOOK_MODE",
            c::MCP_SESSION_FIELD_DESCRIPTION_HOOK_MODE,
        ),
        (
            "MCP_SESSION_SESSION_ID_DESCRIPTION",
            c::MCP_SESSION_SESSION_ID_DESCRIPTION,
        ),
        (
            "MCP_SESSION_AGENT_ID_DESCRIPTION",
            c::MCP_SESSION_AGENT_ID_DESCRIPTION,
        ),
        (
            "MCP_SESSION_STATUS_DESCRIPTION",
            c::MCP_SESSION_STATUS_DESCRIPTION,
        ),
        ("META_CLIENT_INFO_KEY", c::META_CLIENT_INFO_KEY),
        ("META_PROTOCOL_VERSION_KEY", c::META_PROTOCOL_VERSION_KEY),
        ("AGENTCAT_TAG_AGENT_ID", c::AGENTCAT_TAG_AGENT_ID),
        ("AGENTCAT_TAG_SESSION_SOURCE", c::AGENTCAT_TAG_SESSION_SOURCE),
        ("AGENTCAT_TAG_AGENT_SOURCE", c::AGENTCAT_TAG_AGENT_SOURCE),
        ("AGENTCAT_TAG_PROTOCOL_VERSION", c::AGENTCAT_TAG_PROTOCOL_VERSION),
        ("AGENTCAT_TAG_MRTR", c::AGENTCAT_TAG_MRTR),
        ("GET_MORE_TOOLS_NAME", c::GET_MORE_TOOLS_NAME),
        ("GET_MORE_TOOLS_DESCRIPTION", c::GET_MORE_TOOLS_DESCRIPTION),
        (
            "GET_MORE_TOOLS_CONTEXT_DESCRIPTION",
            c::GET_MORE_TOOLS_CONTEXT_DESCRIPTION,
        ),
        ("GET_MORE_TOOLS_TITLE", c::GET_MORE_TOOLS_TITLE),
        ("GET_MORE_TOOLS_RESPONSE", c::GET_MORE_TOOLS_RESPONSE),
    ];
    for (name, value) in pairs {
        assert_eq!(ts[name].as_str(), Some(*value), "{name}");
    }
}

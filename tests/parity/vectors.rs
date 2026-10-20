//! Golden vectors computed by the TypeScript SDK's own functions
//! (`scripts/ts-fixtures/generate.sh`), checked as they are.

use super::support::{capture_logs, fixture_json, obj};
use crate::handles::{
    build_handle_tags, build_mint_back_text, build_structured_mint_back, extract_handle,
    mirror_structured_mint_back, resolve_handles,
};
use crate::inject::{
    InjectConfig, InjectMemory, Injected, InjectedList, build_injected_list, heuristic_strip,
    report_missing_descriptor, strip_arguments,
};
use crate::ksuid::{Ksuid, derive_session_id, is_valid_session_id};
use crate::tokens::{estimate_input_tokens, estimate_output_tokens, estimate_tokens};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// A JSON array serialized by the TS generator.
fn parse_tools(json: &Value) -> Vec<Value> {
    serde_json::from_str(json.as_str().expect("a JSON string")).expect("a JSON array of tools")
}

fn registry_names(list: &InjectedList) -> BTreeMap<String, BTreeSet<String>> {
    list.registry
        .iter()
        .map(|(tool, entry)| {
            let names = entry.names().into_iter().map(str::to_owned).collect();
            (tool.clone(), names)
        })
        .collect()
}

fn ts_registry(value: &Value) -> BTreeMap<String, BTreeSet<String>> {
    serde_json::from_value(value.clone()).expect("{tool: [names]}")
}

#[test]
fn schema_injection_matches() {
    let v = fixture_json("ts-vectors.json");
    let inputs = parse_tools(&v["injection_input_json"]);
    for config in v["injection"].as_array().expect("injection configs") {
        let label = config["label"].as_str().expect("a label");
        let c = &config["config"];
        let flag = |key: &str, default: bool| c.get(key).and_then(Value::as_bool).unwrap_or(default);
        let cfg = InjectConfig::new(
            flag("enableTracing", true),
            c.get("resolveSessionId").is_some_and(|v| v == true),
            flag("enableAgentTracking", false),
            flag("enableToolCallContext", true),
            c.get("customContextDescription").and_then(Value::as_str),
            flag("enableReportMissing", true),
        );
        let (list, _) = capture_logs(|| build_injected_list(&cfg, &InjectMemory::default(), &inputs));
        assert_eq!(
            serde_json::to_string(&list.tools).unwrap(),
            serde_json::to_string(&parse_tools(&config["tools_json"])).unwrap(),
            "{label}: tools"
        );
        assert_eq!(
            registry_names(&list),
            ts_registry(&config["registry"]),
            "{label}: registry"
        );
        let output: BTreeSet<String> =
            serde_json::from_value(config["output_registry"].clone()).expect("an array of names");
        assert_eq!(
            list.output_registry.iter().cloned().collect::<BTreeSet<_>>(),
            output,
            "{label}: output registry"
        );
    }

    // The customer's own get_more_tools: listed once, never replaced.
    let case = &v["injection_customer_get_more_tools"];
    let cfg = InjectConfig::new(true, false, false, true, None, true);
    let (list, logs) = capture_logs(|| {
        build_injected_list(&cfg, &InjectMemory::default(), &parse_tools(&case["input_json"]))
    });
    assert_eq!(
        serde_json::to_string(&list.tools).unwrap(),
        serde_json::to_string(&parse_tools(&case["tools_json"])).unwrap()
    );
    assert_eq!(registry_names(&list), ts_registry(&case["registry"]));
    assert_eq!(
        logs,
        [
            "WARN: This server registers its own \"get_more_tools\" tool, which is shadowed by AgentCat's report-missing tool: calls to it are answered by AgentCat and never reach your handler. Rename your tool, or set enableReportMissing: false to keep yours reachable."
        ]
    );
}

fn bytes_of(value: &Value) -> Vec<u8> {
    value
        .as_array()
        .expect("a byte array")
        .iter()
        .map(|b| u8::try_from(b.as_u64().expect("a byte")).expect("a byte"))
        .collect()
}

#[test]
fn derived_session_ids_match() {
    for case in fixture_json("ts-vectors.json")["derive"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let project = case["project_id"].as_str();
        assert_eq!(
            derive_session_id(id, project),
            case["session_id"].as_str().unwrap(),
            "{case}"
        );
    }
}

#[test]
fn ksuid_encoding_matches() {
    let v = fixture_json("ts-vectors.json");
    for case in v["ksuid_from_parts"].as_array().unwrap() {
        let encoded = Ksuid::from_parts(case["time_ms"].as_u64().unwrap(), &bytes_of(&case["payload"]))
            .ok()
            .map(|k| k.encode());
        assert_eq!(encoded.as_deref(), case["encoded"].as_str(), "{case}");
    }
    for case in v["ksuid_parse"].as_array().unwrap() {
        let k = Ksuid::parse(case["encoded"].as_str().unwrap()).unwrap();
        assert_eq!(u64::from(k.timestamp()), case["timestamp"].as_u64().unwrap());
        assert_eq!(k.payload().to_vec(), bytes_of(&case["payload"]));
    }
}

#[cfg(feature = "posthog")]
#[test]
fn posthog_uuid_v7_matches() {
    for case in fixture_json("ts-vectors.json")["uuidv7"].as_array().unwrap() {
        assert_eq!(
            crate::exporters::posthog_uuid_v7(case["id"].as_str().unwrap()),
            case["uuid"].as_str().unwrap(),
            "{case}"
        );
    }
}

#[test]
fn trace_context_matches() {
    use crate::exporters::trace_context as t;
    for case in fixture_json("ts-vectors.json")["trace_context"]
        .as_array()
        .unwrap()
    {
        let id = case["id"].as_str().unwrap();
        assert_eq!(t::trace_id(id), case["trace_id"].as_str().unwrap());
        assert_eq!(t::span_id(id), case["span_id"].as_str().unwrap());
        assert_eq!(t::datadog_trace_id(id), case["dd_trace_id"].as_str().unwrap());
        assert_eq!(t::datadog_span_id(id), case["dd_span_id"].as_str().unwrap());
    }
}

#[test]
fn session_id_validation_matches() {
    for case in fixture_json("ts-vectors.json")["is_valid_session_id"]
        .as_array()
        .unwrap()
    {
        assert_eq!(
            is_valid_session_id(case["value"].as_str().unwrap()),
            case["valid"].as_bool().unwrap(),
            "{case}"
        );
    }
}

#[test]
fn handle_extraction_matches() {
    for case in fixture_json("ts-vectors.json")["extract_handle"]
        .as_array()
        .unwrap()
    {
        let args = obj(case["args"].clone());
        let got = extract_handle(Some(&args), case["name"].as_str().unwrap());
        assert_eq!(got.as_deref(), case["value"].as_str(), "{case}");
    }
}

#[test]
fn resolution_mint_back_and_tags_match() {
    for case in fixture_json("ts-vectors.json")["resolutions"].as_array().unwrap() {
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
        let args = obj(args);
        let res = resolve_handles(hook_mode, agent_tracking, Some(&args), ours);
        let source = res.session_source.as_str();
        assert_eq!(source, case["session_source"].as_str().unwrap(), "{label}");
        let minted = source == "minted";
        if minted {
            assert!(is_valid_session_id(&res.session_id));
        } else {
            assert_eq!(res.session_id, case["session_id"].as_str().unwrap(), "{label}");
        }
        assert_eq!(res.agent_id.as_deref(), case["agent_id"].as_str(), "{label}");
        let text = build_mint_back_text(&res).map(|t| {
            if minted {
                t.replace(&res.session_id, "<minted>")
            } else {
                t
            }
        });
        assert_eq!(text.as_deref(), case["mint_back_text"].as_str(), "{label}");
        let structured = build_structured_mint_back(&res).map(|mut m| {
            if minted {
                m.insert("session_id".into(), json!("<minted>"));
            }
            Value::Object(m)
        });
        assert_eq!(structured.unwrap_or(Value::Null), case["structured"], "{label}");
        let tags = build_handle_tags(&res, Some("2026-07-28"));
        assert_eq!(serde_json::to_value(&tags).unwrap(), case["tags"], "{label}");
    }
}

#[test]
fn structured_mirror_matches() {
    for case in fixture_json("ts-vectors.json")["mirror"].as_array().unwrap() {
        let mut sc = case["result"].get("structuredContent").cloned();
        mirror_structured_mint_back(&mut sc, obj(case["mint"].clone()));
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
    for case in fixture_json("ts-vectors.json")["strip"].as_array().unwrap() {
        let tool = case["tool"].as_str().unwrap();
        let mut args = obj(case["args"].clone());
        let strip = match case["registry"].get(tool).and_then(Value::as_array) {
            Some(names) => {
                let has = |n: &str| names.iter().any(|v| v == n);
                Injected {
                    session_id: has("session_id"),
                    agent_id: has("agent_id"),
                    context: has("context"),
                }
            }
            None => heuristic_strip(tool),
        };
        strip_arguments(&mut args, strip);
        assert_eq!(Value::Object(args), case["out"], "{case}");
    }
}

#[test]
fn report_missing_descriptor_matches() {
    let v = fixture_json("ts-vectors.json");
    let ts: Value = serde_json::from_str(v["report_missing"]["descriptor_json"].as_str().unwrap()).unwrap();
    assert_eq!(
        serde_json::to_string(&report_missing_descriptor()).unwrap(),
        serde_json::to_string(&ts).unwrap()
    );
}

#[test]
fn token_estimates_match() {
    let v = fixture_json("ts-vectors.json");
    for case in v["tokens_input"].as_array().unwrap() {
        let args = (!case["args"].is_null()).then(|| case["args"].clone());
        assert_eq!(
            estimate_input_tokens(args.as_ref()).map(u64::from),
            case["tokens"].as_u64(),
            "{case}"
        );
    }
    for case in v["tokens_output"].as_array().unwrap() {
        let response = (!case["response"].is_null()).then(|| case["response"].clone());
        assert_eq!(
            estimate_output_tokens(response.as_ref()).map(u64::from),
            case["tokens"].as_u64(),
            "{case}"
        );
    }
    for case in v["tokens_bytes"].as_array().unwrap() {
        assert_eq!(
            u64::from(estimate_tokens(case["bytes"].as_f64().unwrap())),
            case["tokens"].as_u64().unwrap(),
            "{case}"
        );
    }
}

#[test]
fn constants_match_ts_source() {
    use crate::constants as c;
    let ts = fixture_json("ts-constants.json");
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

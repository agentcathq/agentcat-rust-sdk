//! The constants of `constants` against `tests/vectors/constants.json`, which the SDK wrote.
use std::sync::OnceLock;

use serde_json::{Map, Value};

use super::*;
use crate::test_support::{assert_same_json_text, vectors};
use crate::wire;

/// Fixture exports that stay out of the crate for now (exporters and diagnostics: a later
/// release).
const NOT_PORTED: [&str; 4] = [
    "AGENTCAT_SOURCE",
    "DIAGNOSTICS_SCOPE_NAME",
    "DEFAULT_DIAGNOSTICS_ENDPOINT",
    "DEFAULT_DIAGNOSTICS_TOKEN",
];

/// The cases of functions, each named after the function it pins; every other case is a
/// constant, named after the constant.
const FUNCTION_CASES: [&str; 3] = [
    "mint_back_session_line",
    "get_more_tools_descriptor",
    "get_more_tools_result",
];

/// Every ported constant, by its export name.
const PORTED: [(&str, &str); 26] = [
    (
        "DEFAULT_CONTEXT_PARAMETER_DESCRIPTION",
        DEFAULT_CONTEXT_PARAMETER_DESCRIPTION,
    ),
    ("AGENTCAT_CUSTOM_EVENT_TYPE", AGENTCAT_CUSTOM_EVENT_TYPE),
    ("SESSION_ID_PARAM_DESCRIPTION", SESSION_ID_PARAM_DESCRIPTION),
    ("SESSION_ID_PARAM_PATTERN", SESSION_ID_PARAM_PATTERN),
    ("SESSION_START_SENTINEL", SESSION_START_SENTINEL),
    ("AGENT_ID_PARAM_DESCRIPTION", AGENT_ID_PARAM_DESCRIPTION),
    ("MINT_BACK_HEADER_ISSUED", MINT_BACK_HEADER_ISSUED),
    ("MINT_BACK_ISSUED_BODY", MINT_BACK_ISSUED_BODY),
    (
        "MINT_BACK_HEADER_UNRECOGNIZED",
        MINT_BACK_HEADER_UNRECOGNIZED,
    ),
    ("MINT_BACK_UNRECOGNIZED_BODY", MINT_BACK_UNRECOGNIZED_BODY),
    ("MCP_SESSION_KEY", MCP_SESSION_KEY),
    (
        "MCP_SESSION_FIELD_DESCRIPTION",
        MCP_SESSION_FIELD_DESCRIPTION,
    ),
    (
        "MCP_SESSION_FIELD_DESCRIPTION_HOOK_MODE",
        MCP_SESSION_FIELD_DESCRIPTION_HOOK_MODE,
    ),
    (
        "MCP_SESSION_SESSION_ID_DESCRIPTION",
        MCP_SESSION_SESSION_ID_DESCRIPTION,
    ),
    (
        "MCP_SESSION_AGENT_ID_DESCRIPTION",
        MCP_SESSION_AGENT_ID_DESCRIPTION,
    ),
    (
        "MCP_SESSION_STATUS_DESCRIPTION",
        MCP_SESSION_STATUS_DESCRIPTION,
    ),
    ("META_CLIENT_INFO_KEY", META_CLIENT_INFO_KEY),
    ("META_PROTOCOL_VERSION_KEY", META_PROTOCOL_VERSION_KEY),
    ("AGENTCAT_TAG_AGENT_ID", AGENTCAT_TAG_AGENT_ID),
    ("AGENTCAT_TAG_SESSION_SOURCE", AGENTCAT_TAG_SESSION_SOURCE),
    ("AGENTCAT_TAG_AGENT_SOURCE", AGENTCAT_TAG_AGENT_SOURCE),
    (
        "AGENTCAT_TAG_PROTOCOL_VERSION",
        AGENTCAT_TAG_PROTOCOL_VERSION,
    ),
    ("AGENTCAT_TAG_MRTR", AGENTCAT_TAG_MRTR),
    ("GET_MORE_TOOLS_NAME", GET_MORE_TOOLS_NAME),
    ("SESSION_ID_PARAM", SESSION_ID_PARAM),
    ("AGENT_ID_PARAM", AGENT_ID_PARAM),
];

/// The cases of `constants.json`, read once for the whole test binary.
fn cases() -> &'static [Map<String, Value>] {
    static CASES: OnceLock<Vec<Map<String, Value>>> = OnceLock::new();
    CASES.get_or_init(|| vectors::cases("constants"))
}

fn case(name: &str) -> Map<String, Value> {
    cases()
        .iter()
        .find(|c| vectors::name(c) == name)
        .cloned()
        .unwrap_or_else(|| panic!("no vector case {name:?}"))
}

fn expected(name: &str) -> Value {
    case(name)
        .remove("value")
        .unwrap_or_else(|| panic!("case {name:?} has no value"))
}

#[test]
fn every_constant_equals_its_fixture_value() {
    for (name, actual) in PORTED {
        assert_same_json_text(&Value::from(actual), &expected(name), name);
    }
}

#[test]
fn every_fixture_constant_is_ported_or_listed() {
    let plain: Vec<String> = cases()
        .iter()
        .map(|c| vectors::name(c).to_owned())
        .filter(|n| !FUNCTION_CASES.contains(&n.as_str()))
        .collect();
    for name in &plain {
        let ported = PORTED.iter().any(|(p, _)| p == name);
        let listed = NOT_PORTED.contains(&name.as_str());
        assert!(
            ported != listed,
            "{name}: must be ported xor listed as not ported"
        );
    }
    for (name, _) in PORTED {
        assert!(
            plain.iter().any(|p| p == name),
            "{name}: no fixture constant of that name"
        );
    }
    for name in NOT_PORTED {
        assert!(
            plain.iter().any(|p| p == name),
            "{name}: no fixture constant of that name"
        );
    }
}

#[test]
fn mint_back_session_line_is_pinned() {
    let case = case("mint_back_session_line");
    let input = case["input"].as_str().unwrap_or_else(|| {
        panic!(
            "mint_back_session_line: input is not a string: {:?}",
            case["input"]
        )
    });
    assert_same_json_text(
        &Value::from(mint_back_session_line(input)),
        &case["value"],
        "mint_back_session_line",
    );
}

#[test]
fn get_more_tools_descriptor_is_pinned() {
    assert_same_json_text(
        &get_more_tools_descriptor(),
        &expected("get_more_tools_descriptor"),
        "get_more_tools_descriptor",
    );
}

#[test]
fn get_more_tools_result_is_pinned() {
    assert_same_json_text(
        &get_more_tools_result(),
        &expected("get_more_tools_result"),
        "get_more_tools_result",
    );
}

#[test]
fn get_more_tools_result_text_is_pinned() {
    let value = expected("get_more_tools_result");
    assert_same_json_text(
        &Value::from(GET_MORE_TOOLS_RESULT_TEXT),
        &value["content"][0]["text"],
        "get_more_tools_result text",
    );
}

/// The `logs` of the `get_more_tools_result` case hold the one line that
/// [`report_missing_log_line`] writes for the case's `context` string, whose length counts
/// UTF-16 units.
#[test]
fn report_missing_log_line_is_pinned() {
    let case = case("get_more_tools_result");
    let context = case["input"]["context"].as_str().unwrap_or_else(|| {
        panic!(
            "get_more_tools_result: input.context is not a string: {:?}",
            case["input"]
        )
    });
    let logs = case["logs"].as_array().unwrap_or_else(|| {
        panic!(
            "get_more_tools_result: logs is not an array: {:?}",
            case.get("logs")
        )
    });
    assert_eq!(
        logs.len(),
        1,
        "get_more_tools_result logs one line: {logs:?}"
    );
    assert_same_json_text(
        &Value::from(report_missing_log_line(wire::utf16_len(context))),
        &logs[0],
        "get_more_tools_result log",
    );
}

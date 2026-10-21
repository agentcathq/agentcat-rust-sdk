//! The constants of `constants` against `tests/vectors/constants.json`, which the TS SDK wrote.
use serde_json::{Map, Value};

use super::*;
use crate::test_support::{assert_js_eq, vectors};

/// TS exports that stay out of the crate for now (exporters and diagnostics: second pass, R04,
/// C05, RD-16).
const NOT_PORTED: [&str; 4] = [
    "AGENTCAT_SOURCE",
    "DIAGNOSTICS_SCOPE_NAME",
    "DEFAULT_DIAGNOSTICS_ENDPOINT",
    "DEFAULT_DIAGNOSTICS_TOKEN",
];

/// The TS function cases; every other case is a plain constant.
const FUNCTION_CASES: [&str; 3] = [
    "mintBackSessionLine",
    "getReportMissingToolDescriptor",
    "handleReportMissing",
];

/// Every ported constant, by its TS export name.
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

fn case(name: &str) -> Map<String, Value> {
    vectors::cases("constants")
        .into_iter()
        .find(|c| vectors::name(c) == name)
        .unwrap_or_else(|| panic!("no vector case {name:?}"))
}

fn expected(name: &str) -> Value {
    case(name)
        .remove("value")
        .unwrap_or_else(|| panic!("case {name:?} has no value"))
}

#[test]
fn every_ported_constant_equals_ts() {
    for (name, actual) in PORTED {
        assert_js_eq(&Value::from(actual), &expected(name), name);
    }
}

#[test]
fn every_ts_constant_is_ported_or_listed() {
    let plain: Vec<String> = vectors::cases("constants")
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
            "{name}: no TS constant of that name"
        );
    }
    for name in NOT_PORTED {
        assert!(
            plain.iter().any(|p| p == name),
            "{name}: no TS constant of that name"
        );
    }
}

#[test]
fn mint_back_session_line_matches_ts() {
    let case = case("mintBackSessionLine");
    let input = case["input"].as_str().unwrap_or_default();
    assert_js_eq(
        &Value::from(mint_back_session_line(input)),
        &case["value"],
        "mintBackSessionLine",
    );
}

#[test]
fn get_more_tools_descriptor_matches_ts() {
    assert_js_eq(
        &get_more_tools_descriptor(),
        &expected("getReportMissingToolDescriptor"),
        "getReportMissingToolDescriptor",
    );
}

#[test]
fn get_more_tools_result_matches_ts() {
    assert_js_eq(
        &get_more_tools_result(),
        &expected("handleReportMissing"),
        "handleReportMissing",
    );
}

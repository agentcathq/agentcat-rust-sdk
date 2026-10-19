//! Golden-file checks against the shared PHP audit fixtures.

use super::support::{fixture_json, fixture_text};
use crate::constants::*;
use crate::ksuid::derive_session_id;
use crate::util::{hex, sha256, utf16_len};

fn constant(name: &str) -> Option<&'static str> {
    Some(match name {
        "DEFAULT_CONTEXT_PARAMETER_DESCRIPTION" => DEFAULT_CONTEXT_PARAMETER_DESCRIPTION,
        "SESSION_ID_PARAM_DESCRIPTION" => SESSION_ID_PARAM_DESCRIPTION,
        "SESSION_ID_PARAM_PATTERN" => SESSION_ID_PARAM_PATTERN,
        "AGENT_ID_PARAM_DESCRIPTION" => AGENT_ID_PARAM_DESCRIPTION,
        "MINT_BACK_HEADER_ISSUED" => MINT_BACK_HEADER_ISSUED,
        "MINT_BACK_ISSUED_BODY" => MINT_BACK_ISSUED_BODY,
        "MINT_BACK_HEADER_UNRECOGNIZED" => MINT_BACK_HEADER_UNRECOGNIZED,
        "MINT_BACK_UNRECOGNIZED_BODY" => MINT_BACK_UNRECOGNIZED_BODY,
        "MCP_SESSION_FIELD_DESCRIPTION" => MCP_SESSION_FIELD_DESCRIPTION,
        "MCP_SESSION_FIELD_DESCRIPTION_HOOK_MODE" => MCP_SESSION_FIELD_DESCRIPTION_HOOK_MODE,
        "MCP_SESSION_SESSION_ID_DESCRIPTION" => MCP_SESSION_SESSION_ID_DESCRIPTION,
        "MCP_SESSION_AGENT_ID_DESCRIPTION" => MCP_SESSION_AGENT_ID_DESCRIPTION,
        "MCP_SESSION_STATUS_DESCRIPTION" => MCP_SESSION_STATUS_DESCRIPTION,
        "GET_MORE_TOOLS_DESCRIPTION" => GET_MORE_TOOLS_DESCRIPTION,
        "GET_MORE_TOOLS_CONTEXT_DESCRIPTION" => GET_MORE_TOOLS_CONTEXT_DESCRIPTION,
        "GET_MORE_TOOLS_RESPONSE_TEXT" => GET_MORE_TOOLS_RESPONSE,
        _ => return None,
    })
}

#[test]
fn agent_facing_strings_match_the_ts_fixture() {
    let strings = fixture_json("php-audit/handles/strings.json");
    let mut checked = 0;
    for (name, expected) in strings.as_object().expect("an object") {
        let Some(actual) = constant(name) else {
            continue;
        };
        assert_eq!(
            Some(actual),
            expected.as_str(),
            "{name} differs from the TS fixture"
        );
        checked += 1;
    }
    assert_eq!(checked, 13);
}

#[test]
fn golden_txt_lengths_and_hashes_match() {
    let golden = fixture_text("php-audit/handles/golden.txt");
    let mut checked = 0;
    for line in golden.lines().take_while(|l| *l != "---JSON---") {
        let cols: Vec<&str> = line.split('\t').collect();
        let Some(actual) = constant(cols[0]) else {
            continue;
        };
        let field = |i: usize, key: &str| -> String {
            cols[i]
                .strip_prefix(key)
                .unwrap_or_else(|| panic!("{} has no {key}", cols[0]))
                .to_owned()
        };
        assert_eq!(
            utf16_len(actual).to_string(),
            field(1, "len_utf16="),
            "{}",
            cols[0]
        );
        assert_eq!(actual.len().to_string(), field(2, "bytes="), "{}", cols[0]);
        assert_eq!(
            hex(&sha256(actual.as_bytes())),
            field(3, "sha256="),
            "{}",
            cols[0]
        );
        assert_eq!(
            actual.matches('\u{2014}').count().to_string(),
            field(4, "emdash="),
            "{}",
            cols[0]
        );
        checked += 1;
    }
    assert_eq!(checked, 16);
}

#[test]
fn derive_vectors_match() {
    let vectors = fixture_json("php-audit/handles/derive-vectors.json");
    let rows = vectors.as_array().expect("an array");
    assert_eq!(rows.len(), 10);
    for row in rows {
        let id = row["id"].as_str().expect("id is a string");
        let project_id = row["projectId"].as_str();
        assert!(project_id.is_some() || row["projectId"].is_null());
        assert_eq!(
            Some(derive_session_id(id, project_id).as_str()),
            row["sessionId"].as_str(),
            "{row}"
        );
    }
    assert_eq!(derive_session_id("x", Some("")), derive_session_id("x", None));
}

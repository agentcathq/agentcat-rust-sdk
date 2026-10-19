//! Golden-file checks against the shared PHP audit fixtures.

use super::support::{fixture_json, fixture_text};
use crate::constants::*;
use crate::handles::{HandleResolution, SessionSource, build_mint_back_text, session_from_hook_value};
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

#[test]
fn exporter_id_vectors_match() {
    use crate::exporters::posthog_uuid_v7;
    use crate::exporters::trace_context::{datadog_span_id, datadog_trace_id, span_id, trace_id};
    let ses = "ses_2yXlShd4ODDcHLYliGW9VgH3m1R";
    let evt = "evt_2yXlTL6bElM5Whm0HjT0Iz8LL0M";
    assert_eq!(trace_id(ses), "b17987c6b00767670c45519276a327c3");
    assert_eq!(span_id(evt), "38fbea6b36e16f37");
    assert_eq!(datadog_trace_id(ses), "884202591345649603");
    assert_eq!(datadog_span_id(evt), "4106133231481483063");
    for (input, expected) in [
        (ses, "01977375-f600-7179-87c6-b00767670c45"),
        (evt, "01977376-0988-78fb-aa6b-36e16f376637"),
        ("my-customer-task-id", "634902a7-d0c7-7220-91c6-0fa56fad8828"),
        ("", "f4c8996f-b924-73b0-8442-98fc1c149afb"),
        (
            "ses_session45600000000000000000",
            "6c350afa-fa75-7081-b0c6-8ad3272daa6c",
        ),
        ("abc_def_ghi", "ae0466b7-1fe3-7984-a344-541280a247bb"),
        // The fixture's "27 dashes" probe: `x_` plus 27 dashes.
        (
            &format!("x_{}", "-".repeat(27)),
            "0145f680-b000-7d7a-8884-df06b0d17ac0",
        ),
    ] {
        assert_eq!(posthog_uuid_v7(input), expected, "{input:?}");
    }
    let (a, b) = (trace_id(""), trace_id(""));
    assert_eq!(a.len(), 32);
    assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
    assert_ne!(a, b);
    let (a, b) = (span_id(""), span_id(""));
    assert_eq!(a.len(), 16);
    assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
    assert_ne!(a, b);
}

#[test]
fn mint_back_blocks_match() {
    let strings = fixture_json("php-audit/handles/strings.json");
    let golden = fixture_text("php-audit/handles/golden.txt");
    let golden_json: serde_json::Value = serde_json::from_str(
        golden
            .split_once("---JSON---\n")
            .and_then(|(_, rest)| rest.split_once("---DERIVE---"))
            .expect("golden.txt has a JSON section")
            .0,
    )
    .expect("the JSON section is JSON");
    let golden_line = |name: &str, value: &str| {
        format!(
            "{name}\tlen_utf16={}\tbytes={}\tsha256={}\temdash={}",
            utf16_len(value),
            value.len(),
            hex(&sha256(value.as_bytes())),
            value.matches('\u{2014}').count()
        )
    };
    let block = |session_id: &str, session_source| {
        build_mint_back_text(&HandleResolution {
            session_id: session_id.to_owned(),
            session_source,
            hook_mode: false,
            agent_id: None,
        })
        .expect("a mint-back block")
    };

    let id = "ses_2cOHEO0LYGADMzRvWTXXVbbgxgm";
    let minted = block(id, SessionSource::Minted);
    assert_eq!(
        Some(minted.as_str()),
        strings[format!("MINTED_BLOCK_FOR_{id}")].as_str()
    );
    assert_eq!(
        Some(minted.as_str()),
        golden_json["MINTED_BLOCK_EXAMPLE"].as_str()
    );
    assert!(
        golden
            .lines()
            .any(|l| l == golden_line("MINTED_BLOCK_EXAMPLE", &minted)),
        "{}",
        golden_line("MINTED_BLOCK_EXAMPLE", &minted)
    );

    let unrecognized = block("", SessionSource::Invalid);
    assert_eq!(
        Some(unrecognized.as_str()),
        strings["UNRECOGNIZED_BLOCK"].as_str()
    );
    assert_eq!(
        Some(unrecognized.as_str()),
        golden_json["UNRECOGNIZED_BLOCK"].as_str()
    );
    assert!(
        golden
            .lines()
            .any(|l| l == golden_line("UNRECOGNIZED_BLOCK", &unrecognized)),
        "{}",
        golden_line("UNRECOGNIZED_BLOCK", &unrecognized)
    );

    assert_eq!(
        session_from_hook_value(Some(" corr-1 "), Some("proj_1")),
        ("ses_2afjLT0irA6g3tOA5CklOPgcSqQ".to_owned(), SessionSource::Hook)
    );
    assert!(golden.contains(
        r#"sessionFromHookValue(' corr-1 ', 'proj_1'): {"sessionId":"ses_2afjLT0irA6g3tOA5CklOPgcSqQ","sessionSource":"hook"}"#
    ));
}

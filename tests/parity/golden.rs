//! Golden-file checks against the shared PHP audit fixtures.

use super::support::{capture_logs, fixture_json, fixture_text};
use crate::constants::*;
use crate::handles::{
    HandleResolution, SessionSource, build_mint_back_text, build_structured_mint_back,
    session_from_hook_value,
};
use crate::inject::{
    InjectConfig, InjectMemory, Injected, build_injected_list, report_missing_descriptor,
    report_missing_response,
};
use crate::ksuid::derive_session_id;
use crate::util::{hex, sha256, utf16_len};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};

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
    let golden_json: Value = serde_json::from_str(
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

/// The registry of a listing, as a set of names per tool.
fn registry_names(registry: &HashMap<String, Injected>) -> BTreeMap<String, BTreeSet<String>> {
    registry
        .iter()
        .map(|(tool, entry)| {
            let names = entry.names().into_iter().map(str::to_owned).collect();
            (tool.clone(), names)
        })
        .collect()
}

/// A fixture registry: `{tool: [names]}`.
fn fixture_registry(value: &Value) -> BTreeMap<String, BTreeSet<String>> {
    value
        .as_object()
        .expect("a registry object")
        .iter()
        .map(|(tool, names)| {
            let names = names
                .as_array()
                .expect("an array of names")
                .iter()
                .map(|n| n.as_str().expect("a name").to_owned())
                .collect();
            (tool.clone(), names)
        })
        .collect()
}

fn string_set(value: &Value) -> BTreeSet<String> {
    value
        .as_array()
        .expect("an array")
        .iter()
        .map(|n| n.as_str().expect("a string").to_owned())
        .collect()
}

#[test]
fn listing_fixtures_match() {
    let fixtures = fixture_json("php-audit/handles/listing-fixtures.json");
    let input = fixtures["input_tools"].as_array().expect("input tools");
    let configs = [
        ("defaults", (true, false, false, true, None, true)),
        ("agent_tracking", (true, false, true, true, None, true)),
        ("hook_mode_agent_tracking", (true, true, true, true, None, true)),
        ("hook_mode_no_agent", (true, true, false, true, None, true)),
        ("tracing_off", (false, false, false, true, None, true)),
        (
            "context_off_report_missing_off",
            (true, false, false, false, None, false),
        ),
        (
            "custom_context_description",
            (true, false, false, true, Some("Why?"), true),
        ),
    ];
    for (key, (tracing, hook_mode, agent_tracking, context, description, report_missing)) in configs {
        let cfg = InjectConfig::new(
            tracing,
            hook_mode,
            agent_tracking,
            context,
            description,
            report_missing,
        );
        let expected = &fixtures[key];
        let list = build_injected_list(&cfg, &InjectMemory::default(), input);
        assert_eq!(
            serde_json::to_string(&list.tools).unwrap(),
            serde_json::to_string(&expected["tools"]).unwrap(),
            "{key}: tools"
        );
        assert_eq!(
            registry_names(&list.registry),
            fixture_registry(&expected["registry"]),
            "{key}: registry"
        );
        assert_eq!(
            list.output_registry.iter().cloned().collect::<BTreeSet<_>>(),
            string_set(&expected["outputRegistry"]),
            "{key}: output registry"
        );
    }
}

#[test]
fn get_more_tools_descriptor_and_reply_match() {
    let strings = fixture_json("php-audit/handles/strings.json");
    assert_eq!(
        serde_json::to_string(&report_missing_descriptor()).unwrap(),
        serde_json::to_string(&strings["GET_MORE_TOOLS_DESCRIPTOR"]).unwrap()
    );
    let (reply, _) = capture_logs(|| report_missing_response(Some(&json!("abc"))));
    assert_eq!(reply, strings["GET_MORE_TOOLS_RESULT"]);

    // TS logs `args?.context?.length ?? 0`.
    for (context, length) in [
        (Some(json!("abc")), "3"),
        (Some(json!("😀")), "2"),
        (Some(json!([1, 2, 3])), "3"),
        (Some(json!({"length": 5})), "5"),
        (Some(json!({"length": "abc"})), "abc"),
        (Some(json!({"length": null})), "0"),
        (Some(json!(42)), "0"),
        (Some(json!(null)), "0"),
        (None, "0"),
    ] {
        let (_, logs) = capture_logs(|| report_missing_response(context.as_ref()));
        assert_eq!(
            logs,
            [format!("Missing tool reported (context length: {length})")],
            "{context:?}"
        );
    }
}

/// The two `---LISTED` sections of the handles golden file: the listings the
/// TS generator prints, and the registry line of the first.
#[test]
fn golden_listed_sections_match() {
    let golden = fixture_text("php-audit/handles/golden.txt");
    let section = |start: &str, end: &str| {
        golden
            .split_once(&format!("{start}\n"))
            .and_then(|(_, rest)| rest.split_once(end))
            .unwrap_or_else(|| panic!("golden.txt has a {start} section"))
            .0
            .to_owned()
    };
    // `JSON.stringify(value, null, 1)`.
    let pretty = |value: &Value| {
        let mut out = Vec::new();
        let mut ser = serde_json::Serializer::with_formatter(
            &mut out,
            serde_json::ser::PrettyFormatter::with_indent(b" "),
        );
        serde::Serialize::serialize(value, &mut ser).unwrap();
        String::from_utf8(out).unwrap()
    };

    let echo = json!({
        "name": "echo",
        "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]},
        "outputSchema": {
            "type": "object",
            "properties": {"count": {"type": "number"}},
            "required": ["count"],
            "additionalProperties": false
        }
    });
    let cfg = InjectConfig::new(true, false, true, true, None, false);
    let list = build_injected_list(
        &cfg,
        &InjectMemory::default(),
        &[report_missing_descriptor(), echo],
    );
    let names: Vec<&str> = list
        .tools
        .iter()
        .map(|t| t["name"].as_str().expect("a tool name"))
        .collect();
    // TS prints its registry Map and output Set in insertion order, which is
    // the listing order here.
    let registry: Vec<Value> = names
        .iter()
        .filter_map(|name| list.registry.get(*name).map(|e| json!([name, e.names()])))
        .collect();
    let output: Vec<&str> = names
        .iter()
        .copied()
        .filter(|name| list.output_registry.contains(*name))
        .collect();
    assert_eq!(
        format!(
            "{}\nregistry: {} output: {}\n",
            pretty(&Value::Array(list.tools.clone())),
            Value::Array(registry),
            json!(output)
        ),
        section(
            "---LISTED (prompted, agent tracking on, context on)---",
            "---LISTED hook mode + agent---"
        )
    );

    let echo = json!({
        "name": "echo",
        "inputSchema": {"type": "object", "properties": {}},
        "outputSchema": {"type": "object", "properties": {}}
    });
    let cfg = InjectConfig::new(true, true, true, false, None, false);
    let list = build_injected_list(&cfg, &InjectMemory::default(), &[echo]);
    assert_eq!(
        format!("{}\n", Value::Array(list.tools)),
        section("---LISTED hook mode + agent---", "---STRUCTURED---")
    );
}

/// The `---STRUCTURED---` rows of the handles golden file: the structured
/// mint-back and the text block for the resolutions the TS generator builds.
#[test]
fn golden_structured_mint_back_matches() {
    let golden = fixture_text("php-audit/handles/golden.txt");
    let section = golden
        .split_once("---STRUCTURED---\n")
        .expect("golden.txt has a STRUCTURED section")
        .1;
    let mut seen = Vec::new();
    for line in section.lines().filter(|l| !l.is_empty()) {
        let mut parts = line.splitn(4, ' ');
        let source = parts.next().unwrap();
        let hook_mode: bool = parts.next().unwrap().parse().unwrap();
        let structured = parts.next().unwrap();
        let text = parts.next().unwrap();
        let (session_id, session_source, agent_id) = match source {
            "minted" => ("ses_A", SessionSource::Minted, None),
            "supplied" => ("ses_A", SessionSource::Supplied, Some("a|b|c")),
            "invalid" => ("", SessionSource::Invalid, Some("a|b|c")),
            "foreign" => ("", SessionSource::Foreign, None),
            "hook" => ("", SessionSource::Hook, Some("a|b|c")),
            other => panic!("unknown row {other}"),
        };
        let res = HandleResolution {
            session_id: session_id.to_owned(),
            session_source,
            hook_mode,
            agent_id: agent_id.map(str::to_owned),
        };
        // Compare as serialized text, so key order counts and null is None.
        let got_structured = build_structured_mint_back(&res)
            .map_or_else(|| "null".to_owned(), |m| serde_json::to_string(&m).unwrap());
        assert_eq!(got_structured, structured, "{source}: structured");
        let got_text = build_mint_back_text(&res)
            .map_or_else(|| "null".to_owned(), |t| serde_json::to_string(&t).unwrap());
        assert_eq!(got_text, text, "{source}: text");
        seen.push(source);
    }
    assert_eq!(seen, ["minted", "supplied", "invalid", "foreign", "hook"]);
}

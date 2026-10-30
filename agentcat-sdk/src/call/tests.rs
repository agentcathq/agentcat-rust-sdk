//! `call`: the cases of `tests/vectors/call.json` and `tests/vectors/call_cases.json`, property
//! tests of the strip and of the mirror's key order, and tests of the error shapes.
use std::collections::BTreeSet;

use proptest::prelude::*;
use rmcp::model::{CallToolResult, ErrorCode, ErrorData};
use serde_json::{Map, Value, json};

use super::{
    cancelled_error, code_name, error_data_error, mirror_mint_back, prepend_mint_back,
    report_missing, result_error, strip_arguments,
};
use crate::constants::{AGENT_ID_PARAM, SESSION_ID_PARAM};
use crate::inject::{CONTEXT_PARAM, ToolEntry};
use crate::test_support::json::index_keys_first;
use crate::test_support::strategies::{json_value, object_key};
use crate::test_support::{assert_same_json_text, config, vectors};

/// The case name prefixes of `call.json`.
const KINDS: [&str; 5] = [
    "strip",
    "mint-back",
    "structured",
    "result-error",
    "report-missing",
];

/// The kind (name prefix) of the case `name`.
fn kind_of(name: &str) -> &'static str {
    KINDS
        .into_iter()
        .find(|kind| {
            name.strip_prefix(kind)
                .is_some_and(|rest| rest.starts_with(':'))
        })
        .unwrap_or_else(|| panic!("{name}: unknown kind"))
}

/// The JSON object `value`.
fn object(value: &Value) -> &Map<String, Value> {
    value
        .as_object()
        .unwrap_or_else(|| panic!("{value} is not an object"))
}

/// The parameter name `name` as the `'static` string a [`ToolEntry`] holds.
fn injectable(name: &str) -> &'static str {
    [SESSION_ID_PARAM, AGENT_ID_PARAM, CONTEXT_PARAM]
        .into_iter()
        .find(|known| *known == name)
        .unwrap_or_else(|| panic!("{name} is not an injected parameter"))
}

/// The registry entry of tool `name` in a registry, `[[tool, [params]]]`.
fn entry_of(registry: &Value, name: &str) -> Option<ToolEntry> {
    registry
        .as_array()?
        .iter()
        .find(|item| item[0] == name)
        .map(|item| ToolEntry {
            injected: item[1]
                .as_array()
                .unwrap()
                .iter()
                .map(|param| injectable(param.as_str().unwrap()))
                .collect(),
            ..ToolEntry::default()
        })
}

/// The rmcp result of the result `fixture`, after checking that it survives a round trip through
/// rmcp's `CallToolResult` (so the comparisons see every key of the result).
fn result_from_json(fixture: &Value) -> CallToolResult {
    let result: CallToolResult = serde_json::from_value(fixture.clone())
        .unwrap_or_else(|e| panic!("rmcp cannot hold {fixture}: {e}"));
    assert_eq!(
        serde_json::to_value(&result).unwrap(),
        *fixture,
        "rmcp's CallToolResult drops or changes a key"
    );
    result
}

/// Checks the fields of an rmcp result that the fixture result has: `content`, `structuredContent`
/// and `isError`.
fn check_result(result: &CallToolResult, fixture: &Value, context: &str) {
    let actual = serde_json::to_value(result).unwrap();
    for key in ["content", "structuredContent", "isError"] {
        match (actual.get(key), fixture.get(key)) {
            (Some(actual), Some(fixture)) => {
                assert_same_json_text(actual, fixture, &format!("{context}: {key}"));
            }
            (None, None) => {}
            (actual, fixture) => {
                panic!("{context}: {key} is {actual:?}, the fixture has {fixture:?}")
            }
        }
    }
}

/// Checks `strip_arguments` against the arguments the fixture gives for the call `input` (`{name,
/// arguments, registry}`).
fn check_strip(input: &Value, fixture: &Value, context: &str) {
    let name = input["name"].as_str().unwrap();
    let arguments = object(&input["arguments"]);
    let entry = entry_of(&input["registry"], name);
    let actual = strip_arguments(arguments, entry.as_ref(), name);
    assert_same_json_text(&Value::Object(actual), fixture, context);
}

#[test]
fn fixture_cases_hold() {
    let mut seen = BTreeSet::new();
    for case in vectors::cases("call") {
        let name = vectors::name(&case);
        let kind = kind_of(name);
        seen.insert(kind);
        let (input, output) = (&case["input"], &case["output"]);
        match kind {
            "strip" => check_strip(input, output, name),
            "mint-back" => {
                let mut result = result_from_json(&input["result"]);
                if let Some(text) = input["text"].as_str() {
                    prepend_mint_back(&mut result, text.to_owned());
                }
                check_result(&result, output, name);
            }
            "structured" => {
                let mut structured = input.get("structuredContent").cloned();
                mirror_mint_back(&mut structured, object(&input["mint"]).clone());
                match output.get("structuredContent") {
                    Some(fixture) => {
                        assert_same_json_text(structured.as_ref().unwrap(), fixture, name);
                    }
                    None => assert_eq!(structured, None, "{name}"),
                }
            }
            "result-error" => {
                let result = result_from_json(input);
                assert_same_json_text(&Value::Object(result_error(&result)), output, name);
            }
            _ => check_report_missing(&case, name),
        }
    }
    assert_eq!(
        seen,
        BTreeSet::from(KINDS),
        "call.json must have cases of every kind"
    );
}

/// A "report-missing:" case: the result and the log line of `report_missing`. The line counts
/// the UTF-16 units of a string context, the items of an array, and 0 for any other value: the
/// `object-length-*` cases pin that an object with a `length` key counts 0 as well.
fn check_report_missing(case: &Map<String, Value>, name: &str) {
    let (result, line) = report_missing(case["input"].get("context"));
    let expected = &case["output"];
    check_result(&result, &expected["value"], name);
    let logs: Vec<&str> = expected["logs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|log| log.as_str().unwrap())
        .collect();
    assert_eq!(vec![line.as_str()], logs, "{name}");
}

/// The wire order of the mirrored `structured_content` is the fixture's, also in the
/// `JSON.stringify` text of each `structured:` case: `assert_same_json_text` sorts index keys
/// itself and cannot see this.
#[test]
fn mirror_wire_order_keeps_the_index_keys_first() {
    let mut checked = 0;
    for case in vectors::cases("call") {
        let name = vectors::name(&case);
        if kind_of(name) != "structured" {
            continue;
        }
        let input = &case["input"];
        let mut structured = input.get("structuredContent").cloned();
        mirror_mint_back(&mut structured, object(&input["mint"]).clone());
        if let Some(expected_text) = case.get("output_text") {
            let actual = serde_json::to_string(structured.as_ref().unwrap()).unwrap();
            assert_eq!(&Value::String(actual), expected_text, "{name}");
            checked += 1;
        } else {
            assert_eq!(structured, None, "{name}");
        }
    }
    assert!(
        checked > 30,
        "only {checked} structured cases have a wire text"
    );
}

/// The JSON-RPC code names.
#[test]
fn code_names_are_the_json_rpc_names() {
    for (code, name) in [
        (ErrorCode::PARSE_ERROR, "ParseError"),
        (ErrorCode::INVALID_REQUEST, "InvalidRequest"),
        (ErrorCode::METHOD_NOT_FOUND, "MethodNotFound"),
        (ErrorCode::INVALID_PARAMS, "InvalidParams"),
        (ErrorCode::INTERNAL_ERROR, "InternalError"),
        (ErrorCode::RESOURCE_NOT_FOUND, "ResourceNotFound"),
        (ErrorCode(-32000), "JsonRpcError(-32000)"),
        (ErrorCode(0), "JsonRpcError(0)"),
        (ErrorCode(42), "JsonRpcError(42)"),
        (ErrorCode(i32::MIN), "JsonRpcError(-2147483648)"),
    ] {
        assert_eq!(code_name(code), name);
    }
}

/// An `ErrorData` gives `{message, type: <code name>, platform: "rust"}`, in that key order, and
/// its `data` stays out.
#[test]
fn error_data_error_has_message_type_and_platform() {
    let error = ErrorData::new(
        ErrorCode::INVALID_PARAMS,
        "bad input",
        Some(json!({"k": 1})),
    );
    let shape = Value::Object(error_data_error(&error));
    assert_eq!(
        serde_json::to_string(&shape).unwrap(),
        r#"{"message":"bad input","type":"InvalidParams","platform":"rust"}"#
    );
    let odd = ErrorData::new(ErrorCode(-32050), "", None);
    assert_eq!(
        Value::Object(error_data_error(&odd)),
        json!({"message": "", "type": "JsonRpcError(-32050)", "platform": "rust"})
    );
}

/// The error of a call whose future was dropped before it completed is `{message, type:
/// "cancelled", platform: "rust"}`, in that key order.
#[test]
fn cancelled_error_has_message_type_and_platform() {
    let shape = Value::Object(cancelled_error());
    assert_eq!(
        serde_json::to_string(&shape).unwrap(),
        r#"{"message":"tool call cancelled before it completed","type":"cancelled","platform":"rust"}"#
    );
}

/// An error result without text has the message `Unknown error`, and one with text has no `type`
/// (`type` is absent).
#[test]
fn result_error_has_no_type() {
    let result = CallToolResult::error(vec![]);
    let shape = result_error(&result);
    assert_eq!(
        Value::Object(shape),
        json!({"message": "Unknown error", "platform": "rust"})
    );
}

/// The cases of `call_cases.json`: `strip:` cases, the arguments a handler gets from random
/// arguments with and without a registry entry, and `structured:` cases, the structured content
/// of a result after the mirror. The check compares the JSON text, so the key order counts.
#[test]
fn strip_and_mirror_match_call_cases() {
    let mut seen = BTreeSet::new();
    for case in vectors::cases("call_cases") {
        let name = vectors::name(&case);
        let kind = kind_of(name);
        seen.insert(kind);
        let input = &case["input"];
        let actual = match kind {
            "strip" => {
                let tool = input["name"].as_str().unwrap();
                let entry = entry_of(&input["registry"], tool);
                Value::Object(strip_arguments(
                    object(&input["arguments"]),
                    entry.as_ref(),
                    tool,
                ))
            }
            "structured" => {
                let mut structured = input.get("structuredContent").cloned();
                mirror_mint_back(&mut structured, object(&input["mint"]).clone());
                let mut result = Map::new();
                if let Some(structured) = structured {
                    result.insert("structuredContent".to_owned(), structured);
                }
                Value::Object(result)
            }
            other => panic!("{name}: call_cases.json has no {other} cases"),
        };
        assert_eq!(
            serde_json::to_string(&actual).unwrap(),
            serde_json::to_string(&case["output"]).unwrap(),
            "{name}"
        );
    }
    assert_eq!(
        seen,
        BTreeSet::from(["strip", "structured"]),
        "call_cases.json has cases of both kinds"
    );
}

fn arg_key() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => prop::sample::select(vec![SESSION_ID_PARAM, AGENT_ID_PARAM, CONTEXT_PARAM]).prop_map(str::to_owned),
        2 => object_key(),
    ]
}

proptest! {
    #![proptest_config(config())]

    /// With a registry entry, the strip removes the names the entry recorded; without one, all
    /// three injected names, or only `session_id` and `agent_id` for `get_more_tools`. Every
    /// other argument keeps its value and its place.
    #[test]
    fn strip_removes_only_the_injected_names_and_keeps_the_order(
        entries in prop::collection::vec((arg_key(), json_value(1, 2)), 0..=6),
        tool in prop::sample::select(vec!["get_more_tools", "t"]),
        registry in prop_oneof![
            Just(None),
            prop::sample::subsequence(vec![SESSION_ID_PARAM, AGENT_ID_PARAM, CONTEXT_PARAM], 0..=3).prop_map(|p| Some(("t", p))),
            prop::sample::subsequence(vec![SESSION_ID_PARAM, AGENT_ID_PARAM, CONTEXT_PARAM], 0..=3).prop_map(|p| Some(("get_more_tools", p))),
            prop::sample::subsequence(vec![SESSION_ID_PARAM, AGENT_ID_PARAM, CONTEXT_PARAM], 0..=3).prop_map(|p| Some(("other", p))),
        ],
    ) {
        let arguments: Map<String, Value> = entries.into_iter().collect();
        let removed: Vec<&str> = match &registry {
            Some((name, params)) if *name == tool => params.clone(),
            _ if tool == "get_more_tools" => vec![SESSION_ID_PARAM, AGENT_ID_PARAM],
            _ => vec![SESSION_ID_PARAM, AGENT_ID_PARAM, CONTEXT_PARAM],
        };
        let registry = registry.map_or(Value::Null, |(name, params)| json!([[name, params]]));
        let entry = entry_of(&registry, tool);
        let actual = strip_arguments(&arguments, entry.as_ref(), tool);
        let expected: Vec<(&String, &Value)> = arguments
            .iter()
            .filter(|(key, _)| !removed.contains(&key.as_str()))
            .collect();
        prop_assert_eq!(actual.iter().collect::<Vec<_>>(), expected);
    }

    /// The mirror gives an object without its own `mcp_session` key that key, with the mint as
    /// its value, after the array-index keys (ascending) and before the other keys (in their
    /// order); the values stay. Any other structured content stays as it is, key order included.
    #[test]
    fn mirror_puts_mcp_session_after_the_index_keys(
        structured in prop_oneof![
            3 => prop::collection::vec((object_key(), json_value(1, 2)), 0..=6)
                .prop_map(|entries| Some(Value::Object(entries.into_iter().collect()))),
            1 => json_value(1, 3).prop_map(Some),
            1 => Just(None),
        ],
        own in any::<bool>(),
        mint_keys in prop::sample::subsequence(vec!["session_id", "agent_id", "status"], 1..=3),
    ) {
        let mut structured = structured;
        if own && let Some(Value::Object(body)) = &mut structured {
            body.insert("mcp_session".to_owned(), json!("customer"));
        }
        let mint: Map<String, Value> = mint_keys.into_iter().map(|key| (key.to_owned(), json!(format!("<{key}>")))).collect();
        let before = structured.clone();
        mirror_mint_back(&mut structured, mint.clone());
        match (&before, &structured) {
            (Some(Value::Object(body)), Some(Value::Object(mirrored)))
                if !body.contains_key("mcp_session") =>
            {
                let order = index_keys_first(
                    std::iter::once("mcp_session").chain(body.keys().map(String::as_str)),
                );
                let keys: Vec<&str> = mirrored.keys().map(String::as_str).collect();
                prop_assert_eq!(keys, order);
                prop_assert_eq!(&mirrored["mcp_session"], &Value::Object(mint));
                for (key, value) in body {
                    prop_assert_eq!(&mirrored[key], value);
                }
            }
            _ => prop_assert_eq!(
                serde_json::to_string(&structured).unwrap(),
                serde_json::to_string(&before).unwrap()
            ),
        }
    }
}

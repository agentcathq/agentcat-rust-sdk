//! `call` against the TS SDK: the cases of `tests/vectors/call.json`, property tests that run
//! random arguments and structured content through the JS oracle, and Rust-only tests of the
//! error shapes (RD-05, RD-06, C24).
use std::collections::BTreeSet;

use proptest::prelude::*;
use rmcp::model::{CallToolResult, ErrorCode, ErrorData};
use serde_json::{Map, Value, json};

use super::{
    code_name, error_data_error, mirror_mint_back, prepend_mint_back, report_missing, result_error,
    strip_arguments,
};
use crate::constants::{AGENT_ID_PARAM, SESSION_ID_PARAM, report_missing_log_line};
use crate::inject::{CONTEXT_PARAM, ToolEntry};
use crate::test_support::strategies::{json_value, object_key};
use crate::test_support::{assert_js_eq, config, oracle, vectors};

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

/// The registry entry of tool `name` in a TS registry, `[[tool, [params]]]`.
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

/// The rmcp result of the TS result `ts`, after checking that it survives a round trip through
/// rmcp's `CallToolResult` (so the comparisons see every key of the TS result).
fn result_from_ts(ts: &Value) -> CallToolResult {
    let result: CallToolResult =
        serde_json::from_value(ts.clone()).unwrap_or_else(|e| panic!("rmcp cannot hold {ts}: {e}"));
    assert_eq!(
        serde_json::to_value(&result).unwrap(),
        *ts,
        "rmcp's CallToolResult drops or changes a key"
    );
    result
}

/// Checks the fields of an rmcp result that TS's result has: `content`, `structuredContent` and
/// `isError`.
fn check_result(result: &CallToolResult, ts: &Value, context: &str) {
    let actual = serde_json::to_value(result).unwrap();
    for key in ["content", "structuredContent", "isError"] {
        match (actual.get(key), ts.get(key)) {
            (Some(actual), Some(ts)) => assert_js_eq(actual, ts, &format!("{context}: {key}")),
            (None, None) => {}
            (actual, ts) => panic!("{context}: {key} is {actual:?}, TS {ts:?}"),
        }
    }
}

/// Checks `strip_arguments` against TS's arguments for the call `input` (`{name, arguments,
/// registry}`).
fn check_strip(input: &Value, ts: &Value, context: &str) {
    let name = input["name"].as_str().unwrap();
    let arguments = object(&input["arguments"]);
    let entry = entry_of(&input["registry"], name);
    let actual = strip_arguments(arguments, entry.as_ref(), name);
    assert_js_eq(&Value::Object(actual), ts, context);
}

#[test]
fn vectors_match_ts() {
    let mut seen = BTreeSet::new();
    for case in vectors::cases("call") {
        let name = vectors::name(&case);
        let kind = kind_of(name);
        seen.insert(kind);
        let (input, output) = (&case["input"], &case["output"]);
        match kind {
            "strip" => check_strip(input, output, name),
            "mint-back" => {
                let mut result = result_from_ts(&input["result"]);
                if let Some(text) = input["text"].as_str() {
                    prepend_mint_back(&mut result, text.to_owned());
                }
                check_result(&result, output, name);
            }
            "structured" => {
                let mut structured = input.get("structuredContent").cloned();
                mirror_mint_back(&mut structured, object(&input["mint"]).clone());
                match output.get("structuredContent") {
                    Some(ts) => assert_js_eq(structured.as_ref().unwrap(), ts, name),
                    None => assert_eq!(structured, None, "{name}"),
                }
            }
            "result-error" => {
                let result = result_from_ts(input);
                let mut ts = output.clone();
                // RD-06: the platform is "rust".
                assert_eq!(ts["platform"], "javascript", "{name}");
                ts["platform"] = json!("rust");
                assert_js_eq(&Value::Object(result_error(&result)), &ts, name);
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

/// A "report-missing:" case: the result and the log line of `report_missing`. A case with
/// deviation RD-26 is one where TS reads the `length` of an object (R23): the test asserts
/// Rust's line, which counts 0, and that it is not TS's.
fn check_report_missing(case: &Map<String, Value>, name: &str) {
    let (result, line) = report_missing(case["input"].get("context"));
    let ts = &case["output"];
    check_result(&result, &ts["value"], name);
    let ts_logs: Vec<&str> = ts["logs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|log| log.as_str().unwrap())
        .collect();
    match case.get("deviation").and_then(Value::as_str) {
        None => assert_eq!(vec![line.as_str()], ts_logs, "{name}"),
        Some(deviation) => {
            assert_eq!(deviation, "RD-26", "{name}");
            assert_eq!(line, report_missing_log_line(0), "{name}: Rust counts 0");
            assert_ne!(vec![line.as_str()], ts_logs, "{name}: TS counts the key");
        }
    }
}

/// The wire order of the mirrored `structured_content` is TS's (C21), also in the `JSON.stringify`
/// text of each `structured:` case: `assert_js_eq` sorts index keys itself and cannot see this.
#[test]
fn mirror_wire_order_matches_ts() {
    let mut checked = 0;
    for case in vectors::cases("call") {
        let name = vectors::name(&case);
        if kind_of(name) != "structured" {
            continue;
        }
        let input = &case["input"];
        let mut structured = input.get("structuredContent").cloned();
        mirror_mint_back(&mut structured, object(&input["mint"]).clone());
        if let Some(ts_text) = case.get("output_text") {
            let actual = serde_json::to_string(structured.as_ref().unwrap()).unwrap();
            assert_eq!(&Value::String(actual), ts_text, "{name}");
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

/// C24: the JSON-RPC code names.
#[test]
fn code_names_follow_c24() {
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

/// RD-05 and RD-06: an `ErrorData` gives `{message, type: <code name>, platform: "rust"}`, in that
/// key order, and its `data` stays out.
#[test]
fn error_data_error_has_the_rd06_shape() {
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

/// RD-06: an error result without text has the TS message `Unknown error`, and one with text has
/// no `type` (TS's is `undefined`).
#[test]
fn result_error_has_no_type() {
    let result = CallToolResult::error(vec![]);
    let shape = result_error(&result);
    assert_eq!(
        Value::Object(shape),
        json!({"message": "Unknown error", "platform": "rust"})
    );
}

/// The TS arguments of a random call, from the oracle: `{name, arguments, registry}`.
fn ts_strip(input: &Value) -> Value {
    oracle::call("oracle.strip", std::slice::from_ref(input))
        .unwrap()
        .unwrap()
}

fn arg_key() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => prop::sample::select(vec![SESSION_ID_PARAM, AGENT_ID_PARAM, CONTEXT_PARAM]).prop_map(str::to_owned),
        2 => object_key(),
    ]
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn strip_matches_ts(
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
        let registry = registry.map_or(Value::Null, |(name, params)| json!([[name, params]]));
        let input = json!({"name": tool, "arguments": arguments, "registry": registry});
        let ts = ts_strip(&input);
        check_strip(&input, &ts, &input.to_string());
    }

    #[test]
    fn mirror_matches_ts(
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
        let mut result = json!({"content": []});
        if let Some(structured) = &structured {
            result["structuredContent"] = structured.clone();
        }
        let ts = oracle::call(
            "handles.mirrorStructuredMintBack",
            &[result, Value::Object(mint.clone())],
        ).unwrap().unwrap();
        mirror_mint_back(&mut structured, mint);
        let context = format!("{structured:?}");
        match (&structured, ts.get("structuredContent")) {
            (Some(actual), Some(ts)) => {
                assert_js_eq(actual, ts, &context);
                // An object that is left alone keeps its order (R23: TS's index-first order of a
                // customer's own keys is a JavaScript artifact); a mirrored one has C21's.
                if let (false, Value::Object(actual), Value::Object(ts)) = (own, actual, ts) {
                    let actual: Vec<&String> = actual.keys().collect();
                    let ts: Vec<&String> = ts.keys().collect();
                    assert_eq!(actual, ts, "{context}: key order");
                }
            }
            (None, None) => {}
            (actual, ts) => panic!("{context}: {actual:?}, TS {ts:?}"),
        }
    }
}

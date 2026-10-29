//! `inject` against the TS SDK: the cases of `tests/vectors/inject.json` (each page listed in turn
//! on one [`InjectState`], as TS lists them on one tracking-data object), a property test that runs
//! random listings through the JS oracle, and a Rust-only test of the shared schemas.
use std::collections::{BTreeMap, BTreeSet};

use proptest::prelude::*;
use rmcp::model::Tool;
use serde_json::{Map, Value, json};

use super::{
    InjectConfig, InjectState, InjectedList, build_injected_list, get_more_tools_tool, inject_tool,
};
use crate::config::Options;
use crate::constants::GET_MORE_TOOLS_NAME;
use crate::test_support::strategies::{js_string, tool_list};
use crate::test_support::{assert_js_eq, config, oracle, vectors};

/// The case name prefixes of `inject.json`.
const KINDS: [&str; 8] = [
    "scenario",
    "flags",
    "schema",
    "output",
    "report-missing",
    "pages",
    "unrepresentable",
    "seeded",
];

/// The keys of the TS options of a case.
const OPTION_KEYS: [&str; 5] = [
    "enableReportMissing",
    "enableTracing",
    "enableToolCallContext",
    "enableAgentTracking",
    "customContextDescription",
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

/// The [`Options`] of the TS options `ts`: the four switches, and the context description when
/// `ts` has one.
fn options_from_ts(ts: &Value) -> Options {
    let ts = ts.as_object().unwrap_or_else(|| panic!("options {ts}"));
    for key in ts.keys() {
        assert!(OPTION_KEYS.contains(&key.as_str()), "unknown option {key}");
    }
    let flag = |key: &str| {
        ts.get(key)
            .and_then(Value::as_bool)
            .unwrap_or_else(|| panic!("option {key} of {ts:?} is not a boolean"))
    };
    let options = Options::default()
        .enable_report_missing(flag("enableReportMissing"))
        .enable_tracing(flag("enableTracing"))
        .enable_tool_call_context(flag("enableToolCallContext"))
        .enable_agent_tracking(flag("enableAgentTracking"));
    match ts.get("customContextDescription") {
        None => options,
        Some(text) => options.custom_context_description(text.as_str().unwrap()),
    }
}

/// The [`InjectConfig`] of an `{options, hookMode}` input (Ruling P1: [`InjectConfig::new`] with
/// the case's hook mode). Without hook mode it is also what [`InjectConfig::from_options`] gives.
fn config_of(input: &Value) -> InjectConfig {
    let options = options_from_ts(&input["options"]);
    let hook_mode = input["hookMode"].as_bool().unwrap();
    let config = InjectConfig::new(&options, hook_mode);
    if !hook_mode {
        assert_eq!(InjectConfig::from_options(&options), config, "{input}");
    }
    config
}

/// The rmcp tool of the TS tool `ts`, after checking that the tool survives a round trip through
/// rmcp's `Tool` (so the comparisons below see every key of the TS output).
fn tool_from_ts(ts: &Value) -> Tool {
    let tool: Tool =
        serde_json::from_value(ts.clone()).unwrap_or_else(|e| panic!("rmcp cannot hold {ts}: {e}"));
    assert_eq!(
        serde_json::to_value(&tool).unwrap(),
        *ts,
        "rmcp's Tool drops or changes a key"
    );
    tool
}

/// The pages of an input, as rmcp tools.
fn pages_of(input: &Value) -> Vec<Vec<Tool>> {
    input["pages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|page| page.as_array().unwrap().iter().map(tool_from_ts).collect())
        .collect()
}

/// The string items of the JSON array `value`.
fn strings(value: &Value) -> Vec<&str> {
    value
        .as_array()
        .unwrap_or_else(|| panic!("{value} is not an array"))
        .iter()
        .map(|item| {
            item.as_str()
                .unwrap_or_else(|| panic!("{item} is not a string"))
        })
        .collect()
}

/// TS's registry of a page, `[[name, [params]]]`, as a map.
fn ts_registry(page: &Value) -> BTreeMap<&str, Vec<&str>> {
    let registry = page["registry"].as_array().unwrap();
    let map: BTreeMap<&str, Vec<&str>> = registry
        .iter()
        .map(|entry| (entry[0].as_str().unwrap(), strings(&entry[1])))
        .collect();
    assert_eq!(map.len(), registry.len(), "TS registry names repeat");
    map
}

/// Checks one tool against TS's: the schemas as `JSON.stringify` writes them (key order counts)
/// and the whole tool with key order ignored.
fn check_tool(tool: &Tool, ts: &Value, context: &str) {
    let actual = serde_json::to_value(tool).unwrap();
    assert_js_eq(
        &actual["inputSchema"],
        &ts["inputSchema"],
        &format!("{context}: inputSchema"),
    );
    match (actual.get("outputSchema"), ts.get("outputSchema")) {
        (Some(actual), Some(ts)) => {
            assert_js_eq(actual, ts, &format!("{context}: outputSchema"));
        }
        (None, None) => {}
        (actual, ts) => panic!("{context}: outputSchema {actual:?}, TS {ts:?}"),
    }
    assert_eq!(actual, *ts, "{context}: the tool (key order ignored)");
}

/// Checks the list of one page against TS's: tool names in order, each tool, the registry (map
/// order ignored, parameter order compared), the output registry and the declared set as sets,
/// and the log lines in order. `declared_before` is the declared set before the page: the names
/// the page adds are the entries that declare `session_id`.
fn check_page(
    list: &InjectedList,
    state: &InjectState,
    declared_before: &BTreeSet<String>,
    ts: &Value,
    context: &str,
) {
    let ts_tools = ts["tools"].as_array().unwrap();
    let names: Vec<&str> = list.tools.iter().map(|tool| tool.name.as_ref()).collect();
    let ts_names: Vec<&str> = ts_tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ts_names, "{context}: tool names");
    for (i, (tool, ts_tool)) in list.tools.iter().zip(ts_tools).enumerate() {
        check_tool(tool, ts_tool, &format!("{context}: tool {i}"));
    }

    let registry: BTreeMap<&str, Vec<&str>> = list
        .entries
        .iter()
        .map(|(name, entry)| (name.as_str(), entry.injected.clone()))
        .collect();
    assert_eq!(
        registry.len(),
        list.entries.len(),
        "{context}: entry names repeat"
    );
    assert_eq!(registry, ts_registry(ts), "{context}: registry");

    let output: BTreeSet<&str> = list
        .entries
        .iter()
        .filter(|(_, entry)| entry.output_injected)
        .map(|(name, _)| name.as_str())
        .collect();
    let ts_output: BTreeSet<&str> = strings(&ts["outputRegistry"]).into_iter().collect();
    assert_eq!(output, ts_output, "{context}: output registry");

    let declared: BTreeSet<&str> = state
        .declared_session_params
        .iter()
        .map(String::as_str)
        .collect();
    let ts_declared: BTreeSet<&str> = strings(&ts["declaredSessionParams"]).into_iter().collect();
    assert_eq!(declared, ts_declared, "{context}: declared session params");
    let mut expected_declared = declared_before.clone();
    for (name, entry) in &list.entries {
        if entry.declares_session {
            expected_declared.insert(name.clone());
        }
    }
    assert_eq!(
        expected_declared, state.declared_session_params,
        "{context}: the entries that declare session_id"
    );

    let logs: Vec<&str> = list.logs.iter().map(String::as_str).collect();
    assert_eq!(logs, strings(&ts["logs"]), "{context}: logs");
}

/// Lists `pages` in turn on one [`InjectState`] and checks each list against TS's.
fn check_pages(config: &InjectConfig, pages: Vec<Vec<Tool>>, ts: &[Value], context: &str) {
    assert_eq!(pages.len(), ts.len(), "{context}: page count");
    let mut state = InjectState::default();
    for (i, (tools, ts_page)) in pages.into_iter().zip(ts).enumerate() {
        let before = state.declared_session_params.clone();
        let list = build_injected_list(config, tools, &mut state);
        check_page(
            &list,
            &state,
            &before,
            ts_page,
            &format!("{context}: page {i}"),
        );
    }
}

/// An "unrepresentable:" case: one tool on one page, which rmcp's `Tool` cannot hold (RD-20).
fn check_rd20(case: &Map<String, Value>, name: &str) {
    let pages = case["input"]["pages"].as_array().unwrap();
    let [page] = pages.as_slice() else {
        panic!("{name}: an unrepresentable case has one page");
    };
    let [tool] = page.as_array().unwrap().as_slice() else {
        panic!("{name}: an unrepresentable case has one tool");
    };
    assert!(
        serde_json::from_value::<Tool>(tool.clone()).is_err(),
        "{name}: rmcp's Tool holds {tool}"
    );
}

#[test]
fn vectors_match_ts() {
    let mut seen = BTreeSet::new();
    for case in vectors::cases("inject") {
        let name = vectors::name(&case);
        let kind = kind_of(name);
        seen.insert(kind);
        let deviation = case.get("deviation").and_then(Value::as_str);
        assert_eq!(
            kind == "unrepresentable",
            deviation == Some("RD-20"),
            "{name}: the unrepresentable cases, and only they, carry RD-20"
        );
        if kind == "unrepresentable" {
            check_rd20(&case, name);
            continue;
        }
        let input = &case["input"];
        let output = case["output"].as_array().unwrap();
        check_pages(&config_of(input), pages_of(input), output, name);
    }
    assert_eq!(
        seen,
        BTreeSet::from(KINDS),
        "inject.json must have cases of every kind"
    );
}

/// [`inject_tool`] gives a tool what the listing gives it: on the first page of each case (so the
/// state is fresh, as it is for TS's first listing), each listed tool whose registry name no other
/// tool there shares is injected alone and compared with TS's tool and registry records. A page of
/// one tool, with no `get_more_tools` shadow warning, also has TS's log lines.
#[test]
fn inject_tool_matches_ts() {
    let mut checked = 0;
    for case in vectors::cases("inject") {
        let name = vectors::name(&case);
        if kind_of(name) == "unrepresentable" {
            continue;
        }
        let input = &case["input"];
        let config = config_of(input);
        let tools = pages_of(input).swap_remove(0);
        let ts = &case["output"][0];
        let shadowed =
            config.report_missing && tools.iter().any(|tool| tool.name == GET_MORE_TOOLS_NAME);
        let mut listed = tools;
        if config.report_missing && !shadowed {
            listed.push(get_more_tools_tool());
        }
        let key = |tool: &Tool| {
            if tool.name.is_empty() {
                "unknown".to_owned()
            } else {
                tool.name.to_string()
            }
        };
        let keys: Vec<String> = listed.iter().map(key).collect();
        let registry = ts_registry(ts);
        let output = strings(&ts["outputRegistry"]);
        let declared = strings(&ts["declaredSessionParams"]);
        let single = listed.len() == 1 && !shadowed;
        for (i, tool) in listed.into_iter().enumerate() {
            let key = &keys[i];
            if keys.iter().filter(|other| *other == key).count() > 1 {
                continue;
            }
            let mut state = InjectState::default();
            let mut logs = Vec::new();
            let (tool, entry) = inject_tool(&config, tool, &mut state, &mut logs);
            let context = format!("{name}: tool {i}");
            check_tool(&tool, &ts["tools"][i], &context);
            let params = registry.get(key.as_str()).cloned().unwrap_or_default();
            assert_eq!(entry.injected, params, "{context}: injected");
            assert_eq!(
                entry.output_injected,
                output.contains(&key.as_str()),
                "{context}: output"
            );
            assert_eq!(
                entry.declares_session,
                declared.contains(&key.as_str()),
                "{context}: declares session_id"
            );
            if single {
                assert_eq!(logs, strings(&ts["logs"]), "{context}: logs");
            }
            checked += 1;
        }
    }
    assert!(checked > 300, "only {checked} tools checked");
}

#[test]
fn get_more_tools_tool_is_the_ts_descriptor() {
    let descriptor = vectors::cases("constants")
        .into_iter()
        .find(|case| vectors::name(case) == "report_missing_tool_descriptor")
        .and_then(|mut case| case.remove("value"))
        .unwrap();
    let tool = serde_json::to_value(get_more_tools_tool()).unwrap();
    assert_js_eq(&tool, &descriptor, "get_more_tools");
}

/// The TS tests "does not mutate the customer's outputSchema object" (handle-injection) and "does
/// not mutate the input tool objects" (engine-listwrap): the pipeline edits schemas with
/// `Arc::make_mut`, so a clone of a listed tool that shares its schemas keeps them as they were,
/// also for a tool whose injection fails (B-044).
#[test]
fn shared_schemas_stay_unchanged() {
    let tools: Vec<Tool> = [
        json!({
            "name": "stats",
            "inputSchema": {"type": "object", "properties": {"q": {"type": "string"}}, "additionalProperties": false},
            "outputSchema": {"type": "object", "properties": {"count": {"type": "number"}}},
        }),
        json!({
            "name": "failing",
            "inputSchema": {"type": "object", "properties": {}},
            "outputSchema": {"type": "object", "properties": "x"},
        }),
    ]
    .into_iter()
    .map(|tool| serde_json::from_value(tool).unwrap())
    .collect();
    let kept = tools.clone();
    let before: Vec<Value> = kept
        .iter()
        .map(|tool| serde_json::to_value(tool).unwrap())
        .collect();
    // The context step stays off: it runs after a failed handle step, as in TS, and would give
    // `failing` its `context`.
    let options = Options::default()
        .enable_agent_tracking(true)
        .enable_tool_call_context(false);
    let mut state = InjectState::default();
    let list = build_injected_list(&InjectConfig::from_options(&options), tools, &mut state);
    assert_ne!(list.tools[0], kept[0], "the pipeline injects into stats");
    assert_eq!(list.tools[1], kept[1], "failing is listed as it came");
    let after: Vec<Value> = kept
        .iter()
        .map(|tool| serde_json::to_value(tool).unwrap())
        .collect();
    assert_eq!(after, before);
}

/// TS options with every switch random, and sometimes a context description.
fn ts_options() -> impl Strategy<Value = Value> {
    let description = prop_oneof![
        2 => Just(None),
        1 => Just(Some(String::new())),
        1 => Just(Some("why".to_owned())),
        1 => js_string(8).prop_map(Some),
    ];
    (
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        description,
    )
        .prop_map(
            |(report_missing, tracing, context, agent_tracking, description)| {
                let mut options = json!({
                    "enableReportMissing": report_missing,
                    "enableTracing": tracing,
                    "enableToolCallContext": context,
                    "enableAgentTracking": agent_tracking,
                });
                if let Some(description) = description {
                    options["customContextDescription"] = json!(description);
                }
                options
            },
        )
}

proptest! {
    #![proptest_config(config())]

    #[test]
    fn inject_matches_ts(
        options in ts_options(),
        hook_mode in any::<bool>(),
        pages in prop::collection::vec(tool_list(), 1..=2),
    ) {
        let input = json!({
            "options": options,
            "hookMode": hook_mode,
            "pages": serde_json::to_value(&pages).unwrap(),
        });
        let ts = oracle::call("oracle.buildInjectedList", std::slice::from_ref(&input)).unwrap().unwrap();
        let ts = ts.as_array().unwrap();
        check_pages(&config_of(&input), pages, ts, &input.to_string());
    }
}

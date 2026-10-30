//! `inject`: the cases of `tests/vectors/inject.json` and `tests/vectors/inject_cases.json` (each
//! page listed in turn on one [`InjectState`]), property tests of the rules of a listing, and a
//! test of the shared schemas.
use std::collections::{BTreeMap, BTreeSet};

use proptest::prelude::*;
use rmcp::model::Tool;
use serde_json::{Map, Value, json};

use super::{
    CONTEXT_PARAM, InjectConfig, InjectState, InjectedList, build_injected_list,
    get_more_tools_tool, inject_tool,
};
use crate::config::Options;
use crate::constants::{AGENT_ID_PARAM, GET_MORE_TOOLS_NAME, SESSION_ID_PARAM};
use crate::test_support::strategies::{edge_string, tool_list};
use crate::test_support::{assert_same_json_text, config, vectors};

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

/// The keys of the options of a case.
const OPTION_KEYS: [&str; 5] = [
    "enable_report_missing",
    "enable_tracing",
    "enable_tool_call_context",
    "enable_agent_tracking",
    "custom_context_description",
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

/// The [`Options`] of the options `fixture`: the four switches, and the context description when
/// `fixture` has one.
fn options_from_json(fixture: &Value) -> Options {
    let fixture = fixture
        .as_object()
        .unwrap_or_else(|| panic!("options {fixture}"));
    for key in fixture.keys() {
        assert!(OPTION_KEYS.contains(&key.as_str()), "unknown option {key}");
    }
    let flag = |key: &str| {
        fixture
            .get(key)
            .and_then(Value::as_bool)
            .unwrap_or_else(|| panic!("option {key} of {fixture:?} is not a boolean"))
    };
    let options = Options::default()
        .enable_report_missing(flag("enable_report_missing"))
        .enable_tracing(flag("enable_tracing"))
        .enable_tool_call_context(flag("enable_tool_call_context"))
        .enable_agent_tracking(flag("enable_agent_tracking"));
    match fixture.get("custom_context_description") {
        None => options,
        Some(text) => options.custom_context_description(text.as_str().unwrap()),
    }
}

/// The [`InjectConfig`] of an `{options, hookMode}` input ([`InjectConfig::new`] with the case's
/// hook mode). Without hook mode it is also what [`InjectConfig::from_options`] gives.
fn config_of(input: &Value) -> InjectConfig {
    let options = options_from_json(&input["options"]);
    let hook_mode = input["hookMode"].as_bool().unwrap();
    let config = InjectConfig::new(&options, hook_mode);
    if !hook_mode {
        assert_eq!(InjectConfig::from_options(&options), config, "{input}");
    }
    config
}

/// The rmcp tool of the tool `fixture`, after checking that the tool survives a round trip through
/// rmcp's `Tool` (so the comparisons below see every key of the output).
fn tool_from_json(fixture: &Value) -> Tool {
    let tool: Tool = serde_json::from_value(fixture.clone())
        .unwrap_or_else(|e| panic!("rmcp cannot hold {fixture}: {e}"));
    assert_eq!(
        serde_json::to_value(&tool).unwrap(),
        *fixture,
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
        .map(|page| {
            page.as_array()
                .unwrap()
                .iter()
                .map(tool_from_json)
                .collect()
        })
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

/// The registry of a page, `[[name, [params]]]`, as a map.
fn expected_registry(page: &Value) -> BTreeMap<&str, Vec<&str>> {
    let registry = page["registry"].as_array().unwrap();
    let map: BTreeMap<&str, Vec<&str>> = registry
        .iter()
        .map(|entry| (entry[0].as_str().unwrap(), strings(&entry[1])))
        .collect();
    assert_eq!(map.len(), registry.len(), "registry names repeat");
    map
}

/// Checks one tool against the fixture's: the schemas as `JSON.stringify` writes them (key order
/// counts) and the whole tool with key order ignored.
fn check_tool(tool: &Tool, fixture: &Value, context: &str) {
    let actual = serde_json::to_value(tool).unwrap();
    assert_same_json_text(
        &actual["inputSchema"],
        &fixture["inputSchema"],
        &format!("{context}: inputSchema"),
    );
    match (actual.get("outputSchema"), fixture.get("outputSchema")) {
        (Some(actual), Some(fixture)) => {
            assert_same_json_text(actual, fixture, &format!("{context}: outputSchema"));
        }
        (None, None) => {}
        (actual, fixture) => {
            panic!("{context}: outputSchema {actual:?}, the fixture has {fixture:?}")
        }
    }
    assert_eq!(actual, *fixture, "{context}: the tool (key order ignored)");
}

/// Checks the list of one page against the fixture's: tool names in order, each tool, the registry
/// (map order ignored, parameter order compared), the output registry and the declared set as sets,
/// and the log lines in order. `declared_before` is the declared set before the page: the names the
/// page adds are the entries that declare `session_id`.
fn check_page(
    list: &InjectedList,
    state: &InjectState,
    declared_before: &BTreeSet<String>,
    fixture: &Value,
    context: &str,
) {
    let expected_tools = fixture["tools"].as_array().unwrap();
    let names: Vec<&str> = list.tools.iter().map(|tool| tool.name.as_ref()).collect();
    let expected_names: Vec<&str> = expected_tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, expected_names, "{context}: tool names");
    for (i, (tool, expected_tool)) in list.tools.iter().zip(expected_tools).enumerate() {
        check_tool(tool, expected_tool, &format!("{context}: tool {i}"));
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
    assert_eq!(registry, expected_registry(fixture), "{context}: registry");

    let output: BTreeSet<&str> = list
        .entries
        .iter()
        .filter(|(_, entry)| entry.output_injected)
        .map(|(name, _)| name.as_str())
        .collect();
    let expected_output: BTreeSet<&str> = strings(&fixture["outputRegistry"]).into_iter().collect();
    assert_eq!(output, expected_output, "{context}: output registry");

    let declared: BTreeSet<&str> = state
        .declared_session_params
        .iter()
        .map(String::as_str)
        .collect();
    let expected_declared: BTreeSet<&str> = strings(&fixture["declaredSessionParams"])
        .into_iter()
        .collect();
    assert_eq!(
        declared, expected_declared,
        "{context}: declared session params"
    );
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
    assert_eq!(logs, strings(&fixture["logs"]), "{context}: logs");
}

/// Lists `pages` in turn on one [`InjectState`] and checks each list against the fixture's.
fn check_pages(config: &InjectConfig, pages: Vec<Vec<Tool>>, fixture: &[Value], context: &str) {
    assert_eq!(pages.len(), fixture.len(), "{context}: page count");
    let mut state = InjectState::default();
    for (i, (tools, expected_page)) in pages.into_iter().zip(fixture).enumerate() {
        let before = state.declared_session_params.clone();
        let list = build_injected_list(config, tools, &mut state);
        check_page(
            &list,
            &state,
            &before,
            expected_page,
            &format!("{context}: page {i}"),
        );
    }
}

/// An "unrepresentable:" case: one tool on one page that rmcp's `Tool` rejects, so the listing
/// never gets it and has no branch for it. The case's `error` is the deserialization error.
fn check_unrepresentable(case: &Map<String, Value>, name: &str) {
    let pages = case["input"]["pages"].as_array().unwrap();
    let [page] = pages.as_slice() else {
        panic!("{name}: an unrepresentable case has one page");
    };
    let [tool] = page.as_array().unwrap().as_slice() else {
        panic!("{name}: an unrepresentable case has one tool");
    };
    let Err(error) = serde_json::from_value::<Tool>(tool.clone()) else {
        panic!("{name}: rmcp's Tool holds {tool}");
    };
    assert_eq!(Value::String(error.to_string()), case["error"], "{name}");
    assert!(
        !case.contains_key("output"),
        "{name}: an unrepresentable case holds no output"
    );
}

#[test]
fn fixture_cases_hold() {
    let mut seen = BTreeSet::new();
    for case in vectors::cases("inject") {
        let name = vectors::name(&case);
        let kind = kind_of(name);
        seen.insert(kind);
        assert_eq!(
            kind == "unrepresentable",
            case.contains_key("error"),
            "{name}: the unrepresentable cases, and only they, hold an error"
        );
        if kind == "unrepresentable" {
            check_unrepresentable(&case, name);
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
/// state is fresh, as it is for a first listing), each listed tool whose registry name no other
/// tool there shares is injected alone and compared with the fixture's tool and registry records. A
/// page of one tool, with no `get_more_tools` shadow warning, also has the fixture's log lines.
#[test]
fn inject_tool_follows_the_fixture_cases() {
    let mut checked = 0;
    for case in vectors::cases("inject") {
        let name = vectors::name(&case);
        if kind_of(name) == "unrepresentable" {
            continue;
        }
        let input = &case["input"];
        let config = config_of(input);
        let tools = pages_of(input).swap_remove(0);
        let fixture = &case["output"][0];
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
        let registry = expected_registry(fixture);
        let output = strings(&fixture["outputRegistry"]);
        let declared = strings(&fixture["declaredSessionParams"]);
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
            check_tool(&tool, &fixture["tools"][i], &context);
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
                assert_eq!(logs, strings(&fixture["logs"]), "{context}: logs");
            }
            checked += 1;
        }
    }
    assert!(checked > 300, "only {checked} tools checked");
}

#[test]
fn get_more_tools_tool_is_the_pinned_descriptor() {
    let descriptor = vectors::cases("constants")
        .into_iter()
        .find(|case| vectors::name(case) == "get_more_tools_descriptor")
        .and_then(|mut case| case.remove("value"))
        .unwrap();
    let tool = serde_json::to_value(get_more_tools_tool()).unwrap();
    assert_same_json_text(&tool, &descriptor, "get_more_tools");
}

/// A listing never mutates the customer's tool objects or their output schemas: the pipeline edits
/// schemas with `Arc::make_mut`, so a clone of a listed tool that shares its schemas keeps them as
/// they were, also for a tool whose injection fails.
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
    // The context step stays off: it runs after a failed handle step and would give `failing` its
    // `context`.
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

/// Options with every switch random, and sometimes a context description.
fn inject_options() -> impl Strategy<Value = Value> {
    let description = prop_oneof![
        2 => Just(None),
        1 => Just(Some(String::new())),
        1 => Just(Some("why".to_owned())),
        1 => edge_string(8).prop_map(Some),
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
                    "enable_report_missing": report_missing,
                    "enable_tracing": tracing,
                    "enable_tool_call_context": context,
                    "enable_agent_tracking": agent_tracking,
                });
                if let Some(description) = description {
                    options["custom_context_description"] = json!(description);
                }
                options
            },
        )
}

/// The cases of `inject_cases.json`: one or two pages of random tools listed with random
/// options and hook modes. The two `listing:regression:` cases are the listings of the seeds in
/// `proptest-regressions/inject/tests.txt`.
#[test]
fn listings_match_inject_cases() {
    let cases = vectors::cases("inject_cases");
    assert!(cases.len() >= 50, "inject_cases.json has its cases");
    for case in &cases {
        let name = vectors::name(case);
        let input = &case["input"];
        let lists = case["output"].as_array().unwrap();
        check_pages(&config_of(input), pages_of(input), lists, name);
    }
}

/// The parameters the pipeline injects.
const INJECTED: [&str; 3] = [SESSION_ID_PARAM, AGENT_ID_PARAM, CONTEXT_PARAM];

/// Whether JSON `value` is falsy: `null`, `false`, a zero or `""`.
fn falsy(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Bool(flag) => !flag,
        Value::Number(number) => number.as_f64() == Some(0.0),
        Value::String(text) => text.is_empty(),
        Value::Array(_) | Value::Object(_) => false,
    }
}

/// The names of `tools`, in order.
fn names(tools: &[Tool]) -> Vec<&str> {
    tools.iter().map(|tool| tool.name.as_ref()).collect()
}

/// The config of `options` and `hook_mode`, and the lists of `pages` listed in turn on one
/// [`InjectState`].
fn list_pages(
    options: &Value,
    hook_mode: bool,
    pages: &[Vec<Tool>],
) -> (InjectConfig, Vec<InjectedList>) {
    let config = config_of(&json!({"options": options, "hookMode": hook_mode}));
    let mut state = InjectState::default();
    let lists = pages
        .iter()
        .map(|page| build_injected_list(&config, page.clone(), &mut state))
        .collect();
    (config, lists)
}

// The tests below take the arguments of one strategy tuple, so the seeds in
// `proptest-regressions/inject/tests.txt` give each of them the same listings.
proptest! {
    #![proptest_config(config())]

    /// A list holds the tools of its page in their order, then `get_more_tools` when
    /// report-missing is on and the page has no tool of that name.
    #[test]
    fn listing_keeps_the_tool_order_and_adds_get_more_tools_last(
        options in inject_options(),
        hook_mode in any::<bool>(),
        pages in prop::collection::vec(tool_list(), 1..=2),
    ) {
        let (config, lists) = list_pages(&options, hook_mode, &pages);
        for (page, list) in pages.iter().zip(&lists) {
            let mut expected = names(page);
            if config.report_missing && !expected.contains(&GET_MORE_TOOLS_NAME) {
                expected.push(GET_MORE_TOOLS_NAME);
            }
            prop_assert_eq!(names(&list.tools), expected);
        }
    }

    /// A listed tool keeps its name, description and annotations, and every property of its
    /// input schema but a falsy one under an injected name, which the pipeline may replace.
    #[test]
    fn listing_keeps_every_customer_property_but_a_falsy_injected_name(
        options in inject_options(),
        hook_mode in any::<bool>(),
        pages in prop::collection::vec(tool_list(), 1..=2),
    ) {
        let (_, lists) = list_pages(&options, hook_mode, &pages);
        for (page, list) in pages.iter().zip(&lists) {
            for (tool, listed) in page.iter().zip(&list.tools) {
                prop_assert_eq!(&listed.name, &tool.name);
                prop_assert_eq!(&listed.description, &tool.description);
                prop_assert_eq!(&listed.annotations, &tool.annotations);
                let Some(Value::Object(properties)) = tool.input_schema.get("properties") else {
                    continue;
                };
                let listed_properties = listed.input_schema["properties"].as_object().unwrap();
                for (key, value) in properties {
                    if !(INJECTED.contains(&key.as_str()) && falsy(value)) {
                        prop_assert_eq!(listed_properties.get(key), Some(value), "{}", key);
                    }
                }
            }
        }
    }

    /// Each parameter the registry records for a tool whose name is unique on its page is in
    /// that tool's `required` list, and among its properties when they are an object.
    #[test]
    fn injected_parameters_are_required(
        options in inject_options(),
        hook_mode in any::<bool>(),
        pages in prop::collection::vec(tool_list(), 1..=2),
    ) {
        let (_, lists) = list_pages(&options, hook_mode, &pages);
        for list in &lists {
            let listed = names(&list.tools);
            for tool in &list.tools {
                let name: &str = tool.name.as_ref();
                if name.is_empty() || listed.iter().filter(|other| **other == name).count() > 1 {
                    continue;
                }
                let (_, entry) = list.entries.iter().find(|(key, _)| key == name).unwrap();
                let schema = &tool.input_schema;
                for param in &entry.injected {
                    let required = schema.get("required").and_then(Value::as_array);
                    prop_assert!(
                        required.is_some_and(|required| required.contains(&json!(param))),
                        "{}: {} is not required", name, param
                    );
                    if let Some(Value::Object(properties)) = schema.get("properties") {
                        prop_assert!(properties.contains_key(*param), "{}: no {}", name, param);
                    }
                }
            }
        }
    }

    /// Listing the tools of a list again, with the same config on a new state, changes none of
    /// them.
    #[test]
    fn relisting_a_listed_page_changes_no_tool(
        options in inject_options(),
        hook_mode in any::<bool>(),
        pages in prop::collection::vec(tool_list(), 1..=2),
    ) {
        let (config, lists) = list_pages(&options, hook_mode, &pages);
        for list in lists {
            let again = build_injected_list(&config, list.tools.clone(), &mut InjectState::default());
            prop_assert_eq!(again.tools, list.tools);
        }
    }
}

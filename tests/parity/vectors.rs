//! Golden vectors computed by the TypeScript SDK's own functions
//! (`scripts/ts-fixtures/generate.sh`), checked as they are.

use super::support::{capture_logs, fixture_json};
use crate::inject::{InjectConfig, InjectMemory, InjectedList, build_injected_list};
use serde_json::Value;
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

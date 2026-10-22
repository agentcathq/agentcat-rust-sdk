//! `ToolRegistry`: pages merge per tool name (RD-12, C19) and the declared-session set only
//! grows (B-040).
use rmcp::model::{JsonObject, Tool};
use serde_json::{Value, json};

use super::ToolRegistry;
use crate::config::Options;
use crate::inject::{InjectConfig, build_injected_list};

/// A tool named `name` whose input schema has the properties `properties`.
fn tool(name: &str, properties: &Value) -> Tool {
    let schema: JsonObject = serde_json::from_value(json!({
        "type": "object",
        "properties": properties,
    }))
    .expect("an object");
    Tool::new(name.to_owned(), "a tool", schema)
}

/// Lists `tools` as one page into `registry`, as `Tracked::list_tools` does.
fn list(registry: &mut ToolRegistry, tools: Vec<Tool>) {
    let config = InjectConfig::from_options(&Options::default());
    let page = build_injected_list(&config, tools, registry.state_mut());
    registry.record(&page);
}

#[test]
fn pages_merge_per_tool() {
    let mut registry = ToolRegistry::default();
    assert!(!registry.listed());
    list(&mut registry, vec![tool("one", &json!({}))]);
    list(&mut registry, vec![tool("two", &json!({}))]);
    assert!(registry.listed());
    assert!(registry.entry("one").is_some(), "page 1 stays after page 2");
    assert!(registry.entry("two").is_some());
    assert!(registry.entry("three").is_none());

    // A relisted tool's entry is replaced: `one` now declares `context` itself.
    let before = registry.entry("one").cloned().expect("listed");
    assert!(before.injected.contains(&"context"));
    list(
        &mut registry,
        vec![tool("one", &json!({"context": {"type": "string"}}))],
    );
    let after = registry.entry("one").cloned().expect("listed");
    assert!(!after.injected.contains(&"context"));
    assert_ne!(before, after);
    assert!(registry.entry("two").is_some(), "other entries stay");
}

#[test]
fn declared_session_set_only_grows() {
    let mut registry = ToolRegistry::default();
    let own = json!({"session_id": {"type": "string"}});
    list(&mut registry, vec![tool("own", &own)]);
    assert!(registry.declares_session("own"));
    assert!(registry.entry("own").is_some_and(|e| e.declares_session));
    assert!(!registry.declares_session("plain"));

    // The tool drops its parameter: the entry is replaced, the declared set keeps the name.
    list(&mut registry, vec![tool("own", &json!({}))]);
    assert!(registry.entry("own").is_some_and(|e| !e.declares_session));
    assert!(registry.declares_session("own"));
}

//! Parity: `src/tests/context-parameters-edge-cases.test.ts`.

use super::support::capture_logs;
use crate::inject::{InjectConfig, InjectMemory, build_injected_list};
use serde_json::{Value, json};

/// TS `addContextParameterToTools(tools)`: the listing with the handles and
/// the report-missing tool off. Returns the advertised tools and the log.
fn context_only(tools: Value) -> (Vec<Value>, Vec<String>) {
    let tools = tools.as_array().cloned().expect("an array of tools");
    let cfg = InjectConfig::new(false, false, false, true, None, false);
    let (list, logs) = capture_logs(|| build_injected_list(&cfg, &InjectMemory::default(), &tools));
    (list.tools, logs)
}

/// TS `addContextParameterToTool(tool)`.
fn one(tool: Value) -> (Value, Vec<String>) {
    let (mut tools, logs) = context_only(json!([tool]));
    assert_eq!(tools.len(), 1);
    (tools.remove(0), logs)
}

mod context_parameters_edge_cases {
    use super::*;

    mod existing_context_parameter_collision {
        use super::*;

        #[test]
        fn should_skip_injection_for_tools_with_existing_context_parameter() {
            let (tool, logs) = one(json!({
                "name": "test-tool",
                "inputSchema": {
                    "type": "object",
                    "properties": {"context": {"type": "number", "description": "existing"}}
                }
            }));
            assert_eq!(tool["inputSchema"]["properties"]["context"]["type"], "number");
            assert_eq!(
                tool["inputSchema"]["properties"]["context"]["description"],
                "existing"
            );
            assert_eq!(
                logs,
                ["WARN: Tool \"test-tool\" already has 'context' parameter. Skipping context injection."]
            );
        }

        #[test]
        fn should_warn_with_tool_name_when_context_exists() {
            let (_, logs) = one(json!({
                "name": "my-special-tool",
                "inputSchema": {"type": "object", "properties": {"context": {"type": "boolean"}}}
            }));
            assert!(logs.iter().any(|l| l.contains("my-special-tool")), "{logs:?}");
        }
    }

    mod complex_schema_handling {
        use super::*;

        fn assert_skipped(tool: Value) {
            let name = tool["name"].as_str().expect("a name").to_owned();
            let (out, logs) = one(tool.clone());
            assert_eq!(out, tool);
            assert!(out["inputSchema"].get("properties").is_none(), "{out}");
            assert_eq!(
                logs,
                [format!(
                    "WARN: Tool \"{name}\" has complex schema (oneOf/allOf/anyOf). Skipping context injection."
                )]
            );
        }

        #[test]
        fn should_skip_injection_for_oneof_schemas() {
            assert_skipped(json!({
                "name": "union-tool",
                "inputSchema": {
                    "oneOf": [
                        {"type": "object", "properties": {"email": {"type": "string"}}},
                        {"type": "object", "properties": {"phone": {"type": "string"}}}
                    ]
                }
            }));
        }

        #[test]
        fn should_skip_injection_for_allof_schemas() {
            assert_skipped(json!({
                "name": "intersection-tool",
                "inputSchema": {
                    "allOf": [
                        {"type": "object", "properties": {"a": {"type": "string"}}},
                        {"type": "object", "properties": {"b": {"type": "string"}}}
                    ]
                }
            }));
        }

        #[test]
        fn should_skip_injection_for_anyof_schemas() {
            assert_skipped(json!({
                "name": "anyof-tool",
                "inputSchema": {"anyOf": [{"type": "string"}, {"type": "number"}]}
            }));
        }
    }

    mod additionalproperties_constraint {
        use super::*;

        #[test]
        fn should_inject_context_and_remove_additionalproperties_false_constraint() {
            let (tool, _) = one(json!({
                "name": "strict-tool",
                "inputSchema": {
                    "type": "object",
                    "properties": {"name": {"type": "string"}},
                    "additionalProperties": false
                }
            }));
            let schema = &tool["inputSchema"];
            assert_eq!(schema["properties"]["context"]["type"], "string");
            assert!(schema.get("additionalProperties").is_none(), "{schema}");
        }

        #[test]
        fn should_preserve_additionalproperties_true_when_injecting() {
            let (tool, _) = one(json!({
                "name": "flexible-tool",
                "inputSchema": {
                    "type": "object",
                    "properties": {"name": {"type": "string"}},
                    "additionalProperties": true
                }
            }));
            let schema = &tool["inputSchema"];
            assert_eq!(schema["properties"]["context"]["type"], "string");
            assert_eq!(schema["additionalProperties"], true);
        }
    }

    mod edge_case_schemas {
        use super::*;
        use crate::constants::DEFAULT_CONTEXT_PARAMETER_DESCRIPTION;

        #[test]
        fn should_handle_empty_object_schema() {
            let (tool, _) = one(json!({"name": "empty-tool", "inputSchema": {}}));
            assert_eq!(tool["inputSchema"]["properties"]["context"]["type"], "string");
            // TS adds no `type: "object"` to a `{}` schema.
            assert_eq!(
                serde_json::to_string(&tool["inputSchema"]).unwrap(),
                serde_json::to_string(&json!({
                    "properties": {
                        "context": {"type": "string", "description": DEFAULT_CONTEXT_PARAMETER_DESCRIPTION}
                    },
                    "required": ["context"]
                }))
                .unwrap()
            );
        }

        #[test]
        fn should_handle_schema_with_no_inputschema() {
            let (tool, _) = one(json!({"name": "no-schema-tool"}));
            assert_eq!(tool["inputSchema"]["properties"]["context"]["type"], "string");
            assert_eq!(tool["inputSchema"]["type"], "object");
            assert_eq!(
                serde_json::to_string(&tool).unwrap(),
                serde_json::to_string(&json!({
                    "name": "no-schema-tool",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "context": {"type": "string", "description": DEFAULT_CONTEXT_PARAMETER_DESCRIPTION}
                        },
                        "required": ["context"]
                    }
                }))
                .unwrap()
            );
        }

        #[test]
        fn should_handle_schema_with_only_type_specified() {
            let (tool, _) = one(json!({"name": "type-only-tool", "inputSchema": {"type": "object"}}));
            assert_eq!(tool["inputSchema"]["properties"]["context"]["type"], "string");
        }
    }

    mod addcontextparametertotools_batch_handling {
        use super::*;

        #[test]
        fn should_skip_get_more_tools_entirely() {
            let (tools, _) = context_only(json!([
                {"name": "get_more_tools", "inputSchema": {}},
                {"name": "other-tool", "inputSchema": {}}
            ]));
            assert!(
                tools[0]["inputSchema"].get("properties").is_none(),
                "{}",
                tools[0]
            );
            assert_eq!(tools[1]["inputSchema"]["properties"]["context"]["type"], "string");
        }

        #[test]
        fn should_handle_mixed_valid_and_complex_schemas() {
            let (tools, logs) = context_only(json!([
                {"name": "valid-tool", "inputSchema": {"type": "object", "properties": {}}},
                {"name": "complex-tool", "inputSchema": {"oneOf": [{"type": "string"}]}},
                {
                    "name": "collision-tool",
                    "inputSchema": {"type": "object", "properties": {"context": {"type": "number"}}}
                },
                {
                    "name": "strict-tool",
                    "inputSchema": {
                        "type": "object",
                        "properties": {"name": {"type": "string"}},
                        "additionalProperties": false
                    }
                }
            ]));
            assert_eq!(tools[0]["inputSchema"]["properties"]["context"]["type"], "string");
            assert!(
                tools[1]["inputSchema"].get("properties").is_none(),
                "{}",
                tools[1]
            );
            assert_eq!(tools[2]["inputSchema"]["properties"]["context"]["type"], "number");
            assert_eq!(tools[3]["inputSchema"]["properties"]["context"]["type"], "string");
            assert!(
                tools[3]["inputSchema"].get("additionalProperties").is_none(),
                "{}",
                tools[3]
            );
            assert_eq!(logs.len(), 2, "{logs:?}");
        }
    }
}

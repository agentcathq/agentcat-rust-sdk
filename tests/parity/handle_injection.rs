//! Parity: `src/tests/handle-injection.test.ts` (schema injection, argument
//! stripping and the output registry).

use std::collections::HashMap;

use super::support::{capture_logs, obj};
use crate::constants::{
    AGENT_ID_PARAM_DESCRIPTION, DEFAULT_CONTEXT_PARAMETER_DESCRIPTION, MCP_SESSION_FIELD_DESCRIPTION,
    MCP_SESSION_FIELD_DESCRIPTION_HOOK_MODE, MCP_SESSION_KEY, MCP_SESSION_STATUS_DESCRIPTION,
    SESSION_ID_PARAM_DESCRIPTION, SESSION_ID_PARAM_PATTERN,
};
use crate::inject::{
    InjectConfig, InjectMemory, Injected, InjectedList, build_injected_list, heuristic_strip, strip_arguments,
};
use rmcp::model::{CallToolRequestParams, JsonObject};
use serde_json::{Value, json};

/// TS `{injectSessionId, injectAgentId}`, with `context` on or off. The
/// report-missing tool stays off: these tests list only their own tools.
fn opts(session_id: bool, agent_id: bool, context: bool) -> InjectConfig {
    InjectConfig {
        inject_session_id: session_id,
        inject_agent_id: agent_id,
        inject_context: context,
        context_description: DEFAULT_CONTEXT_PARAMETER_DESCRIPTION.to_owned(),
        report_missing: false,
    }
}

fn build(cfg: &InjectConfig, tools: &[Value]) -> InjectedList {
    build_injected_list(cfg, &InjectMemory::default(), tools)
}

/// The TS test helper `makeTool(name, schema)`; `None` leaves out the schema.
fn make_tool(name: &str, schema: Option<Value>) -> Value {
    let mut tool = json!({"name": name});
    if let Some(schema) = schema {
        tool["inputSchema"] = schema;
    }
    tool
}

fn customer_schema() -> Value {
    json!({"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]})
}

fn structured_tool(name: &str, output_schema: Value) -> Value {
    json!({"name": name, "inputSchema": customer_schema(), "outputSchema": output_schema})
}

fn object_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {"count": {"type": "number"}},
        "required": ["count"],
        "additionalProperties": false
    })
}

fn keys(v: &Value) -> Vec<&str> {
    v.as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect()
}

/// The registry entry of `tool`, as its parameter names.
fn recorded(list: &InjectedList, tool: &str) -> Option<Vec<&'static str>> {
    list.registry.get(tool).map(|e| e.names())
}

/// A registry entry: TS `new Set([...])` of the injected names.
fn entry(session_id: bool, agent_id: bool, context: bool) -> Injected {
    Injected {
        session_id,
        agent_id,
        context,
    }
}

/// TS `stripInjectedArguments(args, toolName, registry)`: what the registry
/// recorded for the tool, or the fallback for a tool it has no entry for.
/// Arguments are rmcp's `Option<JsonObject>`; only an object is stripped.
fn strip_injected_arguments(
    args: Option<Value>,
    tool: &str,
    registry: Option<&HashMap<String, Injected>>,
) -> Option<Value> {
    let strip = registry
        .and_then(|r| r.get(tool))
        .copied()
        .unwrap_or_else(|| heuristic_strip(tool));
    let mut args: Option<JsonObject> = args.map(obj);
    if let Some(args) = args.as_mut() {
        strip_arguments(args, strip);
    }
    args.map(Value::Object)
}

mod addhandleparameterstotools {
    use super::*;

    #[test]
    fn injects_session_id_and_agent_id_after_customer_params_both_required() {
        let list = build(
            &opts(true, true, false),
            &[make_tool("add_todo", Some(customer_schema()))],
        );
        let schema = &list.tools[0]["inputSchema"];
        assert_eq!(keys(&schema["properties"]), ["text", "session_id", "agent_id"]);
        assert_eq!(schema["required"], json!(["text", "session_id", "agent_id"]));
        assert_eq!(
            schema["properties"]["session_id"]["description"],
            SESSION_ID_PARAM_DESCRIPTION
        );
        assert_eq!(
            schema["properties"]["session_id"]["pattern"],
            SESSION_ID_PARAM_PATTERN
        );
        assert_eq!(SESSION_ID_PARAM_PATTERN, "^(start|ses_[0-9A-Za-z]{27})$");
        assert_eq!(
            schema["properties"]["agent_id"]["description"],
            AGENT_ID_PARAM_DESCRIPTION
        );
        assert!(schema["properties"]["agent_id"].get("pattern").is_none());
        assert_eq!(recorded(&list, "add_todo"), Some(vec!["session_id", "agent_id"]));
    }

    #[test]
    fn creates_the_required_array_when_the_schema_has_none() {
        let list = build(
            &opts(true, true, false),
            &[make_tool(
                "bare",
                Some(json!({"type": "object", "properties": {}})),
            )],
        );
        assert_eq!(
            list.tools[0]["inputSchema"]["required"],
            json!(["session_id", "agent_id"])
        );
    }

    #[test]
    fn does_not_duplicate_session_id_in_an_existing_required_array() {
        let list = build(
            &opts(true, false, false),
            &[make_tool(
                "weird_session",
                Some(json!({"type": "object", "properties": {}, "required": ["session_id"]})),
            )],
        );
        assert_eq!(list.tools[0]["inputSchema"]["required"], json!(["session_id"]));
    }

    #[test]
    fn does_not_duplicate_agent_id_in_an_existing_required_array() {
        let list = build(
            &opts(false, true, false),
            &[make_tool(
                "weird",
                Some(json!({"type": "object", "properties": {}, "required": ["agent_id"]})),
            )],
        );
        assert_eq!(list.tools[0]["inputSchema"]["required"], json!(["agent_id"]));
    }

    #[test]
    fn leaves_required_untouched_when_the_customer_declares_agent_id() {
        let list = build(
            &opts(false, true, false),
            &[make_tool(
                "owns_agent",
                Some(json!({
                    "type": "object",
                    "properties": {"agent_id": {"type": "string", "description": "mine"}},
                    "required": []
                })),
            )],
        );
        let schema = &list.tools[0]["inputSchema"];
        assert_eq!(schema["required"], json!([]));
        assert_eq!(schema["properties"]["agent_id"]["description"], "mine");
    }

    #[test]
    fn full_order_with_context_injector_customer_session_id_agent_id_context() {
        let list = build(
            &opts(true, true, true),
            &[make_tool("add_todo", Some(customer_schema()))],
        );
        let schema = &list.tools[0]["inputSchema"];
        assert_eq!(
            keys(&schema["properties"]),
            ["text", "session_id", "agent_id", "context"]
        );
        assert_eq!(
            schema["required"],
            json!(["text", "session_id", "agent_id", "context"])
        );
        assert_eq!(
            recorded(&list, "add_todo"),
            Some(vec!["session_id", "agent_id", "context"])
        );
    }

    #[test]
    fn hook_mode_no_session_id_agent_id_uses_the_same_single_description() {
        let list = build(
            &opts(false, true, false),
            &[make_tool("add_todo", Some(customer_schema()))],
        );
        let schema = &list.tools[0]["inputSchema"];
        assert!(schema["properties"].get("session_id").is_none());
        assert_eq!(
            schema["properties"]["agent_id"]["description"],
            AGENT_ID_PARAM_DESCRIPTION
        );
        assert_eq!(schema["required"], json!(["text", "agent_id"]));
        assert_eq!(recorded(&list, "add_todo"), Some(vec!["agent_id"]));
    }

    #[test]
    fn skips_a_param_the_tool_already_defines_and_does_not_record_it() {
        let mut schema = customer_schema();
        schema["properties"]["session_id"] = json!({"type": "string", "description": "customer's own"});
        let (list, _) =
            capture_logs(|| build(&opts(true, true, false), &[make_tool("deploy", Some(schema))]));
        let schema = &list.tools[0]["inputSchema"];
        assert_eq!(
            schema["properties"]["session_id"]["description"],
            "customer's own"
        );
        assert!(schema["properties"]["session_id"].get("pattern").is_none());
        assert_eq!(schema["required"], json!(["text", "agent_id"]));
        assert_eq!(recorded(&list, "deploy"), Some(vec!["agent_id"]));
    }

    #[test]
    fn skips_complex_schemas_entirely() {
        let tool = make_tool("odd", Some(json!({"oneOf": [{"type": "object"}]})));
        let (list, logs) = capture_logs(|| build(&opts(true, true, false), std::slice::from_ref(&tool)));
        let schema = &list.tools[0]["inputSchema"];
        assert!(schema.get("properties").is_none());
        assert!(schema.get("required").is_none());
        assert_eq!(list.tools[0], tool);
        // The listing gives every named tool an entry; this one records nothing.
        assert_eq!(recorded(&list, "odd"), Some(vec![]));
        assert_eq!(
            logs,
            ["WARN: Tool \"odd\" has complex schema (oneOf/allOf/anyOf). Skipping handle injection."]
        );
    }

    #[test]
    fn removes_additionalproperties_false_and_handles_missing_schemas() {
        let mut strict = customer_schema();
        strict["additionalProperties"] = json!(false);
        let list = build(
            &opts(true, true, false),
            &[
                make_tool("strict", Some(strict)),
                make_tool("bare", None),
                make_tool("empty", Some(json!({}))),
            ],
        );
        assert!(list.tools[0]["inputSchema"].get("additionalProperties").is_none());
        for tool in &list.tools[1..] {
            let schema = &tool["inputSchema"];
            assert_eq!(keys(&schema["properties"]), ["session_id", "agent_id"]);
            assert_eq!(schema["required"], json!(["session_id", "agent_id"]));
        }
        // A missing schema becomes TS's empty object schema; `{}` gets no `type`.
        assert_eq!(
            keys(&list.tools[1]["inputSchema"]),
            ["type", "properties", "required"]
        );
        assert_eq!(keys(&list.tools[2]["inputSchema"]), ["properties", "required"]);
    }

    #[test]
    fn does_not_skip_get_more_tools_handles_are_injected_context_stays_bespoke() {
        let gmt = make_tool(
            "get_more_tools",
            Some(json!({
                "type": "object",
                "properties": {"context": {"type": "string", "description": "bespoke"}},
                "required": ["context"]
            })),
        );
        let list = build(&opts(true, true, true), &[gmt]);
        let schema = &list.tools[0]["inputSchema"];
        assert_eq!(keys(&schema["properties"]), ["context", "session_id", "agent_id"]);
        assert_eq!(schema["properties"]["context"]["description"], "bespoke");
        assert_eq!(schema["required"], json!(["context", "session_id", "agent_id"]));
        assert_eq!(
            recorded(&list, "get_more_tools"),
            Some(vec!["session_id", "agent_id"])
        );
    }

    #[test]
    fn agent_id_copy_prescribes_the_self_chosen_model_harness_nonce_format() {
        assert!(AGENT_ID_PARAM_DESCRIPTION.contains("required on every call including your first"));
        assert!(AGENT_ID_PARAM_DESCRIPTION.contains("opus-4.80-1m|claude-code|k3n9x"));
        assert!(AGENT_ID_PARAM_DESCRIPTION.contains("never inherited"));
        assert!(!AGENT_ID_PARAM_DESCRIPTION.contains("the server will issue"));
        assert!(!AGENT_ID_PARAM_DESCRIPTION.contains("session_id"));
    }
}

mod stripinjectedarguments {
    use super::*;

    /// `deploy` owns its `session_id`, so the listing never recorded it.
    fn registry() -> HashMap<String, Injected> {
        HashMap::from([
            ("add_todo".to_owned(), entry(true, true, true)),
            ("deploy".to_owned(), entry(false, true, true)),
        ])
    }

    #[test]
    fn strips_exactly_what_was_injected() {
        assert_eq!(
            strip_injected_arguments(
                Some(json!({"text": "x", "session_id": "ses_1", "agent_id": "agt_1", "context": "why"})),
                "add_todo",
                Some(&registry()),
            ),
            Some(json!({"text": "x"}))
        );
    }

    #[test]
    fn preserves_the_customer_s_own_session_id_param() {
        assert_eq!(
            strip_injected_arguments(
                Some(json!({"session_id": "prod-42", "agent_id": "agt_1", "context": "why"})),
                "deploy",
                Some(&registry()),
            ),
            Some(json!({"session_id": "prod-42"}))
        );
    }

    #[test]
    fn falls_back_to_stripping_all_three_when_no_registry_entry_exists() {
        assert_eq!(
            strip_injected_arguments(
                Some(json!({"text": "x", "session_id": "a", "agent_id": "b", "context": "c"})),
                "unlisted_tool",
                Some(&registry()),
            ),
            Some(json!({"text": "x"}))
        );
        assert_eq!(
            strip_injected_arguments(Some(json!({"text": "x", "session_id": "a"})), "any", None),
            Some(json!({"text": "x"}))
        );
    }

    #[test]
    fn fallback_never_strips_get_more_tools_own_context() {
        assert_eq!(
            strip_injected_arguments(
                Some(json!({"context": "need a tool", "session_id": "a", "agent_id": "b"})),
                "get_more_tools",
                None,
            ),
            Some(json!({"context": "need a tool"}))
        );
    }

    #[test]
    fn handles_non_object_args() {
        assert_eq!(
            strip_injected_arguments(None, "add_todo", Some(&registry())),
            None
        );
    }
}

mod registry_storage_request_cloning {
    use super::*;

    #[test]
    fn clones_the_request_with_stripped_arguments_leaving_the_original_intact() {
        let registry = HashMap::from([("add_todo".to_owned(), entry(true, true, true))]);
        let request = CallToolRequestParams::new("add_todo").with_arguments(obj(json!({
            "text": "x",
            "session_id": "ses_1",
            "context": "why"
        })));
        let mut cloned = request.clone();
        if let Some(args) = cloned.arguments.as_mut() {
            strip_arguments(args, registry["add_todo"]);
        }
        assert_eq!(cloned.arguments, Some(obj(json!({"text": "x"}))));
        // The event keeps the raw request.
        let original = request.arguments.expect("the original keeps its arguments");
        assert_eq!(original["session_id"], "ses_1");
        assert_eq!(original["context"], "why");
    }
}

mod outputschema_injection {
    use super::*;

    #[test]
    fn injects_mcp_session_into_a_plain_object_outputschema_and_registers_the_tool() {
        let list = build(
            &opts(true, true, false),
            &[structured_tool("get_stats", object_output_schema())],
        );
        let output = &list.tools[0]["outputSchema"];
        let prop = &output["properties"][MCP_SESSION_KEY];
        assert_eq!(prop["type"], "object");
        assert_eq!(prop["description"], MCP_SESSION_FIELD_DESCRIPTION);
        assert_eq!(keys(&prop["properties"]), ["session_id", "agent_id", "status"]);
        assert_eq!(
            prop["properties"]["status"],
            json!({
                "type": "string",
                "enum": ["issued", "active", "unrecognized"],
                "description": MCP_SESSION_STATUS_DESCRIPTION
            })
        );
        assert_eq!(output["required"], json!(["count"]));
        assert_eq!(output["additionalProperties"], false);
        assert_eq!(output["properties"]["count"], json!({"type": "number"}));
        assert!(list.output_registry.contains("get_stats"));
    }

    #[test]
    fn sub_properties_track_modes_hook_mode_drops_session_id_and_status_track_876027b9() {
        let hook = build(
            &opts(false, true, false),
            &[structured_tool("t", object_output_schema())],
        );
        let hook_prop = &hook.tools[0]["outputSchema"]["properties"][MCP_SESSION_KEY];
        assert_eq!(keys(&hook_prop["properties"]), ["agent_id"]);
        assert_eq!(hook_prop["description"], MCP_SESSION_FIELD_DESCRIPTION_HOOK_MODE);

        let session_only = build(
            &opts(true, false, false),
            &[structured_tool("t", object_output_schema())],
        );
        let session_prop = &session_only.tools[0]["outputSchema"]["properties"][MCP_SESSION_KEY];
        assert_eq!(keys(&session_prop["properties"]), ["session_id", "status"]);
        assert_eq!(session_prop["description"], MCP_SESSION_FIELD_DESCRIPTION);
    }

    #[test]
    fn does_not_mutate_the_customer_s_outputschema_object() {
        let tools = [structured_tool("t", object_output_schema())];
        let list = build(&opts(true, true, false), &tools);
        assert!(
            tools[0]["outputSchema"]["properties"]
                .get(MCP_SESSION_KEY)
                .is_none()
        );
        assert!(list.tools[0]["outputSchema"]["properties"][MCP_SESSION_KEY].is_object());
    }

    #[test]
    fn skips_complex_outputschema_oneof_allof_anyof_and_does_not_register() {
        let complex = json!({"oneOf": [{"type": "object", "properties": {"a": {"type": "string"}}}]});
        let (list, logs) = capture_logs(|| {
            build(
                &opts(true, true, false),
                &[structured_tool("poly", complex.clone())],
            )
        });
        assert_eq!(list.tools[0]["outputSchema"], complex);
        assert!(list.output_registry.is_empty());
        assert!(list.tools[0]["inputSchema"]["properties"]["session_id"].is_object());
        assert_eq!(
            logs,
            [
                "WARN: Tool \"poly\" has complex outputSchema (oneOf/allOf/anyOf). Skipping mcp_session injection; mint-back stays content-only for this tool."
            ]
        );
    }

    #[test]
    fn composed_inputschema_early_return_skips_output_injection_too_nothing_registered() {
        let mut tool = structured_tool("mixed", object_output_schema());
        tool["inputSchema"] = json!({"oneOf": [{"type": "object", "properties": {"a": {"type": "string"}}}]});
        let (list, _) = capture_logs(|| build(&opts(true, true, false), &[tool]));
        assert_eq!(list.tools[0]["outputSchema"], object_output_schema());
        assert!(list.output_registry.is_empty());
    }

    #[test]
    fn collision_a_customer_declared_mcp_session_is_never_clobbered() {
        let schema = json!({"type": "object", "properties": {MCP_SESSION_KEY: {"type": "string"}}});
        let (list, logs) = capture_logs(|| build(&opts(true, true, false), &[structured_tool("t", schema)]));
        assert_eq!(
            list.tools[0]["outputSchema"]["properties"][MCP_SESSION_KEY],
            json!({"type": "string"})
        );
        assert!(list.output_registry.is_empty());
        assert_eq!(
            logs,
            ["WARN: Tool \"t\" already declares 'mcp_session' in outputSchema. Skipping injection."]
        );
    }

    #[test]
    fn no_outputschema_untouched_nothing_registered() {
        let list = build(
            &opts(true, true, false),
            &[make_tool("t", Some(customer_schema()))],
        );
        assert!(list.tools[0].get("outputSchema").is_none());
        assert!(list.output_registry.is_empty());
    }

    #[test]
    fn no_injection_at_all_neither_handle_leaves_outputschema_alone() {
        let list = build(
            &opts(false, false, false),
            &[structured_tool("t", object_output_schema())],
        );
        assert!(
            list.tools[0]["outputSchema"]["properties"]
                .get(MCP_SESSION_KEY)
                .is_none()
        );
        assert!(list.output_registry.is_empty());
    }
}

mod rust_extras {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn stripping_keeps_argument_order() {
        let mut args = obj(json!({"a": 1, "session_id": "x", "b": 2, "context": "y", "c": 3}));
        strip_arguments(&mut args, entry(true, true, true));
        assert_eq!(keys(&Value::Object(args)), ["a", "b", "c"]);
    }

    #[test]
    fn malformed_schema_parts_take_the_ts_failure_path() {
        let plain = || json!({"type": "object", "properties": {"q": {"type": "string"}}});
        let malformed = |properties: Value| json!({"type": "object", "properties": properties});
        let tools = [
            json!({"name": "a", "inputSchema": malformed(json!("abc"))}),
            json!({"name": "b", "inputSchema": malformed(json!(5))}),
            json!({"name": "c", "inputSchema": malformed(json!(true))}),
            json!({"name": "d", "inputSchema": malformed(json!([]))}),
            json!({
                "name": "e",
                "inputSchema": plain(),
                "outputSchema": {"type": "object", "properties": "x"}
            }),
            json!({"name": "ok", "inputSchema": plain()}),
        ];
        let cfg = InjectConfig::new(true, false, true, true, None, false);
        let (list, logs) = capture_logs(|| build_injected_list(&cfg, &InjectMemory::default(), &tools));

        let handle = |tool: &str, rest: &str| {
            format!(
                "WARN: Handle injection failed for tool \"{tool}\"; listing it unmodified - TypeError: {rest}"
            )
        };
        let context = |tool: &str, rest: &str| {
            format!(
                "WARN: Context injection failed for tool \"{tool}\"; listing it unmodified - TypeError: {rest}"
            )
        };
        assert_eq!(
            logs,
            [
                handle("a", "Cannot create property 'session_id' on string 'abc'"),
                handle("b", "Cannot create property 'session_id' on number '5'"),
                handle("c", "Cannot create property 'session_id' on boolean 'true'"),
                handle("e", "Cannot create property 'mcp_session' on string 'x'"),
                context("a", "Cannot create property 'context' on string 'abc'"),
                context("b", "Cannot create property 'context' on number '5'"),
                context("c", "Cannot create property 'context' on boolean 'true'"),
            ]
        );

        for (i, name) in ["a", "b", "c"].into_iter().enumerate() {
            assert_eq!(list.tools[i], tools[i], "{name}");
            assert_eq!(recorded(&list, name), Some(vec![]), "{name}");
        }

        // JS stores the parameters as named keys of the array, which JSON
        // drops; the registry still records them.
        assert_eq!(
            serde_json::to_string(&list.tools[3]).unwrap(),
            serde_json::to_string(&json!({
                "name": "d",
                "inputSchema": {
                    "type": "object",
                    "properties": [],
                    "required": ["session_id", "agent_id", "context"]
                }
            }))
            .unwrap()
        );
        assert_eq!(
            recorded(&list, "d"),
            Some(vec!["session_id", "agent_id", "context"])
        );

        // The handle step is rolled back; the context step runs on the
        // original tool.
        let e = &list.tools[4];
        assert_eq!(keys(&e["inputSchema"]["properties"]), ["q", "context"]);
        assert_eq!(e["inputSchema"]["required"], json!(["context"]));
        assert_eq!(e["outputSchema"], tools[4]["outputSchema"]);
        assert_eq!(recorded(&list, "e"), Some(vec!["context"]));
        assert!(!list.output_registry.contains("e"));

        let ok = &list.tools[5]["inputSchema"];
        assert_eq!(
            keys(&ok["properties"]),
            ["q", "session_id", "agent_id", "context"]
        );
        assert_eq!(ok["required"], json!(["session_id", "agent_id", "context"]));
        assert_eq!(
            recorded(&list, "ok"),
            Some(vec!["session_id", "agent_id", "context"])
        );
        assert_eq!(list.tools.len(), 6);
        assert_eq!(list.registry.len(), 6);
        assert!(list.output_registry.is_empty());
    }

    #[test]
    fn falsy_input_schemas_take_the_default_schema() {
        let tools: Vec<Value> = [json!(null), json!(false), json!(0), json!("")]
            .into_iter()
            .enumerate()
            .map(|(i, schema)| json!({"name": format!("t{i}"), "inputSchema": schema}))
            .collect();
        let (list, logs) = capture_logs(|| build(&opts(true, false, true), &tools));
        assert!(logs.is_empty(), "{logs:?}");
        for (i, tool) in list.tools.iter().enumerate() {
            let schema = &tool["inputSchema"];
            assert_eq!(keys(schema), ["type", "properties", "required"], "t{i}");
            assert_eq!(keys(&schema["properties"]), ["session_id", "context"], "t{i}");
            assert_eq!(schema["required"], json!(["session_id", "context"]), "t{i}");
        }
    }

    #[test]
    fn removing_additional_properties_keeps_key_order() {
        let tool = json!({
            "name": "strict",
            "inputSchema": {
                "type": "object",
                "additionalProperties": false,
                "properties": {"q": {}},
                "required": ["q"]
            }
        });
        for cfg in [opts(true, false, false), opts(false, false, true)] {
            let list = build(&cfg, std::slice::from_ref(&tool));
            assert_eq!(
                keys(&list.tools[0]["inputSchema"]),
                ["type", "properties", "required"]
            );
        }
    }

    #[test]
    fn concurrent_listings_report_each_conflict_once() {
        let memory = Arc::new(InjectMemory::default());
        let tools = Arc::new([
            json!({"name": "own_session", "inputSchema": {"type": "object", "properties": {"session_id": {"type": "string"}}}}),
            json!({"name": "plain", "inputSchema": {"type": "object", "properties": {"q": {"type": "string"}}}}),
        ]);
        let cfg = Arc::new(InjectConfig::new(true, false, false, true, None, true));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let (memory, tools, cfg) = (memory.clone(), tools.clone(), cfg.clone());
                std::thread::spawn(move || {
                    capture_logs(|| {
                        for _ in 0..100 {
                            let list = build_injected_list(&cfg, &memory, &tools[..]);
                            assert_eq!(list.tools.len(), 3);
                        }
                    })
                    .1
                })
            })
            .collect();
        let errors: usize = threads
            .into_iter()
            .map(|t| t.join().expect("a listing thread never panics"))
            .map(|logs| logs.iter().filter(|l| l.starts_with("ERROR:")).count())
            .sum();
        assert_eq!(errors, 1);
        assert_eq!(memory.declared_session_params(), ["own_session"]);
    }
}

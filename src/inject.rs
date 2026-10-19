//! Tool-schema injection: the `session_id`, `agent_id` and `context`
//! parameters, the `mcp_session` output property, and argument stripping.
//!
//! The engine is pure and deterministic: the same configuration and tool
//! always give the same advertised schema. A tool call on an instance that
//! never served `tools/list` can therefore recompute exactly what a listing
//! advertised, one tool at a time.

use std::collections::HashSet;
use std::sync::Mutex;

use serde_json::{Map, Value, json};

use crate::constants::{
    AGENT_ID_PARAM, AGENT_ID_PARAM_DESCRIPTION, CONTEXT_PARAM, DEFAULT_CONTEXT_PARAMETER_DESCRIPTION,
    GET_MORE_TOOLS_CONTEXT_DESCRIPTION, GET_MORE_TOOLS_DESCRIPTION, GET_MORE_TOOLS_NAME,
    GET_MORE_TOOLS_RESPONSE, GET_MORE_TOOLS_TITLE, MCP_SESSION_AGENT_ID_DESCRIPTION,
    MCP_SESSION_FIELD_DESCRIPTION, MCP_SESSION_FIELD_DESCRIPTION_HOOK_MODE, MCP_SESSION_KEY,
    MCP_SESSION_SESSION_ID_DESCRIPTION, MCP_SESSION_STATUS_DESCRIPTION, SESSION_ID_PARAM,
    SESSION_ID_PARAM_DESCRIPTION, SESSION_ID_PARAM_PATTERN,
};
use crate::logging::write_to_log;

/// Which `AgentCat` parameters were injected into one tool's input schema.
/// These are exactly the arguments stripped before the customer's handler
/// runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Injected {
    pub(crate) session_id: bool,
    pub(crate) agent_id: bool,
    pub(crate) context: bool,
}

/// What the engine decided for one tool.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ToolInjection {
    /// Parameters to strip from this tool's arguments.
    pub(crate) strip: Injected,
    /// The customer's own schema declares `session_id`: the value is theirs,
    /// never an `AgentCat` handle.
    pub(crate) declares_session_id: bool,
    /// `mcp_session` was declared in this tool's output schema.
    pub(crate) output_injected: bool,
}

/// The configuration the injection depends on.
#[derive(Clone, Debug)]
pub(crate) struct InjectConfig {
    pub(crate) inject_session_id: bool,
    pub(crate) inject_agent_id: bool,
    pub(crate) inject_context: bool,
    pub(crate) context_description: String,
    pub(crate) report_missing: bool,
}

/// Grow-only, per-configuration memory shared by every tracked instance.
#[derive(Default)]
pub(crate) struct InjectMemory {
    /// Tools already reported for a `session_id` collision (log once).
    reported_conflicts: Mutex<HashSet<String>>,
    /// Tools whose schema declares `session_id` itself.
    declared_session_params: Mutex<HashSet<String>>,
    /// The customer registers its own `get_more_tools` (warn once).
    warned_customer_report_missing: std::sync::atomic::AtomicBool,
}

impl InjectMemory {
    pub(crate) fn declares_session_id(&self, tool: &str) -> bool {
        self.declared_session_params
            .lock()
            .is_ok_and(|s| s.contains(tool))
    }

    fn record_declared(&self, tool: &str) {
        if let Ok(mut s) = self.declared_session_params.lock() {
            s.insert(tool.to_owned());
        }
    }

    /// Returns true the first time a conflict is reported for `tool`.
    fn first_conflict(&self, tool: &str) -> bool {
        self.reported_conflicts
            .lock()
            .is_ok_and(|mut s| s.insert(tool.to_owned()))
    }

    pub(crate) fn record_customer_report_missing(&self) {
        use std::sync::atomic::Ordering;
        if !self.warned_customer_report_missing.swap(true, Ordering::Relaxed) {
            write_to_log(&format!(
                "WARN: This server registers its own \"{GET_MORE_TOOLS_NAME}\" tool, which is shadowed by AgentCat's report-missing tool: calls to it are answered by AgentCat and never reach your handler. Rename your tool, or set enable_report_missing to false to keep yours reachable."
            ));
        }
    }
}

/// JavaScript truthiness of a JSON value.
fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(_) | Value::Object(_)) => true,
    }
}

fn is_composed(schema: &Map<String, Value>) -> bool {
    truthy(schema.get("oneOf")) || truthy(schema.get("allOf")) || truthy(schema.get("anyOf"))
}

/// True when any `properties` object anywhere in the schema (root, composed
/// branches, `$defs`, nested objects) declares `param`.
fn declared_anywhere(schema: &Map<String, Value>, param: &str) -> bool {
    fn walk(value: &Value, param: &str) -> bool {
        match value {
            Value::Object(map) => {
                map.get("properties")
                    .and_then(Value::as_object)
                    .is_some_and(|p| p.contains_key(param))
                    || map.values().any(|v| walk(v, param))
            }
            Value::Array(items) => items.iter().any(|v| walk(v, param)),
            _ => false,
        }
    }
    schema
        .get("properties")
        .and_then(Value::as_object)
        .is_some_and(|p| p.contains_key(param))
        || schema.values().any(|v| walk(v, param))
}

fn declares(schema: &Map<String, Value>, param: &str) -> bool {
    truthy(schema.get("properties").and_then(|p| p.get(param)))
}

/// Ensures `schema.properties` is an object and drops
/// `additionalProperties: false`, which would reject injected parameters.
fn prepare_properties(schema: &mut Map<String, Value>) {
    if !truthy(schema.get("properties")) || !schema["properties"].is_object() {
        schema.insert("properties".into(), Value::Object(Map::new()));
    }
    if schema.get("additionalProperties") == Some(&Value::Bool(false)) {
        crate::util::shift_remove(schema, "additionalProperties");
    }
    // MCP requires object schemas; an empty `{}` schema that now carries
    // properties gets the missing `type` (a TS gap, fixed here).
    if !schema.contains_key("type") {
        let mut out = Map::with_capacity(schema.len() + 1);
        out.insert("type".into(), Value::String("object".into()));
        out.extend(std::mem::take(schema));
        *schema = out;
    }
}

/// Adds `param` to the schema's `required` array: created when absent,
/// appended without duplicates, customer entries never reordered.
fn add_to_required(schema: &mut Map<String, Value>, param: &str) {
    match schema.get_mut("required") {
        Some(Value::Array(required)) => {
            if !required.iter().any(|v| v.as_str() == Some(param)) {
                required.push(Value::String(param.into()));
            }
        }
        _ => {
            schema.insert("required".into(), json!([param]));
        }
    }
}

/// Inserts a property, making `properties` an object first if needed.
fn insert_property(schema: &mut Map<String, Value>, name: &str, definition: Value) {
    let properties = schema
        .entry("properties")
        .or_insert_with(|| Value::Object(Map::new()));
    if !properties.is_object() {
        *properties = Value::Object(Map::new());
    }
    if let Value::Object(p) = properties {
        p.insert(name.to_owned(), definition);
    }
}

/// Injects the `AgentCat` parameters into one tool's schemas, in place.
///
/// Order matches the TypeScript SDK: customer parameters, then `session_id`,
/// `agent_id`, then `context`. Both handles are schema-required with soft
/// enforcement: a call that omits them still works.
pub(crate) fn inject_tool(
    cfg: &InjectConfig,
    memory: &InjectMemory,
    name: &str,
    input: &mut Map<String, Value>,
    output: Option<&mut Map<String, Value>>,
) -> ToolInjection {
    let mut result = ToolInjection::default();
    let log_name = if name.is_empty() { "unknown" } else { name };

    // ── Handles ──
    if cfg.inject_session_id || cfg.inject_agent_id {
        if is_composed(input) {
            // Injection is skipped, but ownership is still recorded: a root
            // `session_id` in a composed schema belongs to the customer.
            if cfg.inject_session_id && declares(input, SESSION_ID_PARAM) {
                memory.record_declared(name);
                result.declares_session_id = true;
            }
            write_to_log(&format!(
                "WARN: Tool \"{log_name}\" has complex schema (oneOf/allOf/anyOf). Skipping handle injection."
            ));
            // A composed tool never advertises the handles, but an agent may
            // still echo them from another tool's mint-back; strip them so a
            // strict handler never sees them. Only when no branch (or
            // definition) of the schema declares the name: then the
            // argument belongs to the customer.
            result.strip.session_id = cfg.inject_session_id && !declared_anywhere(input, SESSION_ID_PARAM);
            result.strip.agent_id = cfg.inject_agent_id && !declared_anywhere(input, AGENT_ID_PARAM);
        } else {
            prepare_properties(input);
            if cfg.inject_session_id {
                if declares(input, SESSION_ID_PARAM) {
                    memory.record_declared(name);
                    result.declares_session_id = true;
                    if memory.first_conflict(name) {
                        write_to_log(&format!(
                            "ERROR: Tool \"{log_name}\" already declares a '{SESSION_ID_PARAM}' parameter. AgentCat will not inject its own, and calls to this tool are published without a session, so they cannot be correlated. Your parameter is untouched and still reaches your handler. If you already manage sessions, pass a resolve_session_id hook to AgentCat — AgentCat will derive its session from your identifier and stop injecting {SESSION_ID_PARAM} entirely."
                        ));
                    }
                } else {
                    insert_property(
                        input,
                        SESSION_ID_PARAM,
                        json!({
                            "type": "string",
                            "description": SESSION_ID_PARAM_DESCRIPTION,
                            "pattern": SESSION_ID_PARAM_PATTERN,
                        }),
                    );
                    result.strip.session_id = true;
                    add_to_required(input, SESSION_ID_PARAM);
                }
            }
            if cfg.inject_agent_id {
                if declares(input, AGENT_ID_PARAM) {
                    write_to_log(&format!(
                        "WARN: Tool \"{log_name}\" already has '{AGENT_ID_PARAM}' parameter. Skipping agent_id injection."
                    ));
                } else {
                    insert_property(
                        input,
                        AGENT_ID_PARAM,
                        json!({"type": "string", "description": AGENT_ID_PARAM_DESCRIPTION}),
                    );
                    result.strip.agent_id = true;
                    add_to_required(input, AGENT_ID_PARAM);
                }
            }
            if let Some(output) = output {
                result.output_injected = inject_output_schema(cfg, log_name, output);
            }
        }
    }

    // ── Context ── (get_more_tools carries its own `context` parameter)
    if cfg.inject_context && name != GET_MORE_TOOLS_NAME {
        if declares(input, CONTEXT_PARAM) {
            write_to_log(&format!(
                "WARN: Tool \"{log_name}\" already has 'context' parameter. Skipping context injection."
            ));
        } else if is_composed(input) {
            write_to_log(&format!(
                "WARN: Tool \"{log_name}\" has complex schema (oneOf/allOf/anyOf). Skipping context injection."
            ));
        } else {
            prepare_properties(input);
            insert_property(
                input,
                CONTEXT_PARAM,
                json!({"type": "string", "description": cfg.context_description}),
            );
            result.strip.context = true;
            add_to_required(input, CONTEXT_PARAM);
        }
    }

    result
}

/// Declares the optional `mcp_session` property in a plain-object output
/// schema, so validating clients accept the mirrored field. Returns true when
/// the property was added.
fn inject_output_schema(cfg: &InjectConfig, log_name: &str, schema: &mut Map<String, Value>) -> bool {
    if is_composed(schema) {
        write_to_log(&format!(
            "WARN: Tool \"{log_name}\" has complex outputSchema (oneOf/allOf/anyOf). Skipping {MCP_SESSION_KEY} injection; mint-back stays content-only for this tool."
        ));
        return false;
    }
    if declares(schema, MCP_SESSION_KEY) {
        write_to_log(&format!(
            "WARN: Tool \"{log_name}\" already declares '{MCP_SESSION_KEY}' in outputSchema. Skipping injection."
        ));
        return false;
    }
    let mut sub = Map::new();
    if cfg.inject_session_id {
        sub.insert(
            SESSION_ID_PARAM.into(),
            json!({"type": "string", "description": MCP_SESSION_SESSION_ID_DESCRIPTION}),
        );
    }
    if cfg.inject_agent_id {
        sub.insert(
            AGENT_ID_PARAM.into(),
            json!({"type": "string", "description": MCP_SESSION_AGENT_ID_DESCRIPTION}),
        );
    }
    if cfg.inject_session_id {
        sub.insert(
            "status".into(),
            json!({
                "type": "string",
                "enum": ["issued", "active", "unrecognized"],
                "description": MCP_SESSION_STATUS_DESCRIPTION,
            }),
        );
    }
    let description = if cfg.inject_session_id {
        MCP_SESSION_FIELD_DESCRIPTION
    } else {
        MCP_SESSION_FIELD_DESCRIPTION_HOOK_MODE
    };
    insert_property(
        schema,
        MCP_SESSION_KEY,
        json!({"type": "object", "description": description, "properties": sub}),
    );
    true
}

/// The argument names stripped for a tool never seen in any listing: all
/// three, except `get_more_tools`' own `context`, which is a real parameter.
pub(crate) fn heuristic_strip(name: &str) -> Injected {
    Injected {
        session_id: true,
        agent_id: true,
        context: name != GET_MORE_TOOLS_NAME,
    }
}

/// Removes the injected arguments. Returns the stripped map; the caller keeps
/// the raw arguments for the event.
pub(crate) fn strip_arguments(args: &mut Map<String, Value>, strip: Injected) {
    if strip.session_id {
        crate::util::shift_remove(args, SESSION_ID_PARAM);
    }
    if strip.agent_id {
        crate::util::shift_remove(args, AGENT_ID_PARAM);
    }
    if strip.context {
        crate::util::shift_remove(args, CONTEXT_PARAM);
    }
}

/// The report-missing tool definition, before injection.
pub(crate) fn report_missing_descriptor() -> Value {
    json!({
        "name": GET_MORE_TOOLS_NAME,
        "description": GET_MORE_TOOLS_DESCRIPTION,
        "inputSchema": {
            "type": "object",
            "properties": {
                "context": {
                    "type": "string",
                    "description": GET_MORE_TOOLS_CONTEXT_DESCRIPTION,
                }
            },
            "required": ["context"],
        },
        // Spec defaults assume the worst (destructive, open-world); declare
        // the honest hints so annotation-aware clients skip confirmations.
        "annotations": {
            "title": GET_MORE_TOOLS_TITLE,
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true,
            "openWorldHint": false,
        },
    })
}

/// The canned answer to a `get_more_tools` call.
pub(crate) fn report_missing_response(context: Option<&Value>) -> Value {
    let len = context.and_then(Value::as_str).map_or(0, crate::util::utf16_len);
    write_to_log(&format!("Missing tool reported (context length: {len})"));
    json!({"content": [{"type": "text", "text": GET_MORE_TOOLS_RESPONSE}]})
}

impl InjectConfig {
    pub(crate) fn new(
        tracing: bool,
        hook_mode: bool,
        agent_tracking: bool,
        tool_call_context: bool,
        context_description: Option<&str>,
        report_missing: bool,
    ) -> Self {
        Self {
            inject_session_id: tracing && !hook_mode,
            inject_agent_id: tracing && agent_tracking,
            inject_context: tool_call_context,
            context_description: context_description
                .filter(|d| !d.is_empty())
                .unwrap_or(DEFAULT_CONTEXT_PARAMETER_DESCRIPTION)
                .to_owned(),
            report_missing,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(agent: bool) -> InjectConfig {
        InjectConfig::new(true, false, agent, true, None, true)
    }

    #[test]
    fn composed_schemas_keep_handles_their_branches_declare() {
        // schemars emits oneOf for enums: a `session_id` field in one variant
        // is the customer's argument and must reach the handler.
        let mut schema: Map<String, Value> = serde_json::from_value(json!({
            "oneOf": [
                {"type": "object", "properties": {"session_id": {"type": "string"}, "cmd": {"type": "string"}}},
                {"type": "object", "properties": {"other": {"type": "string"}}}
            ]
        }))
        .unwrap();
        let r = inject_tool(&cfg(true), &InjectMemory::default(), "term", &mut schema, None);
        assert!(!r.strip.session_id);
        assert!(r.strip.agent_id);
        assert!(!r.strip.context);
    }

    #[test]
    fn composed_schemas_strip_undeclared_echoed_handles() {
        let mut schema: Map<String, Value> =
            serde_json::from_value(json!({"anyOf": [{"type": "object", "properties": {"a": {}}}], "$defs": {"X": {"properties": {"agent_id": {}}}}}))
                .unwrap();
        let r = inject_tool(&cfg(true), &InjectMemory::default(), "t", &mut schema, None);
        assert!(r.strip.session_id);
        assert!(!r.strip.agent_id, "declared in $defs");
    }
}

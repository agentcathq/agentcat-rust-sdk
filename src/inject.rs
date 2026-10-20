//! Tool-list injection: the `session_id`, `agent_id` and `context`
//! parameters, the `mcp_session` output property, the report-missing tool,
//! and argument stripping.
//!
//! `build_injected_list` is the TypeScript SDK's `buildInjectedList` on JSON
//! values, failure paths included: a tool whose schema JavaScript cannot
//! extend is listed unmodified, with the error JavaScript would raise. The
//! advertised tools and registries depend only on the configuration and the
//! tools, so a tool call on an instance that never served `tools/list` can
//! recompute exactly what a listing advertised.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Map, Value, json};

use crate::constants::{
    AGENT_ID_PARAM, AGENT_ID_PARAM_DESCRIPTION, CONTEXT_PARAM, DEFAULT_CONTEXT_PARAMETER_DESCRIPTION,
    GET_MORE_TOOLS_CONTEXT_DESCRIPTION, GET_MORE_TOOLS_DESCRIPTION, GET_MORE_TOOLS_NAME,
    GET_MORE_TOOLS_RESPONSE, GET_MORE_TOOLS_TITLE, MCP_SESSION_AGENT_ID_DESCRIPTION,
    MCP_SESSION_FIELD_DESCRIPTION, MCP_SESSION_FIELD_DESCRIPTION_HOOK_MODE, MCP_SESSION_KEY,
    MCP_SESSION_SESSION_ID_DESCRIPTION, MCP_SESSION_STATUS_DESCRIPTION, SESSION_ID_PARAM,
    SESSION_ID_PARAM_DESCRIPTION, SESSION_ID_PARAM_PATTERN,
};
use crate::js::{self, template_string, truthy};
use crate::logging::write_to_log;

/// Which AgentCat parameters were injected into one tool's input schema.
/// These are exactly the arguments stripped before the customer's handler
/// runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Injected {
    pub(crate) session_id: bool,
    pub(crate) agent_id: bool,
    pub(crate) context: bool,
}

#[cfg(test)]
impl Injected {
    /// The parameter names, in injection order.
    pub(crate) fn names(self) -> Vec<&'static str> {
        [
            (self.session_id, SESSION_ID_PARAM),
            (self.agent_id, AGENT_ID_PARAM),
            (self.context, CONTEXT_PARAM),
        ]
        .into_iter()
        .filter_map(|(set, name)| set.then_some(name))
        .collect()
    }
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

/// Grow-only memory of one `AgentCat`, shared by all of its tracked servers
/// and listings.
#[derive(Default)]
pub(crate) struct InjectMemory {
    /// Tools already reported for a `session_id` collision (log once).
    reported_conflicts: Mutex<HashSet<String>>,
    /// Tools whose schema declares `session_id` itself.
    declared_session_params: Mutex<HashSet<String>>,
    /// The customer registers its own `get_more_tools` (warn once).
    warned_customer_report_missing: AtomicBool,
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
        if !self.warned_customer_report_missing.swap(true, Ordering::Relaxed) {
            write_to_log(&format!(
                "WARN: This server registers its own \"{GET_MORE_TOOLS_NAME}\" tool, which is shadowed by AgentCat's report-missing tool: calls to it are answered by AgentCat and never reach your handler. Rename your tool, or set enableReportMissing: false to keep yours reachable."
            ));
        }
    }

    #[cfg(test)]
    pub(crate) fn reported_conflicts(&self) -> Vec<String> {
        sorted(&self.reported_conflicts)
    }

    #[cfg(test)]
    pub(crate) fn declared_session_params(&self) -> Vec<String> {
        sorted(&self.declared_session_params)
    }
}

#[cfg(test)]
fn sorted(set: &Mutex<HashSet<String>>) -> Vec<String> {
    let mut names: Vec<String> = set
        .lock()
        .map(|s| s.iter().cloned().collect())
        .unwrap_or_default();
    names.sort();
    names
}

/// What a listing advertises, and what it injected where.
#[derive(Debug, Default)]
pub(crate) struct InjectedList {
    pub(crate) tools: Vec<Value>,
    /// Per tool name: the parameters injected into its input schema.
    pub(crate) registry: HashMap<String, Injected>,
    /// Tools whose output schema declares the injected `mcp_session`.
    pub(crate) output_registry: HashSet<String>,
}

/// `target[key]` in JavaScript: only an object has named members here (a
/// named key on an array or a primitive reads `undefined`).
fn get<'a>(target: Option<&'a Value>, key: &str) -> Option<&'a Value> {
    target?.as_object()?.get(key)
}

/// `target[key] = value` in strict-mode JS, on JSON.
fn js_set(target: &mut Value, key: &str, value: Value) -> Result<(), String> {
    match target {
        // An existing key keeps its position, as in JS.
        Value::Object(map) => {
            map.insert(key.to_owned(), value);
            Ok(())
        }
        // JS stores a named property on the array object; JSON drops it.
        Value::Array(_) => Ok(()),
        Value::Null => Err(format!(
            "TypeError: Cannot set properties of null (setting '{key}')"
        )),
        other => Err(format!(
            "TypeError: Cannot create property '{key}' on {} '{}'",
            js::type_of(other),
            js::template_string(other)
        )),
    }
}

/// `tool.name || "unknown"`, as the logs print it and the registries key it.
fn tool_name(tool: &Value) -> String {
    match get(Some(tool), "name") {
        Some(name) if truthy(Some(name)) => template_string(name),
        _ => "unknown".to_owned(),
    }
}

/// `tool.name === "get_more_tools"`.
fn is_get_more_tools(tool: &Value) -> bool {
    get(Some(tool), "name").and_then(Value::as_str) == Some(GET_MORE_TOOLS_NAME)
}

fn is_composed(schema: Option<&Value>) -> bool {
    truthy(get(schema, "oneOf")) || truthy(get(schema, "allOf")) || truthy(get(schema, "anyOf"))
}

/// `schema.properties[param]` is truthy.
fn declares(schema: Option<&Value>, param: &str) -> bool {
    truthy(get(get(schema, "properties"), param))
}

/// The copy of the input schema both injectors extend: a missing schema
/// becomes an empty object schema, a missing `properties` becomes `{}`, and
/// `additionalProperties: false`, which would reject the injected
/// parameters, is removed.
fn prepare_input(schema: Option<&Value>) -> Result<Value, String> {
    let mut input = match schema {
        Some(schema) if truthy(Some(schema)) => schema.clone(),
        _ => json!({"type": "object", "properties": {}, "required": []}),
    };
    if !truthy(get(Some(&input), "properties")) {
        js_set(&mut input, "properties", Value::Object(Map::new()))?;
    }
    if let Value::Object(map) = &mut input
        && map.get("additionalProperties") == Some(&Value::Bool(false))
    {
        crate::util::shift_remove(map, "additionalProperties");
    }
    Ok(input)
}

/// `schema.properties[key] = value` on a schema `prepare_input` returned. On
/// an array schema, `properties` is the object JS stored as a named key of
/// the array, which JSON drops, so the write is lost with it.
fn set_property(schema: &mut Value, key: &str, value: Value) -> Result<(), String> {
    match schema.as_object_mut().and_then(|map| map.get_mut("properties")) {
        Some(properties) => js_set(properties, key, value),
        None => Ok(()),
    }
}

/// Appends `param` to `schema.required` unless listed; anything but an
/// array is replaced by `[param]`.
fn add_to_required(schema: &mut Value, param: &str) -> Result<(), String> {
    if let Some(Value::Array(required)) = schema.as_object_mut().and_then(|map| map.get_mut("required")) {
        if !required.iter().any(|v| v.as_str() == Some(param)) {
            required.push(Value::String(param.to_owned()));
        }
        return Ok(());
    }
    js_set(schema, "required", json!([param]))
}

/// Builds the advertised tool list: the report-missing tool appended, then
/// the handles, then `context`, so injected parameters follow the
/// customer's own in the order `session_id`, `agent_id`, `context`.
///
/// `tools` is never changed. A tool whose injection fails is listed as it
/// was before that step, and its registry records are rolled back.
pub(crate) fn build_injected_list(
    cfg: &InjectConfig,
    memory: &InjectMemory,
    tools: &[Value],
) -> InjectedList {
    let mut result = tools.to_vec();
    if cfg.report_missing {
        if result.iter().any(is_get_more_tools) {
            memory.record_customer_report_missing();
        } else {
            result.push(report_missing_descriptor());
        }
    }

    let mut list = InjectedList::default();
    if cfg.inject_session_id || cfg.inject_agent_id {
        for tool in &mut result {
            let name = tool_name(tool);
            let mut modified = tool.clone();
            match add_handles(cfg, memory, &mut modified, &name, &mut list) {
                Ok(()) => *tool = modified,
                Err(e) => {
                    list.registry.remove(&name);
                    list.output_registry.remove(&name);
                    write_to_log(&format!(
                        "WARN: Handle injection failed for tool \"{name}\"; listing it unmodified - {e}"
                    ));
                }
            }
        }
    }
    if cfg.inject_context {
        // get_more_tools carries its own `context` parameter.
        for tool in result.iter_mut().filter(|t| !is_get_more_tools(t)) {
            let name = tool_name(tool);
            let mut modified = tool.clone();
            match add_context(cfg, &mut modified, &name, &mut list.registry) {
                Ok(()) => *tool = modified,
                Err(e) => {
                    if let Some(entry) = list.registry.get_mut(&name) {
                        entry.context = false;
                    }
                    write_to_log(&format!(
                        "WARN: Context injection failed for tool \"{name}\"; listing it unmodified - {e}"
                    ));
                }
            }
        }
    }
    // A listed tool without injections still gets an (empty) entry: the
    // strip fallback applies only to tools never seen in a listing.
    let names = result
        .iter()
        .filter_map(|tool| get(Some(tool), "name"))
        .filter(|name| truthy(Some(name)));
    for name in names {
        list.registry.entry(template_string(name)).or_default();
    }
    list.tools = result;
    list
}

/// The handle step (TS `addHandleParametersToTool`) on a copy of one tool.
fn add_handles(
    cfg: &InjectConfig,
    memory: &InjectMemory,
    tool: &mut Value,
    name: &str,
    list: &mut InjectedList,
) -> Result<(), String> {
    let schema = get(Some(tool), "inputSchema");
    if is_composed(schema) {
        // Injection is skipped, but ownership is still recorded: a root
        // `session_id` in a composed schema belongs to the customer.
        if cfg.inject_session_id && declares(schema, SESSION_ID_PARAM) {
            memory.record_declared(name);
        }
        write_to_log(&format!(
            "WARN: Tool \"{name}\" has complex schema (oneOf/allOf/anyOf). Skipping handle injection."
        ));
        return Ok(());
    }
    let mut input = prepare_input(schema)?;

    if cfg.inject_session_id {
        if declares(Some(&input), SESSION_ID_PARAM) {
            memory.record_declared(name);
            if memory.first_conflict(name) {
                write_to_log(&format!(
                    "ERROR: Tool \"{name}\" already declares a '{SESSION_ID_PARAM}' parameter. AgentCat will not inject its own, and calls to this tool are published without a session, so they cannot be correlated. Your parameter is untouched and still reaches your handler. If you already manage sessions, pass a resolveSessionId hook to track() — AgentCat will derive its session from your identifier and stop injecting {SESSION_ID_PARAM} entirely."
                ));
            }
        } else {
            set_property(
                &mut input,
                SESSION_ID_PARAM,
                json!({
                    "type": "string",
                    "description": SESSION_ID_PARAM_DESCRIPTION,
                    "pattern": SESSION_ID_PARAM_PATTERN,
                }),
            )?;
            list.registry.entry(name.to_owned()).or_default().session_id = true;
            add_to_required(&mut input, SESSION_ID_PARAM)?;
        }
    }
    if cfg.inject_agent_id {
        if declares(Some(&input), AGENT_ID_PARAM) {
            write_to_log(&format!(
                "WARN: Tool \"{name}\" already has '{AGENT_ID_PARAM}' parameter. Skipping agent_id injection."
            ));
        } else {
            set_property(
                &mut input,
                AGENT_ID_PARAM,
                json!({"type": "string", "description": AGENT_ID_PARAM_DESCRIPTION}),
            )?;
            list.registry.entry(name.to_owned()).or_default().agent_id = true;
            add_to_required(&mut input, AGENT_ID_PARAM)?;
        }
    }
    js_set(tool, "inputSchema", input)?;
    add_mcp_session(cfg, tool, name, &mut list.output_registry)
}

/// Declares the optional `mcp_session` property in a plain output schema
/// (on a copy), so validating clients accept the mirrored field.
fn add_mcp_session(
    cfg: &InjectConfig,
    tool: &mut Value,
    name: &str,
    output_registry: &mut HashSet<String>,
) -> Result<(), String> {
    let schema = get(Some(tool), "outputSchema");
    let Some(schema) = schema.filter(|s| truthy(Some(s))) else {
        return Ok(());
    };
    if is_composed(Some(schema)) {
        write_to_log(&format!(
            "WARN: Tool \"{name}\" has complex outputSchema (oneOf/allOf/anyOf). Skipping {MCP_SESSION_KEY} injection; mint-back stays content-only for this tool."
        ));
        return Ok(());
    }
    let mut copy = schema.clone();
    if !truthy(get(Some(&copy), "properties")) {
        js_set(&mut copy, "properties", Value::Object(Map::new()))?;
    }
    if declares(Some(&copy), MCP_SESSION_KEY) {
        write_to_log(&format!(
            "WARN: Tool \"{name}\" already declares '{MCP_SESSION_KEY}' in outputSchema. Skipping injection."
        ));
        return Ok(());
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
    set_property(
        &mut copy,
        MCP_SESSION_KEY,
        json!({"type": "object", "description": description, "properties": sub}),
    )?;
    js_set(tool, "outputSchema", copy)?;
    output_registry.insert(name.to_owned());
    Ok(())
}

/// The context step (TS `addContextParameterToTool`) on a copy of one tool.
fn add_context(
    cfg: &InjectConfig,
    tool: &mut Value,
    name: &str,
    registry: &mut HashMap<String, Injected>,
) -> Result<(), String> {
    let schema = get(Some(tool), "inputSchema");
    if declares(schema, CONTEXT_PARAM) {
        write_to_log(&format!(
            "WARN: Tool \"{name}\" already has 'context' parameter. Skipping context injection."
        ));
        return Ok(());
    }
    if is_composed(schema) {
        write_to_log(&format!(
            "WARN: Tool \"{name}\" has complex schema (oneOf/allOf/anyOf). Skipping context injection."
        ));
        return Ok(());
    }
    let mut input = prepare_input(schema)?;
    set_property(
        &mut input,
        CONTEXT_PARAM,
        json!({"type": "string", "description": cfg.context_description}),
    )?;
    registry.entry(name.to_owned()).or_default().context = true;
    add_to_required(&mut input, CONTEXT_PARAM)?;
    js_set(tool, "inputSchema", input)
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

/// Removes the injected arguments in place; the other keys keep their order,
/// as after a JavaScript `delete`. The caller strips the handler's copy and
/// keeps the raw arguments for the event.
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

/// The canned answer to a `get_more_tools` call. The log line prints what
/// TS prints for `args?.context?.length ?? 0`.
pub(crate) fn report_missing_response(context: Option<&Value>) -> Value {
    let length = match context {
        Some(Value::String(s)) => crate::util::utf16_len(s).to_string(),
        Some(Value::Array(items)) => items.len().to_string(),
        Some(Value::Object(map)) => match map.get("length") {
            None | Some(Value::Null) => "0".to_owned(),
            Some(length) => template_string(length),
        },
        _ => "0".to_owned(),
    };
    write_to_log(&format!("Missing tool reported (context length: {length})"));
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

    #[test]
    fn js_set_fails_like_strict_mode_javascript() {
        let mut null = Value::Null;
        assert_eq!(
            js_set(&mut null, "x", json!(1)),
            Err("TypeError: Cannot set properties of null (setting 'x')".to_owned())
        );
        let mut float = json!(2.5);
        assert_eq!(
            js_set(&mut float, "x", json!(1)),
            Err("TypeError: Cannot create property 'x' on number '2.5'".to_owned())
        );
        let mut array = json!([1]);
        assert_eq!(js_set(&mut array, "x", json!(1)), Ok(()));
        assert_eq!(array, json!([1]));
        let mut object = json!({"a": 1, "x": 0, "b": 2});
        assert_eq!(js_set(&mut object, "x", json!(1)), Ok(()));
        assert_eq!(serde_json::to_string(&object).unwrap(), r#"{"a":1,"x":1,"b":2}"#);
    }

    #[test]
    fn tool_names_print_as_javascript_template_strings() {
        assert_eq!(tool_name(&json!({"name": "t"})), "t");
        assert_eq!(tool_name(&json!({"name": 5})), "5");
        assert_eq!(tool_name(&json!({"name": ""})), "unknown");
        assert_eq!(tool_name(&json!({"name": 0})), "unknown");
        assert_eq!(tool_name(&json!({})), "unknown");
    }
}

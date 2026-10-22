//! The tool-list pipeline of the TS SDK, `buildInjectedList` (B-031 to B-048), on rmcp tools: the
//! `get_more_tools` tool, the `session_id` and `agent_id` parameters, the `mcp_session` property of
//! output schemas and the `context` parameter, with TS's skips, log lines and per-tool fault
//! isolation. Agents read the injected schemas, so they are TS's, key order included.
//!
//! TS reads the schemas with JavaScript semantics, and so does this module: a key is "declared"
//! when its value is truthy (B-043), a composition key counts when its value is truthy, and
//! setting a property on a `properties` value that is a string, number or boolean throws
//! JavaScript's `TypeError`, which TS turns into a log line and an unmodified tool (B-044).
//! `inject/tests.rs` checks the pipeline against `tests/vectors/inject.json` and the JS oracle.
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use rmcp::model::{JsonObject, Tool};
use serde_json::{Map, Value, json};

use crate::config::Options;
use crate::constants::{
    AGENT_ID_PARAM, AGENT_ID_PARAM_DESCRIPTION, GET_MORE_TOOLS_NAME,
    MCP_SESSION_AGENT_ID_DESCRIPTION, MCP_SESSION_FIELD_DESCRIPTION,
    MCP_SESSION_FIELD_DESCRIPTION_HOOK_MODE, MCP_SESSION_KEY, MCP_SESSION_SESSION_ID_DESCRIPTION,
    MCP_SESSION_STATUS_DESCRIPTION, SESSION_ID_PARAM, SESSION_ID_PARAM_DESCRIPTION,
    SESSION_ID_PARAM_PATTERN, get_more_tools_descriptor,
};
use crate::js;

#[cfg(test)]
mod tests;

/// The name of the injected `context` parameter.
pub(crate) const CONTEXT_PARAM: &str = "context";

/// The registry name TS gives a tool whose name is empty (`name || "unknown"`).
const UNKNOWN_TOOL: &str = "unknown";

/// The composition keywords that make a schema "complex" when their value is truthy.
const COMPOSITION_KEYS: [&str; 3] = ["oneOf", "allOf", "anyOf"];

/// The warning TS logs once per tracking-data object when the listing has its own
/// `get_more_tools` while report-missing is on (B-028).
const SHADOW_WARNING: &str = "WARN: This server registers its own \"get_more_tools\" tool, which is shadowed by AgentCat's report-missing tool: calls to it are answered by AgentCat and never reach your handler. Rename your tool, or set enableReportMissing: false to keep yours reachable.";

/// What the pipeline injects, from the [`Options`] (B-031).
#[derive(Debug, Clone, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "the four switches of the TS pipeline (B-031)"
)]
pub(crate) struct InjectConfig {
    /// Whether listings get `get_more_tools`.
    pub(crate) report_missing: bool,
    /// Whether tools get `session_id`: tracing on and no hook mode.
    pub(crate) inject_session_id: bool,
    /// Whether tools get `agent_id`: tracing and agent tracking on.
    pub(crate) inject_agent_id: bool,
    /// Whether tools get `context`.
    pub(crate) inject_context: bool,
    /// The description of the `context` parameter: the custom one, or the default.
    pub(crate) context_description: String,
}

impl InjectConfig {
    /// The pipeline settings of `options` with hook mode `hook_mode` (B-031, B-064):
    /// `session_id` with tracing and without hook mode, `agent_id` with tracing and agent
    /// tracking, `context` with the tool-call context.
    pub(crate) fn new(options: &Options, hook_mode: bool) -> Self {
        let tracing = options.tracing();
        Self {
            report_missing: options.report_missing(),
            inject_session_id: tracing && !hook_mode,
            inject_agent_id: tracing && options.agent_tracking(),
            inject_context: options.tool_call_context(),
            context_description: options.context_description().to_owned(),
        }
    }

    /// The pipeline settings of `options`, in the hook mode they configure.
    pub(crate) fn from_options(options: &Options) -> Self {
        Self::new(options, options.hook_mode())
    }

    /// Whether the handle step runs: it injects `session_id`, `agent_id` or both (B-031 step 3).
    fn injects_handles(&self) -> bool {
        self.inject_session_id || self.inject_agent_id
    }
}

/// What the pipeline recorded for one tool name: TS's registry entry (the injected parameters),
/// its output-registry membership and whether the tool declares its own `session_id`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ToolEntry {
    /// The parameters injected into the input schema, in the order TS records them: `session_id`,
    /// `agent_id`, `context` (two tools of one name on one page share TS's record, in the order
    /// their parameters were recorded).
    pub(crate) injected: Vec<&'static str>,
    /// Whether the output schema got `mcp_session`, which gates the structured mirror (B-047).
    pub(crate) output_injected: bool,
    /// Whether the input schema declares its own `session_id` while the SDK injects one, so a
    /// call's `session_id` is the customer's (B-040).
    pub(crate) declares_session: bool,
}

/// The state TS keeps per tracking-data object across listings (B-028, B-040).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct InjectState {
    /// The tools whose input schema declared its own `session_id`; it only grows.
    pub(crate) declared_session_params: BTreeSet<String>,
    /// The tools whose `session_id` collision has been logged, so it is logged once.
    pub(crate) reported_conflicts: BTreeSet<String>,
    /// Whether the `get_more_tools` shadow warning has been logged.
    pub(crate) shadow_reported: bool,
}

/// One injected listing page.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct InjectedList {
    /// The tools to list, `get_more_tools` appended when report-missing adds it.
    pub(crate) tools: Vec<Tool>,
    /// One entry per tool name, by registry name, in name order. A tool named `""` is recorded as
    /// `"unknown"`, as TS records it, and only when TS's registry has that name.
    pub(crate) entries: Vec<(String, ToolEntry)>,
    /// The TS log lines of the listing, in TS's order.
    pub(crate) logs: Vec<String>,
}

/// TS's `buildInjectedList` on one listing page (B-031 to B-048): appends `get_more_tools` (or
/// logs the shadow warning once, B-028), runs the handle step over every tool, then the context
/// step over every tool but `get_more_tools`, and gives every listed tool with a name an entry.
/// The steps of each tool are those of [`inject_tool`]; the listing runs them step by step over
/// all tools, as TS does, so its log lines come in TS's order and tools that share a name share
/// their record. Schemas are edited with `Arc::make_mut`, so a schema the caller still holds stays
/// as it was.
pub(crate) fn build_injected_list(
    config: &InjectConfig,
    mut tools: Vec<Tool>,
    state: &mut InjectState,
) -> InjectedList {
    let mut logs = Vec::new();
    if config.report_missing {
        if tools.iter().any(|tool| tool.name == GET_MORE_TOOLS_NAME) {
            if !state.shadow_reported {
                state.shadow_reported = true;
                logs.push(SHADOW_WARNING.to_owned());
            }
        } else {
            tools.push(get_more_tools_tool());
        }
    }
    let mut registry = PageRegistry::default();
    if config.injects_handles() {
        let mut injected = Vec::with_capacity(tools.len());
        for tool in tools {
            let key = registry_name(&tool).to_owned();
            let (tool, step) = inject_handles(config, tool, state, &mut logs);
            registry.handles(key, step);
            injected.push(tool);
        }
        tools = injected;
    }
    if config.inject_context {
        let mut injected = Vec::with_capacity(tools.len());
        for tool in tools {
            if tool.name == GET_MORE_TOOLS_NAME {
                injected.push(tool);
                continue;
            }
            let key = registry_name(&tool).to_owned();
            let (tool, step) = inject_context(config, tool, &mut logs);
            registry.context(key, step);
            injected.push(tool);
        }
        tools = injected;
    }
    for tool in &tools {
        if !tool.name.is_empty() {
            registry.params.entry(tool.name.to_string()).or_default();
        }
    }
    InjectedList {
        tools,
        entries: registry.into_entries(),
        logs,
    }
}

/// The steps of [`build_injected_list`] for one tool: the handle step, then the context step
/// (except for `get_more_tools`). Its log lines go to `logs`; its entry is what a listing of this
/// tool alone records.
pub(crate) fn inject_tool(
    config: &InjectConfig,
    tool: Tool,
    state: &mut InjectState,
    logs: &mut Vec<String>,
) -> (Tool, ToolEntry) {
    let (mut tool, mut entry) = if config.injects_handles() {
        let (tool, step) = inject_handles(config, tool, state, logs);
        (tool, step.entry)
    } else {
        (tool, ToolEntry::default())
    };
    if config.inject_context && tool.name != GET_MORE_TOOLS_NAME {
        let (injected, step) = inject_context(config, tool, logs);
        if step == ContextStep::Injected {
            entry.injected.push(CONTEXT_PARAM);
        }
        tool = injected;
    }
    (tool, entry)
}

/// The `get_more_tools` tool, from `getReportMissingToolDescriptor()` (B-045).
pub(crate) fn get_more_tools_tool() -> Tool {
    // The descriptor is a valid tool (`inject::tests::get_more_tools_tool_is_the_ts_descriptor`),
    // so the default never appears.
    serde_json::from_value(get_more_tools_descriptor()).unwrap_or_default()
}

/// TS's registries of one page: the injected parameters by name (`InjectedParamsRegistry`), the
/// output registry, and the names whose tool declared its own `session_id` on this page.
#[derive(Default)]
struct PageRegistry {
    params: BTreeMap<String, Vec<&'static str>>,
    output: BTreeSet<String>,
    declares_session: BTreeSet<String>,
}

impl PageRegistry {
    /// Records the handle step of the tool `key`: its parameters join the name's record, or, when
    /// the step failed, the name's records are dropped, as TS deletes them (B-044).
    fn handles(&mut self, key: String, step: HandleStep) {
        if step.entry.declares_session {
            self.declares_session.insert(key.clone());
        }
        if step.failed {
            self.params.remove(&key);
            self.output.remove(&key);
            return;
        }
        if step.entry.output_injected {
            self.output.insert(key.clone());
        }
        if !step.entry.injected.is_empty() {
            let params = self.params.entry(key).or_default();
            for param in step.entry.injected {
                if !params.contains(&param) {
                    params.push(param);
                }
            }
        }
    }

    /// Records the context step of the tool `key`: `context` joins the name's record, or leaves
    /// it when the step failed (B-044).
    fn context(&mut self, key: String, step: ContextStep) {
        match step {
            ContextStep::Untouched => {}
            ContextStep::Injected => {
                let params = self.params.entry(key).or_default();
                if !params.contains(&CONTEXT_PARAM) {
                    params.push(CONTEXT_PARAM);
                }
            }
            ContextStep::Failed => {
                if let Some(params) = self.params.get_mut(&key) {
                    params.retain(|param| *param != CONTEXT_PARAM);
                }
            }
        }
    }

    /// One entry per name of the parameter registry. TS's output registry can hold `"unknown"`
    /// alone (a tool named `""` whose handle step injected no parameter but extended its output
    /// schema); no call reaches a tool by that name, so it gets no entry.
    fn into_entries(self) -> Vec<(String, ToolEntry)> {
        let Self {
            params,
            output,
            declares_session,
        } = self;
        params
            .into_iter()
            .map(|(name, injected)| {
                let entry = ToolEntry {
                    injected,
                    output_injected: output.contains(&name),
                    declares_session: declares_session.contains(&name),
                };
                (name, entry)
            })
            .collect()
    }
}

/// What the handle step did to one tool.
struct HandleStep {
    /// The handle parameters it injected, whether it extended the output schema, and whether the
    /// tool declares its own `session_id`. After a failure only the last is kept.
    entry: ToolEntry,
    /// Whether the step failed, so the tool is listed as it came (B-044).
    failed: bool,
}

/// What the context step did to one tool.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContextStep {
    /// The tool declares `context` or has a composed schema.
    Untouched,
    /// The input schema got `context`.
    Injected,
    /// The step failed, so the tool is listed as it came (B-044).
    Failed,
}

/// The name TS keys a tool by in its registries and log lines: its name, or `"unknown"` when the
/// name is empty.
fn registry_name(tool: &Tool) -> &str {
    if tool.name.is_empty() {
        UNKNOWN_TOOL
    } else {
        &tool.name
    }
}

/// The handle step of `addHandleParametersToTool` (B-032 to B-034, B-037 to B-041, B-043,
/// B-044, B-047). A composed input schema is left alone, with a warning; it still declares its
/// own `session_id` when its root `properties` has a truthy one. Otherwise the step works on a
/// copy and gives the tool as it came when the copy throws.
fn inject_handles(
    config: &InjectConfig,
    tool: Tool,
    state: &mut InjectState,
    logs: &mut Vec<String>,
) -> (Tool, HandleStep) {
    let key = registry_name(&tool).to_owned();
    let mut entry = ToolEntry::default();
    if is_composed(&tool.input_schema) {
        if config.inject_session_id && declares(&tool.input_schema, SESSION_ID_PARAM) {
            state.declared_session_params.insert(key.clone());
            entry.declares_session = true;
        }
        logs.push(format!(
            "WARN: Tool \"{key}\" has complex schema (oneOf/allOf/anyOf). Skipping handle injection."
        ));
        return (
            tool,
            HandleStep {
                entry,
                failed: false,
            },
        );
    }
    // TS's `{ ...tool }`: the copy shares the schemas until `Arc::make_mut` copies one, and
    // `tool` stays as it came for a failure.
    let mut injected = tool.clone();
    match add_handles(config, &mut injected, &key, &mut entry, state, logs) {
        Ok(()) => (
            injected,
            HandleStep {
                entry,
                failed: false,
            },
        ),
        Err(error) => {
            logs.push(format!(
                "WARN: Handle injection failed for tool \"{key}\"; listing it unmodified - {error}"
            ));
            let entry = ToolEntry {
                declares_session: entry.declares_session,
                ..ToolEntry::default()
            };
            (
                tool,
                HandleStep {
                    entry,
                    failed: true,
                },
            )
        }
    }
}

/// The body of the handle step on a tool whose input schema is not composed: `properties` made
/// an object when falsy, `additionalProperties: false` dropped, `session_id` and `agent_id` added
/// unless declared, then the output schema's `mcp_session`. `Err` holds the `TypeError` text.
fn add_handles(
    config: &InjectConfig,
    tool: &mut Tool,
    key: &str,
    entry: &mut ToolEntry,
    state: &mut InjectState,
    logs: &mut Vec<String>,
) -> Result<(), String> {
    let schema = Arc::make_mut(&mut tool.input_schema);
    ensure_properties(schema);
    drop_additional_false(schema);
    if config.inject_session_id {
        if declares(schema, SESSION_ID_PARAM) {
            entry.declares_session = true;
            state.declared_session_params.insert(key.to_owned());
            if state.reported_conflicts.insert(key.to_owned()) {
                logs.push(format!(
                    "ERROR: Tool \"{key}\" already declares a '{SESSION_ID_PARAM}' parameter. AgentCat will not inject its own, and calls to this tool are published without a session, so they cannot be correlated. Your parameter is untouched and still reaches your handler. If you already manage sessions, pass a resolveSessionId hook to track() \u{2014} AgentCat will derive its session from your identifier and stop injecting {SESSION_ID_PARAM} entirely."
                ));
            }
        } else {
            let property = json!({
                "type": "string",
                "description": SESSION_ID_PARAM_DESCRIPTION,
                "pattern": SESSION_ID_PARAM_PATTERN,
            });
            set_property(ensure_properties(schema), SESSION_ID_PARAM, property)?;
            entry.injected.push(SESSION_ID_PARAM);
            add_to_required(schema, SESSION_ID_PARAM);
        }
    }
    if config.inject_agent_id {
        if declares(schema, AGENT_ID_PARAM) {
            logs.push(format!(
                "WARN: Tool \"{key}\" already has '{AGENT_ID_PARAM}' parameter. Skipping agent_id injection."
            ));
        } else {
            let property = json!({"type": "string", "description": AGENT_ID_PARAM_DESCRIPTION});
            set_property(ensure_properties(schema), AGENT_ID_PARAM, property)?;
            entry.injected.push(AGENT_ID_PARAM);
            add_to_required(schema, AGENT_ID_PARAM);
        }
    }
    if let Some(output) = &mut tool.output_schema {
        entry.output_injected = add_mcp_session(config, output, key, logs)?;
    }
    Ok(())
}

/// The output-schema part of the handle step (`addMcpSessionToOutputSchema`, B-047): `Ok(true)`
/// when `mcp_session` was added as the last property. A composed schema or one that declares
/// `mcp_session` is left alone, with a warning.
fn add_mcp_session(
    config: &InjectConfig,
    output: &mut Arc<JsonObject>,
    key: &str,
    logs: &mut Vec<String>,
) -> Result<bool, String> {
    if is_composed(output) {
        logs.push(format!(
            "WARN: Tool \"{key}\" has complex outputSchema (oneOf/allOf/anyOf). Skipping {MCP_SESSION_KEY} injection; mint-back stays content-only for this tool."
        ));
        return Ok(false);
    }
    // TS gives its copy `properties: {}` before this check; a falsy `properties` declares nothing.
    if declares(output, MCP_SESSION_KEY) {
        logs.push(format!(
            "WARN: Tool \"{key}\" already declares '{MCP_SESSION_KEY}' in outputSchema. Skipping injection."
        ));
        return Ok(false);
    }
    let mut properties = Map::new();
    if config.inject_session_id {
        properties.insert(
            SESSION_ID_PARAM.to_owned(),
            json!({"type": "string", "description": MCP_SESSION_SESSION_ID_DESCRIPTION}),
        );
    }
    if config.inject_agent_id {
        properties.insert(
            AGENT_ID_PARAM.to_owned(),
            json!({"type": "string", "description": MCP_SESSION_AGENT_ID_DESCRIPTION}),
        );
    }
    if config.inject_session_id {
        properties.insert(
            "status".to_owned(),
            json!({
                "type": "string",
                "enum": ["issued", "active", "unrecognized"],
                "description": MCP_SESSION_STATUS_DESCRIPTION,
            }),
        );
    }
    let description = if config.inject_session_id {
        MCP_SESSION_FIELD_DESCRIPTION
    } else {
        MCP_SESSION_FIELD_DESCRIPTION_HOOK_MODE
    };
    let property = json!({"type": "object", "description": description, "properties": properties});
    set_property(
        ensure_properties(Arc::make_mut(output)),
        MCP_SESSION_KEY,
        property,
    )?;
    Ok(true)
}

/// The context step of `addContextParameterToTool` (B-035, B-037, B-038, B-042 to B-044): a
/// tool that declares `context` is left alone, then one with a composed schema, each with a
/// warning; otherwise `properties` is made an object when falsy, `additionalProperties: false`
/// dropped, and `context` added last and to `required`.
fn inject_context(
    config: &InjectConfig,
    tool: Tool,
    logs: &mut Vec<String>,
) -> (Tool, ContextStep) {
    let key = registry_name(&tool);
    if declares(&tool.input_schema, CONTEXT_PARAM) {
        logs.push(format!(
            "WARN: Tool \"{key}\" already has '{CONTEXT_PARAM}' parameter. Skipping context injection."
        ));
        return (tool, ContextStep::Untouched);
    }
    if is_composed(&tool.input_schema) {
        logs.push(format!(
            "WARN: Tool \"{key}\" has complex schema (oneOf/allOf/anyOf). Skipping context injection."
        ));
        return (tool, ContextStep::Untouched);
    }
    let mut injected = tool.clone();
    let schema = Arc::make_mut(&mut injected.input_schema);
    ensure_properties(schema);
    drop_additional_false(schema);
    let property = json!({"type": "string", "description": config.context_description});
    match set_property(ensure_properties(schema), CONTEXT_PARAM, property) {
        Ok(()) => {
            add_to_required(schema, CONTEXT_PARAM);
            (injected, ContextStep::Injected)
        }
        Err(error) => {
            logs.push(format!(
                "WARN: Context injection failed for tool \"{key}\"; listing it unmodified - {error}"
            ));
            (tool, ContextStep::Failed)
        }
    }
}

/// JavaScript's truthiness of a JSON value: `false`, `0`, `""` and `null` are falsy.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// Whether `schema` has a truthy `oneOf`, `allOf` or `anyOf` (B-039).
fn is_composed(schema: &JsonObject) -> bool {
    COMPOSITION_KEYS
        .iter()
        .any(|key| schema.get(*key).is_some_and(truthy))
}

/// Whether the `properties` of `schema` declare `name` as TS checks it: `properties[name]` is
/// truthy (B-043). Only an object holds the name: JavaScript finds none of the injected names on a
/// string, number, boolean or array.
fn declares(schema: &JsonObject, name: &str) -> bool {
    matches!(schema.get("properties"), Some(Value::Object(properties)) if properties.get(name).is_some_and(truthy))
}

/// TS's `if (!schema.properties) schema.properties = {}`: a missing `properties` is appended, a
/// falsy one replaced in its place. Gives the `properties` value.
fn ensure_properties(schema: &mut JsonObject) -> &mut Value {
    let properties = schema
        .entry("properties")
        .or_insert_with(|| Value::Object(Map::new()));
    if !truthy(properties) {
        *properties = Value::Object(Map::new());
    }
    properties
}

/// TS's `delete schema.additionalProperties` when it is `false` (B-038); the other keys keep
/// their order.
fn drop_additional_false(schema: &mut JsonObject) {
    if schema.get("additionalProperties") == Some(&Value::Bool(false)) {
        schema.shift_remove("additionalProperties");
    }
}

/// `properties[name] = value` in JavaScript's strict mode: an object takes the key (in its place
/// when it has it already), an array takes it as a named property that JSON never shows, and a
/// string, number or boolean throws the `TypeError` whose text is the `Err` (`ensure_properties`
/// has replaced a falsy value, so no `null` gets here).
fn set_property(properties: &mut Value, name: &str, value: Value) -> Result<(), String> {
    match properties {
        Value::Object(map) => {
            map.insert(name.to_owned(), value);
            Ok(())
        }
        Value::Array(_) => Ok(()),
        other => Err(format!(
            "TypeError: Cannot create property '{name}' on {} '{}'",
            js::type_of(Some(other)),
            js::to_js_string(Some(other)),
        )),
    }
}

/// TS's `addToRequired`: `param` appended to a `required` list that lacks it; any other
/// `required` (absent or not a list) becomes `[param]`, in its place when present.
fn add_to_required(schema: &mut JsonObject, param: &'static str) {
    if let Some(Value::Array(required)) = schema.get_mut("required") {
        if !required.iter().any(|item| item.as_str() == Some(param)) {
            required.push(json!(param));
        }
        return;
    }
    schema.insert("required".to_owned(), json!([param]));
}

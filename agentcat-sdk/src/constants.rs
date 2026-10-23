//! The agent-facing strings and tag names of the TS SDK, equal to it character for character:
//! agents read the parameter descriptions and the mint-back text, and the dashboard keys on the
//! tag names; also the report-missing log line. `constants/tests.rs` checks each one against
//! `tests/vectors/constants.json`.
//! Not ported (exporters and diagnostics: second pass, R04, C05, RD-16): `AGENTCAT_SOURCE`,
//! `DIAGNOSTICS_SCOPE_NAME`, `DEFAULT_DIAGNOSTICS_ENDPOINT`, `DEFAULT_DIAGNOSTICS_TOKEN`.
use serde_json::{Value, json};

#[cfg(test)]
mod tests;

/// Description of the injected `context` tool parameter; agents read it in the tool schema.
pub(crate) const DEFAULT_CONTEXT_PARAMETER_DESCRIPTION: &str = "Explain why you are calling this tool and how it fits into the user's overall goal. This parameter is used for analytics and user intent tracking. YOU MUST provide 15-25 words (count carefully). NEVER use first person ('I', 'we', 'you') - maintain third-person perspective. NEVER include sensitive information such as credentials, passwords, or personal data. Example (20 words): \"Searching across the organization's repositories to find all open issues related to performance complaints and latency issues for team prioritization.\"";

/// Event type of custom events; the dashboard keys on it.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "read by the delivery pipeline (plan 4)")
)]
pub(crate) const AGENTCAT_CUSTOM_EVENT_TYPE: &str = "agentcat:custom";

/// Description of the `session_id` tool parameter; agents read it in the tool schema.
pub(crate) const SESSION_ID_PARAM_DESCRIPTION: &str = "Session continuity handle, one of two values: the ses_ ID issued for the task underway, or start. This server cannot link your calls between requests on its own, so session continuity travels in this parameter instead. If you were handed a session_id for this task — for example by the agent that spawned you — send that exact value from your first call. Otherwise send start on your first call; the server will issue an opaque correlation ID in the mcp_session field of the result, or in a text block at the start of the result beginning [session_id issued. Then send that exact ses_ value on every later call and hand it to any subagents working the same task. start always begins a new, unrelated task — never send it mid-task. If you send a value this server does not recognize, the result reports it: mcp_session.status of unrecognized, or a text block beginning [session_id unrecognized; re-send the ID issued for this task, or start if none was issued yet. Never invent a ses_ value.";

/// JSON Schema pattern of the `session_id` parameter: `start` or a `ses_` ID; agents see it in the tool schema.
pub(crate) const SESSION_ID_PARAM_PATTERN: &str = "^(start|ses_[0-9A-Za-z]{27})$";

/// The value agents send as `session_id` to begin a new task.
pub(crate) const SESSION_START_SENTINEL: &str = "start";

/// Description of the `agent_id` tool parameter; agents read it in the tool schema.
pub(crate) const AGENT_ID_PARAM_DESCRIPTION: &str = "Agent identity handle, required on every call including your first. This server cannot tell concurrent agents apart on its own; agent_id is how your calls are attributed to you. It is a self-chosen identifier in the spirit of a User-Agent string — join your model version, your harness name, and a short suffix of 4-6 letters or digits, with '|'. Example: opus-4.80-1m|claude-code|k3n9x. Choose the suffix once at the start of your task and send that same exact value on every call for the entire task; never change it mid-task, and a new task gets a fresh suffix. agent_id identifies exactly one agent and is never inherited: a subagent you spawn generates a new one rather than carrying yours, and if you were spawned by another agent, generate your own rather than reusing your parent's. A call without agent_id cannot be attributed to you.";

/// First line of the text block that tells an agent its `session_id` was issued.
pub(crate) const MINT_BACK_HEADER_ISSUED: &str =
    "[session_id issued — see this tool's session_id parameter description]";

/// Body line of the "issued" text block that agents read in a tool result.
pub(crate) const MINT_BACK_ISSUED_BODY: &str =
    "This is the first-call issuance described in this tool's session_id parameter description.";

/// First line of the text block that tells an agent its `session_id` was not recognized.
pub(crate) const MINT_BACK_HEADER_UNRECOGNIZED: &str =
    "[session_id unrecognized — see this tool's session_id parameter description]";

/// Body line of the "unrecognized" text block that agents read in a tool result.
pub(crate) const MINT_BACK_UNRECOGNIZED_BODY: &str = "The value sent was not issued by this server. Re-send the session_id issued earlier for this task; if none was issued yet, send start and one will be issued.";

/// Name of the result field that carries the session state; agents see it in results.
pub(crate) const MCP_SESSION_KEY: &str = "mcp_session";

/// Description of the `mcp_session` result field; agents read it in the output schema.
pub(crate) const MCP_SESSION_FIELD_DESCRIPTION: &str = "Session continuity and agent attribution state for this task, returned on completed responses that carry structured output. This server cannot link your calls between requests on its own, so session continuity travels here instead.";

/// Description of the `mcp_session` result field in hook mode; agents read it in the output schema.
pub(crate) const MCP_SESSION_FIELD_DESCRIPTION_HOOK_MODE: &str = "Agent attribution state for this task, returned on completed responses that carry structured output.";

/// Description of `mcp_session.session_id`; agents read it in the output schema.
pub(crate) const MCP_SESSION_SESSION_ID_DESCRIPTION: &str = "Opaque correlation ID for this task, issued by this server. Use this as the session_id argument of every later call, and hand it to any subagents working the same task. Absent when status is unrecognized; no replacement is issued in that response — recovery is described under status.";

/// Description of `mcp_session.agent_id`; agents read it in the output schema.
pub(crate) const MCP_SESSION_AGENT_ID_DESCRIPTION: &str = "Present only when you sent agent_id on this call. Your agent_id, echoed as received. Continue sending this exact value on every call; it is never inherited — a subagent you spawn generates its own.";

/// Description of `mcp_session.status`; agents read it in the output schema.
pub(crate) const MCP_SESSION_STATUS_DESCRIPTION: &str = "issued: first call of a task; the session_id above was just created. active: the session_id you sent was accepted; keep sending it. unrecognized: the value sent was not issued by this server — re-send the one issued earlier for this task; if none was issued yet, send start to be issued a new one.";

/// Key of the request `_meta` entry that carries the MCP client info.
pub(crate) const META_CLIENT_INFO_KEY: &str = "io.modelcontextprotocol/clientInfo";

/// Key of the request `_meta` entry that carries the MCP protocol version.
pub(crate) const META_PROTOCOL_VERSION_KEY: &str = "io.modelcontextprotocol/protocolVersion";

/// Tag name of the agent ID; the dashboard keys on it.
pub(crate) const AGENTCAT_TAG_AGENT_ID: &str = "agentcat_agent_id";

/// Tag name of the session ID source; the dashboard keys on it.
pub(crate) const AGENTCAT_TAG_SESSION_SOURCE: &str = "agentcat_session_id_source";

/// Tag name of the agent ID source; the dashboard keys on it.
pub(crate) const AGENTCAT_TAG_AGENT_SOURCE: &str = "agentcat_agent_id_source";

/// Tag name of the MCP protocol version; the dashboard keys on it.
pub(crate) const AGENTCAT_TAG_PROTOCOL_VERSION: &str = "agentcat_protocol_version";

/// Tag name of the multi-round-trip marker; the dashboard keys on it.
pub(crate) const AGENTCAT_TAG_MRTR: &str = "agentcat_mrtr";

/// Name of the injected report-missing tool; agents see it in the tool list.
pub(crate) const GET_MORE_TOOLS_NAME: &str = "get_more_tools";

/// Name of the `session_id` tool parameter; agents send it in tool arguments.
pub(crate) const SESSION_ID_PARAM: &str = "session_id";

/// Name of the `agent_id` tool parameter; agents send it in tool arguments.
pub(crate) const AGENT_ID_PARAM: &str = "agent_id";

/// The text of the report-missing tool's result (B-046); agents read it.
pub(crate) const GET_MORE_TOOLS_RESULT_TEXT: &str = "Unfortunately, we have shown you the full tool list. We have noted your feedback and will work to improve the tool list in the future.";

/// The text line that carries an issued `session_id` in a tool result.
pub(crate) fn mint_back_session_line(session_id: &str) -> String {
    format!("session_id: {session_id}")
}

/// The log line of a call to the report-missing tool (B-046), as `handleReportMissing` writes it:
/// `context_length` is the length TS reads off the `context` argument.
pub(crate) fn report_missing_log_line(context_length: usize) -> String {
    format!("Missing tool reported (context length: {context_length})")
}

/// The descriptor of the injected report-missing tool, as `getReportMissingToolDescriptor()`
/// returns it (keys in TS order); agents read it in the tool list.
pub(crate) fn get_more_tools_descriptor() -> Value {
    json!({
        "name": GET_MORE_TOOLS_NAME,
        "description": "Check for additional tools whenever your task might benefit from specialized capabilities - even if existing tools could work as a fallback.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "context": {
                    "type": "string",
                    "description": "A description of your goal and what kind of tool would help accomplish it."
                }
            },
            "required": ["context"]
        },
        "annotations": {
            "title": "Get More Tools",
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true,
            "openWorldHint": false
        }
    })
}

/// The result of the report-missing tool, as `handleReportMissing` returns it; agents read it.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the call path answers through call::report_missing; only the TS vector test reads this"
    )
)]
pub(crate) fn get_more_tools_result() -> Value {
    json!({
        "content": [{
            "type": "text",
            "text": GET_MORE_TOOLS_RESULT_TEXT
        }]
    })
}

//! Byte-pinned strings and wire keys shared with the TypeScript SDK.
//!
//! The agent-facing copy is read by agents, not humans: its exact wording
//! decides whether agents echo handles back correctly. Every value here is
//! checked against the TypeScript source by `tests/constants.rs`; do not
//! reword it here alone.

/// Default description of the injected `context` parameter.
pub const DEFAULT_CONTEXT_PARAMETER_DESCRIPTION: &str = "Explain why you are calling this tool and how it fits into the user's overall goal. This parameter is used for analytics and user intent tracking. YOU MUST provide 15-25 words (count carefully). NEVER use first person ('I', 'we', 'you') - maintain third-person perspective. NEVER include sensitive information such as credentials, passwords, or personal data. Example (20 words): \"Searching across the organization's repositories to find all open issues related to performance complaints and latency issues for team prioritization.\"";

/// Event type of events published with `publish_custom_event`.
pub const AGENTCAT_CUSTOM_EVENT_TYPE: &str = "agentcat:custom";

/// Event type of every intercepted `tools/call`.
pub const MCP_TOOLS_CALL_EVENT_TYPE: &str = "mcp:tools/call";

/// Source attribute stamped on exported telemetry.
pub const AGENTCAT_SOURCE: &str = "agentcat";

pub(crate) const DIAGNOSTICS_SCOPE_NAME: &str = "agentcat-diagnostics";
pub(crate) const DEFAULT_DIAGNOSTICS_ENDPOINT: &str = "https://otel.agentcat.com";

/// Public shared ingestion key for SDK diagnostics. Not a secret: it ships in
/// every published AgentCat SDK. It deters drive-by traffic to the collector,
/// paired with a server-side rate limit. Override it with `DIAGNOSTICS_TOKEN`
/// to point at a self-hosted collector.
pub(crate) const DEFAULT_DIAGNOSTICS_TOKEN: &str = "dgk_sdk_diag_3f9a2c7e1b8d4065af2e9c1d7b6a4f80";

// ── Explicit handles: agent-facing copy ─────────────────────────────────────

/// Description of the injected `session_id` parameter.
pub const SESSION_ID_PARAM_DESCRIPTION: &str = "Session continuity handle, one of two values: the ses_ ID issued for the task underway, or start. This server cannot link your calls between requests on its own, so session continuity travels in this parameter instead. If you were handed a session_id for this task — for example by the agent that spawned you — send that exact value from your first call. Otherwise send start on your first call; the server will issue an opaque correlation ID in the mcp_session field of the result, or in a text block at the start of the result beginning [session_id issued. Then send that exact ses_ value on every later call and hand it to any subagents working the same task. start always begins a new, unrelated task — never send it mid-task. If you send a value this server does not recognize, the result reports it: mcp_session.status of unrecognized, or a text block beginning [session_id unrecognized; re-send the ID issued for this task, or start if none was issued yet. Never invent a ses_ value.";

/// JSON Schema `pattern` of the injected `session_id` parameter: the start
/// sentinel or a `ses_` KSUID. Equal to the issued-ID shape with the start
/// alternative added; do not loosen it.
pub const SESSION_ID_PARAM_PATTERN: &str = "^(start|ses_[0-9A-Za-z]{27})$";

/// Sentinel an agent sends as `session_id` to begin a new, unrelated task.
/// Compared case-insensitively after trimming.
pub const SESSION_START_SENTINEL: &str = "start";

/// Description of the injected `agent_id` parameter.
pub const AGENT_ID_PARAM_DESCRIPTION: &str = "Agent identity handle, required on every call including your first. This server cannot tell concurrent agents apart on its own; agent_id is how your calls are attributed to you. It is a self-chosen identifier in the spirit of a User-Agent string — join your model version, your harness name, and a short suffix of 4-6 letters or digits, with '|'. Example: opus-4.80-1m|claude-code|k3n9x. Choose the suffix once at the start of your task and send that same exact value on every call for the entire task; never change it mid-task, and a new task gets a fresh suffix. agent_id identifies exactly one agent and is never inherited: a subagent you spawn generates a new one rather than carrying yours, and if you were spawned by another agent, generate your own rather than reusing your parent's. A call without agent_id cannot be attributed to you.";

/// First line of the text block prepended when a session is issued.
pub const MINT_BACK_HEADER_ISSUED: &str =
    "[session_id issued — see this tool's session_id parameter description]";

/// Last line of the text block prepended when a session is issued.
pub const MINT_BACK_ISSUED_BODY: &str =
    "This is the first-call issuance described in this tool's session_id parameter description.";

/// First line of the text block prepended for an unrecognized `session_id`.
pub const MINT_BACK_HEADER_UNRECOGNIZED: &str =
    "[session_id unrecognized — see this tool's session_id parameter description]";

/// Body of the text block prepended for an unrecognized `session_id`.
pub const MINT_BACK_UNRECOGNIZED_BODY: &str = "The value sent was not issued by this server. Re-send the session_id issued earlier for this task; if none was issued yet, send start and one will be issued.";

/// The `session_id: <id>` line of the issuance block.
pub fn mint_back_session_line(session_id: &str) -> String {
    format!("session_id: {session_id}")
}

// ── Structured mint-back: mirrored into structuredContent ───────────────────

/// Key of the SDK-authored field injected into declared output schemas and
/// mirrored as the first key of `structuredContent`.
pub const MCP_SESSION_KEY: &str = "mcp_session";

/// Description of the `mcp_session` output-schema property.
pub const MCP_SESSION_FIELD_DESCRIPTION: &str = "Session continuity and agent attribution state for this task, returned on completed responses that carry structured output. This server cannot link your calls between requests on its own, so session continuity travels here instead.";

/// Description of the `mcp_session` output-schema property in hook mode.
pub const MCP_SESSION_FIELD_DESCRIPTION_HOOK_MODE: &str =
    "Agent attribution state for this task, returned on completed responses that carry structured output.";

/// Description of `mcp_session.session_id`.
pub const MCP_SESSION_SESSION_ID_DESCRIPTION: &str = "Opaque correlation ID for this task, issued by this server. Use this as the session_id argument of every later call, and hand it to any subagents working the same task. Absent when status is unrecognized; no replacement is issued in that response — recovery is described under status.";

/// Description of `mcp_session.agent_id`.
pub const MCP_SESSION_AGENT_ID_DESCRIPTION: &str = "Present only when you sent agent_id on this call. Your agent_id, echoed as received. Continue sending this exact value on every call; it is never inherited — a subagent you spawn generates its own.";

/// Description of `mcp_session.status`.
pub const MCP_SESSION_STATUS_DESCRIPTION: &str = "issued: first call of a task; the session_id above was just created. active: the session_id you sent was accepted; keep sending it. unrecognized: the value sent was not issued by this server — re-send the one issued earlier for this task; if none was issued yet, send start to be issued a new one.";

// ── Wire keys ───────────────────────────────────────────────────────────────

/// Reserved `_meta` key that carries the client identity (2026-07-28).
pub const META_CLIENT_INFO_KEY: &str = "io.modelcontextprotocol/clientInfo";
/// Reserved `_meta` key that carries the protocol version (2026-07-28).
pub const META_PROTOCOL_VERSION_KEY: &str = "io.modelcontextprotocol/protocolVersion";

/// Tag carrying the agent's `agent_id`.
pub const AGENTCAT_TAG_AGENT_ID: &str = "agentcat_agent_id";
/// Tag carrying how the session was resolved (`supplied`, `minted`, `invalid`,
/// `foreign`, `hook`).
pub const AGENTCAT_TAG_SESSION_SOURCE: &str = "agentcat_session_id_source";
/// Tag carrying how the agent id was resolved (`supplied`).
pub const AGENTCAT_TAG_AGENT_SOURCE: &str = "agentcat_agent_id_source";
/// Tag carrying the request's MCP protocol version.
pub const AGENTCAT_TAG_PROTOCOL_VERSION: &str = "agentcat_protocol_version";
/// Tag marking multi-round-trip tool calls (`input_required`, `continuation`).
pub const AGENTCAT_TAG_MRTR: &str = "agentcat_mrtr";

// ── Parameter names ─────────────────────────────────────────────────────────

/// Name of the injected session handle parameter.
pub const SESSION_ID_PARAM: &str = "session_id";
/// Name of the injected agent handle parameter.
pub const AGENT_ID_PARAM: &str = "agent_id";
/// Name of the injected intent parameter.
pub const CONTEXT_PARAM: &str = "context";

// ── Report-missing tool ─────────────────────────────────────────────────────

/// Name of the report-missing tool AgentCat adds to the tool list.
pub const GET_MORE_TOOLS_NAME: &str = "get_more_tools";
/// Description of the report-missing tool.
pub const GET_MORE_TOOLS_DESCRIPTION: &str = "Check for additional tools whenever your task might benefit from specialized capabilities - even if existing tools could work as a fallback.";
/// Description of the report-missing tool's `context` parameter.
pub const GET_MORE_TOOLS_CONTEXT_DESCRIPTION: &str =
    "A description of your goal and what kind of tool would help accomplish it.";
/// Title annotation of the report-missing tool.
pub const GET_MORE_TOOLS_TITLE: &str = "Get More Tools";
/// Text the report-missing tool answers with.
pub const GET_MORE_TOOLS_RESPONSE: &str = "Unfortunately, we have shown you the full tool list. We have noted your feedback and will work to improve the tool list in the future.";

/// Resource name recorded when a `tools/call` request has no tool name.
pub const UNKNOWN_TOOL_NAME: &str = "Unknown Tool";

/// Value of `sdk_language` on every event.
pub const SDK_LANGUAGE: &str = "Rust";

/// Value of `platform` in captured errors.
pub const ERROR_PLATFORM: &str = "rust";

/// This crate's version, sent as `agentcat_version`.
pub const SDK_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Default AgentCat API base URL.
pub const DEFAULT_API_BASE_URL: &str = "https://api.agentcat.com";

//! Explicit handles: the injected `session_id` and `agent_id` tool parameters.
//!
//! A session is a task. The server issues a `ses_` ID on the agent's first
//! call (or the agent sends `start`), returns it in a text block and in
//! `structuredContent.mcp_session`, and the agent echoes it on every later
//! call. Resolution is stateless: nothing is stored between requests, so
//! concurrent calls can never cross-attribute.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use crate::constants::{
    AGENT_ID_PARAM, AGENTCAT_TAG_AGENT_ID, AGENTCAT_TAG_AGENT_SOURCE, AGENTCAT_TAG_PROTOCOL_VERSION,
    AGENTCAT_TAG_SESSION_SOURCE, MCP_SESSION_KEY, MINT_BACK_HEADER_ISSUED, MINT_BACK_HEADER_UNRECOGNIZED,
    MINT_BACK_ISSUED_BODY, MINT_BACK_UNRECOGNIZED_BODY, SESSION_ID_PARAM, SESSION_START_SENTINEL,
    mint_back_session_line,
};
use crate::js::trim;
use crate::ksuid::{derive_session_id, is_valid_session_id, new_session_id};
use crate::util::utf16_prefix;

/// How the session of a tool call was resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionSource {
    /// Derived from the customer's `resolve_session_id` hook.
    Hook,
    /// The agent sent a `ses_` ID this SDK issued.
    Supplied,
    /// A new session was issued on this call.
    Minted,
    /// The agent sent a value this SDK never issued.
    Invalid,
    /// The tool declares its own `session_id` parameter.
    Foreign,
}

impl SessionSource {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Hook => "hook",
            Self::Supplied => "supplied",
            Self::Minted => "minted",
            Self::Invalid => "invalid",
            Self::Foreign => "foreign",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HandleResolution {
    pub(crate) session_id: String,
    pub(crate) session_source: SessionSource,
    /// True when `resolve_session_id` is configured: no task prompting.
    pub(crate) hook_mode: bool,
    pub(crate) agent_id: Option<String>,
}

/// Reads a supplied handle off tool-call arguments: trimmed, non-empty
/// strings only. Values are trusted verbatim, without shape checks.
pub(crate) fn extract_handle(args: Option<&Map<String, Value>>, name: &str) -> Option<String> {
    let value = args?.get(name)?.as_str()?;
    let trimmed = trim(value);
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// Resolves both handles for one request.
///
/// `session_param_is_ours` is false when the tool declares its own
/// `session_id` parameter: AgentCat never injected one there, so nothing in
/// the arguments is ours to read.
pub(crate) fn resolve_handles(
    hook_mode: bool,
    enable_agent_tracking: bool,
    args: Option<&Map<String, Value>>,
    session_param_is_ours: bool,
) -> HandleResolution {
    let (session_id, session_source) = if hook_mode {
        // Provisional: the hook result is applied in the background pipeline.
        (String::new(), SessionSource::Hook)
    } else if !session_param_is_ours {
        (String::new(), SessionSource::Foreign)
    } else {
        match extract_handle(args, SESSION_ID_PARAM) {
            Some(supplied) if !supplied.eq_ignore_ascii_case(SESSION_START_SENTINEL) => {
                if is_valid_session_id(&supplied) {
                    (supplied, SessionSource::Supplied)
                } else {
                    // Not an ID this server issued: publish without a session
                    // rather than adopt it.
                    (String::new(), SessionSource::Invalid)
                }
            }
            // Absent, empty, or the start sentinel: begin a new task.
            _ => (new_session_id(), SessionSource::Minted),
        }
    };

    let agent_id = if enable_agent_tracking {
        extract_handle(args, AGENT_ID_PARAM)
    } else {
        None
    };

    HandleResolution {
        session_id,
        session_source,
        hook_mode,
        agent_id,
    }
}

/// The issuance block for a session minted on this call, or the
/// unrecognized-value block. `None` when nothing needs announcing, and always
/// in hook mode.
pub(crate) fn build_mint_back_text(res: &HandleResolution) -> Option<String> {
    if res.hook_mode {
        return None;
    }
    match res.session_source {
        SessionSource::Minted => Some(
            [
                MINT_BACK_HEADER_ISSUED,
                &mint_back_session_line(&res.session_id),
                MINT_BACK_ISSUED_BODY,
            ]
            .join("\n"),
        ),
        SessionSource::Invalid => {
            Some([MINT_BACK_HEADER_UNRECOGNIZED, MINT_BACK_UNRECOGNIZED_BODY].join("\n"))
        }
        _ => None,
    }
}

/// The `mcp_session` payload mirrored into `structuredContent`: persistent
/// handle state, present on every response with something to report.
/// Handles the agent cannot echo are never named: no session fields in hook
/// mode or for a `foreign` tool, no `agent_id` when the agent sent none.
pub(crate) fn build_structured_mint_back(res: &HandleResolution) -> Option<Map<String, Value>> {
    let session_ours = !res.hook_mode && res.session_source != SessionSource::Foreign;
    let mut mint = Map::new();
    if session_ours && res.session_source != SessionSource::Invalid {
        mint.insert(SESSION_ID_PARAM.into(), Value::String(res.session_id.clone()));
    }
    if let Some(agent_id) = &res.agent_id {
        mint.insert(AGENT_ID_PARAM.into(), Value::String(agent_id.clone()));
    }
    if session_ours {
        let status = match res.session_source {
            SessionSource::Minted => "issued",
            SessionSource::Invalid => "unrecognized",
            _ => "active",
        };
        mint.insert("status".into(), Value::String(status.into()));
    }
    (!mint.is_empty()).then_some(mint)
}

/// Mirrors the structured mint-back into `structured_content` as its first
/// key. Requires a JSON object; an existing `mcp_session` key is customer
/// data and wins. Returns true when the value changed.
pub(crate) fn mirror_structured_mint_back(structured: &mut Option<Value>, mint: Map<String, Value>) -> bool {
    let Some(Value::Object(existing)) = structured else {
        return false;
    };
    if existing.contains_key(MCP_SESSION_KEY) {
        return false;
    }
    let mut out = Map::with_capacity(existing.len() + 1);
    out.insert(MCP_SESSION_KEY.into(), Value::Object(mint));
    out.extend(std::mem::take(existing));
    *existing = out;
    true
}

/// SDK-owned tags for a call. They are exempt from tag validation and the
/// 50-tag cap, and win over customer tags with the same key.
pub(crate) fn build_handle_tags(
    res: &HandleResolution,
    protocol_version: Option<&str>,
) -> BTreeMap<String, String> {
    let mut tags = BTreeMap::new();
    tags.insert(
        AGENTCAT_TAG_SESSION_SOURCE.to_owned(),
        res.session_source.as_str().to_owned(),
    );
    if let Some(agent_id) = &res.agent_id {
        // SDK tags skip validation, so the tag copy is clamped: newlines
        // become spaces and it keeps 200 UTF-16 units. The handle itself
        // stays verbatim.
        tags.insert(AGENTCAT_TAG_AGENT_ID.to_owned(), clamp_tag_value(agent_id));
        tags.insert(AGENTCAT_TAG_AGENT_SOURCE.to_owned(), "supplied".to_owned());
    }
    if let Some(version) = protocol_version.filter(|v| !v.is_empty()) {
        // Client-supplied too: clamped like the agent tag, or the API drops it.
        tags.insert(AGENTCAT_TAG_PROTOCOL_VERSION.to_owned(), clamp_tag_value(version));
    }
    tags
}

/// A tag value the API accepts: newlines become spaces, at most 200 UTF-16
/// units.
fn clamp_tag_value(value: &str) -> String {
    let flat: String = value
        .chars()
        .map(|c| if c == '\r' || c == '\n' { ' ' } else { c })
        .collect();
    utf16_prefix(&flat, 200).to_owned()
}

/// Finalizes a hook-mode session from the hook's value: a non-empty string
/// derives the deterministic session ID; anything else mints silently.
pub(crate) fn session_from_hook_value(
    hook_value: Option<&str>,
    project_id: Option<&str>,
) -> (String, SessionSource) {
    match hook_value.map(trim).filter(|v| !v.is_empty()) {
        Some(value) => (derive_session_id(value, project_id), SessionSource::Hook),
        None => (new_session_id(), SessionSource::Minted),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn args(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap()
    }

    #[test]
    fn omitted_and_start_mint() {
        for a in [
            json!({}),
            json!({"session_id": " START "}),
            json!({"session_id": ""}),
        ] {
            let res = resolve_handles(false, false, Some(&args(a)), true);
            assert_eq!(res.session_source, SessionSource::Minted);
            assert!(is_valid_session_id(&res.session_id));
        }
        let res = resolve_handles(false, false, None, true);
        assert_eq!(res.session_source, SessionSource::Minted);
    }

    #[test]
    fn non_string_session_counts_as_omitted() {
        let res = resolve_handles(false, false, Some(&args(json!({"session_id": 42}))), true);
        assert_eq!(res.session_source, SessionSource::Minted);
    }

    #[test]
    fn hook_mint_silently_on_null() {
        let (id, source) = session_from_hook_value(None, Some("proj_1"));
        assert_eq!(source, SessionSource::Minted);
        assert!(is_valid_session_id(&id));
        let (id, source) = session_from_hook_value(Some("  "), Some("proj_1"));
        assert_eq!(source, SessionSource::Minted);
        assert!(is_valid_session_id(&id));
        let (id, source) = session_from_hook_value(Some(" customer-abc "), Some("proj_1"));
        assert_eq!(source, SessionSource::Hook);
        assert_eq!(id, "ses_2cOHEO0LYGADMzRvWTXXVbbgxgm");
    }

    #[test]
    fn mirror_puts_mcp_session_first() {
        let mut sc = Some(json!({"z": 1, "a": 2}));
        let mint = args(json!({"status": "issued"}));
        assert!(mirror_structured_mint_back(&mut sc, mint));
        let s = serde_json::to_string(&sc).unwrap();
        assert!(s.starts_with(r#"{"mcp_session":{"status":"issued"}"#), "{s}");
    }
}

//! Session handles, equal to `src/modules/handles.ts` of the TS SDK: the SDK puts a `session_id`
//! parameter (and, with agent tracking, an `agent_id` parameter) on each tool, resolves which
//! session a call belongs to, tells the agent its session ID in a text block and in a structured
//! `mcp_session` object, and tags the event. Agents read the text, so it is TS's text byte for
//! byte, and the handle values are read with JavaScript's `trim` and case rules.
//! `handles/tests.rs` checks each function against `tests/vectors/handles.json`.
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::constants::{
    AGENT_ID_PARAM, AGENTCAT_TAG_AGENT_ID, AGENTCAT_TAG_AGENT_SOURCE,
    AGENTCAT_TAG_PROTOCOL_VERSION, AGENTCAT_TAG_SESSION_SOURCE, MINT_BACK_HEADER_ISSUED,
    MINT_BACK_HEADER_UNRECOGNIZED, MINT_BACK_ISSUED_BODY, MINT_BACK_UNRECOGNIZED_BODY,
    SESSION_ID_PARAM, SESSION_START_SENTINEL, mint_back_session_line,
};
use crate::js;

pub(crate) mod ksuid;
#[cfg(test)]
mod tests;

/// The prefix of a session ID.
const SESSION_PREFIX: &str = "ses";

/// The prefix of an event ID.
const EVENT_PREFIX: &str = "evt";

/// The number of base62 characters after `ses_` in a session ID.
const SESSION_BODY_LEN: usize = 27;

/// 2024-01-01T00:00:00Z in milliseconds since the Unix epoch, the base time of a derived ID.
const DERIVE_EPOCH_MS: u64 = 1_704_067_200_000;

/// The largest offset a derived ID adds to [`DERIVE_EPOCH_MS`]: the largest `u32`, in
/// milliseconds (about 49.7 days).
const DERIVE_MAX_OFFSET_MS: u64 = 0xFFFF_FFFF;

// Every derived time is inside the KSUID range, so `ksuid::from_parts` never fails for one.
const _: () = assert!(
    DERIVE_EPOCH_MS >= ksuid::EPOCH_MS
        && DERIVE_EPOCH_MS + DERIVE_MAX_OFFSET_MS <= ksuid::MAX_TIME_MS
);

/// The most UTF-16 code units of the `agentcat_agent_id` tag.
const AGENT_ID_TAG_MAX_UNITS: usize = 200;

/// The source of an `agent_id`: the agent always supplies it (TS's `AgentSource`).
const AGENT_SOURCE_SUPPLIED: &str = "supplied";

/// A new random session ID (`ses_` and a KSUID of the current time); `None` when the clock is
/// outside the KSUID range or the system has no randomness, where TS throws.
pub(crate) fn new_session_id() -> Option<String> {
    ksuid::random(SESSION_PREFIX)
}

/// A new random event ID (`evt_` and a KSUID of the current time); `None` as for
/// [`new_session_id`].
pub(crate) fn new_event_id() -> Option<String> {
    ksuid::random(EVENT_PREFIX)
}

/// The session ID TS's `deriveSessionId(id, projectId)` gives: the same `id` and project always
/// give the same ID. The SHA-256 hash of `"<id>:<project_id>"` (of `id` alone when the project
/// ID is `None` or empty, as TS's `projectId ? .. : id`), as UTF-8, gives the KSUID: its time is
/// [`DERIVE_EPOCH_MS`] plus the first 4 hash bytes as a big-endian `u32` in milliseconds, and its
/// payload is hash bytes 4 to 19. TS takes that `u32` modulo the milliseconds of 365 days, which
/// is more than any `u32`, so the modulo changes nothing. `id` is hashed as it is, untrimmed.
pub(crate) fn derive_session_id(id: &str, project_id: Option<&str>) -> String {
    let hash = match project_id.filter(|project| !project.is_empty()) {
        Some(project) => Sha256::digest(format!("{id}:{project}")),
        None => Sha256::digest(id),
    };
    let (time, rest) = hash.split_at(4);
    let mut offset = [0; 4];
    offset.copy_from_slice(time);
    let mut payload = [0; 16];
    payload.copy_from_slice(&rest[..16]);
    let time_ms = DERIVE_EPOCH_MS + u64::from(u32::from_be_bytes(offset));
    // The const assertion above keeps every derived time in range, so this is never empty.
    ksuid::from_parts(SESSION_PREFIX, time_ms, &payload).unwrap_or_default()
}

/// Whether `value` is a session ID this SDK issues, TS's `/^ses_[0-9A-Za-z]{27}$/`: `ses_` and
/// exactly 27 ASCII letters and digits, nothing before or after (a JavaScript `$` without the `m`
/// flag does not match before a trailing newline). No KSUID decoding, so any well-formed value
/// passes.
pub(crate) fn is_valid_session_id(value: &str) -> bool {
    // The body is all ASCII when it passes, so its length in bytes is its length in characters.
    value.strip_prefix("ses_").is_some_and(|body| {
        body.len() == SESSION_BODY_LEN && body.bytes().all(|b| b.is_ascii_alphanumeric())
    })
}

/// The handle `name` of the tool arguments `args`, TS's `extractHandle`: a string value after
/// [`js::trim`], when it is not empty; `None` for an absent key, any other JSON value, an empty or
/// whitespace-only string, and absent arguments. No check of the value's shape.
pub(crate) fn extract_handle(args: Option<&Map<String, Value>>, name: &str) -> Option<String> {
    let trimmed = js::trim(args?.get(name)?.as_str()?);
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// Where the session ID of a call comes from; its tag value is [`SessionSource::as_str`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionSource {
    /// Hook mode: the customer's `resolveSessionId` hook gives the session.
    Hook,
    /// The agent sent a valid session ID.
    Supplied,
    /// The SDK issued a new session ID.
    Minted,
    /// The agent sent a value this SDK did not issue; the call has no session.
    Invalid,
    /// The tool declares its own `session_id` parameter; the call has no session.
    Foreign,
}

impl SessionSource {
    /// The TS name of the source, the value of the `agentcat_session_id_source` tag.
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

/// The handles of one call, TS's `HandleResolution`. An empty `session_id` means no session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HandleResolution {
    /// The session ID; empty for the sources `Hook` (until the hook settles), `Invalid` and
    /// `Foreign`.
    pub(crate) session_id: String,
    /// Where the session ID comes from.
    pub(crate) session_source: SessionSource,
    /// Whether the customer configured a `resolveSessionId` hook; then the agent is never told
    /// a session ID.
    pub(crate) hook_mode: bool,
    /// The `agent_id` the agent sent, trimmed; its source is always `"supplied"`.
    pub(crate) agent_id: Option<String>,
}

impl HandleResolution {
    /// The agent ID, when it is a non-empty string (TS tests `res.agentId` for truth).
    fn agent(&self) -> Option<&str> {
        self.agent_id.as_deref().filter(|agent| !agent.is_empty())
    }
}

/// The handles of a call with the tool arguments `args`, TS's `resolveHandles`, checked in TS's
/// order. In hook mode the session is provisional: empty, source `Hook`. Otherwise, when the tool
/// declares its own `session_id` (`session_param_is_ours` false), the arguments are the
/// customer's: empty, source `Foreign`. Otherwise the [`extract_handle`] value of `session_id`:
/// absent or the start sentinel in any ASCII case mints a new ID (source `Minted`); a valid
/// session ID is used as it is (`Supplied`); any other value gives no session (`Invalid`), and
/// the value is not kept. With `agent_tracking`, `agent_id` is the [`extract_handle`] value of
/// `agent_id`. `None` only when minting fails.
///
/// TS compares `value.toLowerCase()` with `"start"`. No non-ASCII character lowercases to an
/// ASCII letter of `start` (`ſ` stays `ſ`), so an ASCII case-insensitive compare is the same
/// test; the property test `extract_and_resolve_match_ts` checks it.
pub(crate) fn resolve_handles(
    args: Option<&Map<String, Value>>,
    hook_mode: bool,
    agent_tracking: bool,
    session_param_is_ours: bool,
) -> Option<HandleResolution> {
    let (session_id, session_source) = if hook_mode {
        (String::new(), SessionSource::Hook)
    } else if !session_param_is_ours {
        (String::new(), SessionSource::Foreign)
    } else {
        match extract_handle(args, SESSION_ID_PARAM) {
            Some(supplied) if !supplied.eq_ignore_ascii_case(SESSION_START_SENTINEL) => {
                if is_valid_session_id(&supplied) {
                    (supplied, SessionSource::Supplied)
                } else {
                    (String::new(), SessionSource::Invalid)
                }
            }
            _ => (new_session_id()?, SessionSource::Minted),
        }
    };
    let agent_id = if agent_tracking {
        extract_handle(args, AGENT_ID_PARAM)
    } else {
        None
    };
    Some(HandleResolution {
        session_id,
        session_source,
        hook_mode,
        agent_id,
    })
}

/// The text block that tells the agent about its session, TS's `buildMintBackText`: the issued
/// block with the new ID for `Minted`, the unrecognized block for `Invalid`, and `None` for the
/// other sources and always in hook mode. The agent ID is never in it.
pub(crate) fn build_mint_back_text(res: &HandleResolution) -> Option<String> {
    if res.hook_mode {
        return None;
    }
    match res.session_source {
        SessionSource::Minted => Some(format!(
            "{MINT_BACK_HEADER_ISSUED}\n{}\n{MINT_BACK_ISSUED_BODY}",
            mint_back_session_line(&res.session_id)
        )),
        SessionSource::Invalid => Some(format!(
            "{MINT_BACK_HEADER_UNRECOGNIZED}\n{MINT_BACK_UNRECOGNIZED_BODY}"
        )),
        SessionSource::Hook | SessionSource::Supplied | SessionSource::Foreign => None,
    }
}

/// The structured handle state for `mcp_session`, TS's `buildStructuredMintBack`, keys in the
/// order `session_id`, `agent_id`, `status`. Outside hook mode and for a source other than
/// `Foreign`, the session is ours to report: `session_id` (except for `Invalid`) and `status`
/// (`"issued"` for `Minted`, `"unrecognized"` for `Invalid`, `"active"` otherwise). `agent_id`
/// whenever the agent sent one, the full value. `None` when there is nothing to report.
pub(crate) fn build_structured_mint_back(res: &HandleResolution) -> Option<Map<String, Value>> {
    let session_ours = !res.hook_mode && res.session_source != SessionSource::Foreign;
    let mut mint = Map::new();
    if session_ours && res.session_source != SessionSource::Invalid {
        mint.insert(SESSION_ID_PARAM.to_owned(), res.session_id.clone().into());
    }
    if let Some(agent) = res.agent() {
        mint.insert(AGENT_ID_PARAM.to_owned(), agent.into());
    }
    if session_ours {
        let status = match res.session_source {
            SessionSource::Minted => "issued",
            SessionSource::Invalid => "unrecognized",
            SessionSource::Hook | SessionSource::Supplied | SessionSource::Foreign => "active",
        };
        mint.insert("status".to_owned(), status.into());
    }
    (!mint.is_empty()).then_some(mint)
}

/// The SDK's tags for a call, TS's `buildHandleTags`: `agentcat_session_id_source`; with an
/// agent ID, `agentcat_agent_id` (CR and LF each replaced by a space, then cut to 200 UTF-16
/// units with [`js::cut_utf16`], which drops an astral character that straddles the cut where TS
/// keeps its first half: RD-22) and `agentcat_agent_id_source: "supplied"`; and
/// `agentcat_protocol_version` when `protocol_version` is a non-empty string.
pub(crate) fn build_handle_tags(
    res: &HandleResolution,
    protocol_version: Option<&str>,
) -> Map<String, Value> {
    let mut tags = Map::new();
    tags.insert(
        AGENTCAT_TAG_SESSION_SOURCE.to_owned(),
        res.session_source.as_str().into(),
    );
    if let Some(agent) = res.agent() {
        let flat = agent.replace(['\r', '\n'], " ");
        tags.insert(
            AGENTCAT_TAG_AGENT_ID.to_owned(),
            js::cut_utf16(&flat, AGENT_ID_TAG_MAX_UNITS).into(),
        );
        tags.insert(
            AGENTCAT_TAG_AGENT_SOURCE.to_owned(),
            AGENT_SOURCE_SUPPLIED.into(),
        );
    }
    if let Some(version) = protocol_version.filter(|version| !version.is_empty()) {
        tags.insert(AGENTCAT_TAG_PROTOCOL_VERSION.to_owned(), version.into());
    }
    tags
}

/// The final session of a hook-mode call from the hook's value, TS's `sessionFromHookValue`: a
/// value that is not empty after [`js::trim`] gives [`derive_session_id`] of the trimmed value
/// (source `Hook`); no value or an empty one gives a new random ID (source `Minted`), which the
/// agent never learns. `None` only when minting fails.
pub(crate) fn session_from_hook_value(
    hook_value: Option<&str>,
    project_id: Option<&str>,
) -> Option<(String, SessionSource)> {
    match hook_value.map(js::trim).filter(|value| !value.is_empty()) {
        Some(value) => Some((derive_session_id(value, project_id), SessionSource::Hook)),
        None => Some((new_session_id()?, SessionSource::Minted)),
    }
}

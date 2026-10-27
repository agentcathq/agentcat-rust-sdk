//! Stage 0 of TS's queue (`src/modules/pendingEvent.ts`): [`settle`] awaits the customer hooks a
//! tool call started (B-022 step 7, B-026), each within its budget (C37), and [`apply`] puts what
//! they gave into the event, as TS's `applyPendingEventFields` does (B-084, B-093, B-095, B-096).
use std::fmt;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use futures::future::join4;
use serde_json::{Map, Value};
use tokio::task::JoinError;

use super::tags::validate_tags;
use super::{TsEvent, set_field};
use crate::capture::panic_message;
use crate::config::{HookError, UserIdentity};
use crate::constants::AGENTCAT_TAG_SESSION_SOURCE;
use crate::handles::session_from_hook_value;
use crate::hooks::{AbortOnDrop, PendingHooks};

#[cfg(test)]
mod tests;

/// How long one call hook may take to settle once its event is processed (TS's
/// `PENDING_HOOK_TIMEOUT_MS`). The log line of a hook that takes longer always names this budget.
pub(crate) const HOOK_BUDGET: Duration = Duration::from_secs(30);

/// One of the four call hooks: its TS label, and the text TS writes before the hook's own error
/// (the TS resolver's `catch`).
struct Label {
    /// The label of the hook in TS's log lines.
    name: &'static str,
    /// The text before the error's `Display` when the hook returns an error.
    error_prefix: &'static str,
}

/// The `resolve_session_id` hook (`invokeSessionHook`).
const RESOLVE_SESSION_ID: Label = Label {
    name: "resolveSessionId",
    error_prefix: "resolveSessionId hook error: ",
};

/// The `identify` hook (`resolveIdentity`).
const IDENTIFY: Label = Label {
    name: "identify",
    error_prefix: "Error: identify callback threw - ",
};

/// The `event_tags` hook (`resolveEventTags`).
const EVENT_TAGS: Label = Label {
    name: "eventTags",
    error_prefix: "eventTags callback error: ",
};

/// The `event_properties` hook (`resolveEventProperties`).
const EVENT_PROPERTIES: Label = Label {
    name: "eventProperties",
    error_prefix: "eventProperties callback error: ",
};

/// What the hooks of one call gave once they settled; `None` for a hook that is not set, gave
/// nothing, failed or ran out of time.
#[derive(Debug, Default)]
pub(crate) struct HookResults {
    /// The value of `resolve_session_id`.
    pub(crate) session_value: Option<String>,
    /// The identity `identify` gave.
    pub(crate) identity: Option<UserIdentity>,
    /// The tags of `event_tags` that passed [`validate_tags`]; `None` when none did.
    pub(crate) tags: Option<Map<String, Value>>,
    /// The properties `event_properties` gave, an empty object included (B-095).
    pub(crate) properties: Option<Map<String, Value>>,
}

/// [`apply`] could not give a hook-mode event its session: minting a session ID failed, where TS
/// throws.
#[derive(Debug)]
pub(crate) struct SessionMintFailed;

impl fmt::Display for SessionMintFailed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("could not generate a session ID")
    }
}

/// Awaits the hook tasks of `hooks` together, each for at most `budget` (C37, RD-35), and pushes
/// TS's log lines to `log`:
///
/// - A hook that has not settled within `budget` is aborted, after `Warning: <label> hook did not
///   settle within 30000ms; publishing event without it`; the text names [`HOOK_BUDGET`] whatever
///   `budget` is.
/// - A hook whose task panicked or was cancelled (its runtime shut down) gives `Warning: <label>
///   hook failed in background - <message>`, the panic's message or the cancellation. The process
///   panic hook has already printed the panic, as for any task.
/// - A hook's own error gives TS's text for that hook followed by the error's `Display`:
///   `resolveSessionId hook error: `, `Error: identify callback threw - `, `eventTags callback
///   error: ` or `eventProperties callback error: `.
/// - The `event_tags` result goes through [`validate_tags`] when it settles, with TS's line for
///   each tag it drops.
///
/// The lines are in the order `settle` sees the hooks settle. Hooks that settled before `settle`
/// first polls them, or between two of its polls, log in TS's label order (`resolveSessionId`,
/// `identify`, `eventTags`, `eventProperties`), whatever order they finished in. TS logs each
/// failure when it happens, even for an event its queue never takes. Runs on a runtime with the
/// time driver; the hook tasks may run on another runtime. Dropping the future aborts the hook
/// tasks it has not settled.
pub(crate) async fn settle(
    mut hooks: PendingHooks,
    budget: Duration,
    log: &mut Vec<String>,
) -> HookResults {
    // Each task leaves `hooks` in a guard that aborts it when this future is dropped (a job the
    // queue abandons) or when its budget runs out.
    let session_task = hooks.resolve_session_id.take().map(AbortOnDrop::new);
    let identify_task = hooks.identify.take().map(AbortOnDrop::new);
    let tags_task = hooks.event_tags.take().map(AbortOnDrop::new);
    let properties_task = hooks.event_properties.take().map(AbortOnDrop::new);
    let lines = Mutex::new(Vec::new());
    let tags = async {
        let raw = settle_one(tags_task, &EVENT_TAGS, budget, &lines).await?;
        let mut dropped = Vec::new();
        let valid = validate_tags(&raw, &mut dropped);
        push(&lines, dropped);
        valid
    };
    // `join4` polls the four in this order every time, so hooks that settle together log in it.
    let (session_value, identity, tags, properties) = join4(
        settle_one(session_task, &RESOLVE_SESSION_ID, budget, &lines),
        settle_one(identify_task, &IDENTIFY, budget, &lines),
        tags,
        settle_one(properties_task, &EVENT_PROPERTIES, budget, &lines),
    )
    .await;
    log.extend(lines.into_inner().unwrap_or_else(PoisonError::into_inner));
    HookResults {
        session_value,
        identity,
        tags,
        properties,
    }
}

/// The value of one hook `task` (`None` when the hook is not set), awaited for at most `budget`;
/// the log line of a failure or a timeout goes to `lines` (see [`settle`]).
async fn settle_one<T>(
    task: Option<AbortOnDrop<Result<Option<T>, HookError>>>,
    hook: &Label,
    budget: Duration,
    lines: &Mutex<Vec<String>>,
) -> Option<T> {
    let task = task?;
    let (value, line) = match tokio::time::timeout(budget, task).await {
        Ok(Ok(Ok(value))) => (value, None),
        Ok(Ok(Err(error))) => (None, Some(format!("{}{error}", hook.error_prefix))),
        Ok(Err(error)) => {
            let message = failure_message(error);
            let line = format!(
                "Warning: {} hook failed in background - {message}",
                hook.name
            );
            (None, Some(line))
        }
        Err(_elapsed) => {
            // `timeout` dropped the task's guard, which aborted it.
            let line = format!(
                "Warning: {} hook did not settle within {}ms; publishing event without it",
                hook.name,
                HOOK_BUDGET.as_millis()
            );
            (None, Some(line))
        }
    };
    push(lines, line);
    value
}

/// The message of a hook task that did not finish: the panic's message, or tokio's text for a
/// cancelled task.
pub(super) fn failure_message(error: JoinError) -> String {
    match error.try_into_panic() {
        Ok(payload) => panic_message(&*payload),
        Err(error) => error.to_string(),
    }
}

/// Appends `new` to `lines`.
fn push(lines: &Mutex<Vec<String>>, new: impl IntoIterator<Item = String>) {
    lines
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .extend(new);
}

/// Puts `results` into `event`, in place, as TS's `applyPendingEventFields` does:
///
/// 1. In hook mode, the session from the hook's value (`session_from_hook_value`, salted with the
///    event's `projectId` when it is not empty) becomes `sessionId`, and its source the
///    `agentcat_session_id_source` tag. When minting fails, nothing more is applied and the
///    error comes back, as TS's throw at that point leaves the later fields unset.
/// 2. An identity sets `identifyActorGivenId`, `identifyActorName` (absent without a name) and
///    `identifyActorData` (`{}` without data) (B-084).
/// 3. The customer tags merge under the SDK tags, `{...customer, ...sdk}`: an SDK tag wins a name
///    collision at the customer tag's place (B-096).
/// 4. The properties replace `properties`, an empty object included (B-095).
///
/// A key the event lacks goes to its place in TS's key order ([`set_field`]).
pub(crate) fn apply(
    event: &mut TsEvent,
    results: HookResults,
    hook_mode: bool,
) -> Result<(), SessionMintFailed> {
    let HookResults {
        session_value,
        identity,
        tags,
        properties,
    } = results;
    if hook_mode {
        let project = event
            .get("projectId")
            .and_then(Value::as_str)
            .filter(|project| !project.is_empty());
        let (session_id, source) =
            session_from_hook_value(session_value.as_deref(), project).ok_or(SessionMintFailed)?;
        set_field(event, "sessionId", Value::String(session_id));
        let mut sdk_tags = event_tags(event);
        sdk_tags.insert(
            AGENTCAT_TAG_SESSION_SOURCE.to_owned(),
            Value::String(source.as_str().to_owned()),
        );
        set_field(event, "tags", Value::Object(sdk_tags));
    }
    if let Some(identity) = identity {
        set_field(
            event,
            "identifyActorGivenId",
            Value::String(identity.user_id),
        );
        match identity.user_name {
            Some(name) => set_field(event, "identifyActorName", Value::String(name)),
            None => {
                event.shift_remove("identifyActorName");
            }
        }
        let data = identity.user_data.unwrap_or_default();
        set_field(event, "identifyActorData", Value::Object(data));
    }
    if let Some(mut merged) = tags {
        merged.extend(event_tags(event));
        set_field(event, "tags", Value::Object(merged));
    }
    if let Some(properties) = properties {
        set_field(event, "properties", Value::Object(properties));
    }
    Ok(())
}

/// A copy of the event's `tags`, or no tags when it has none (`event.tags ?? {}`).
fn event_tags(event: &TsEvent) -> Map<String, Value> {
    match event.get("tags") {
        Some(Value::Object(tags)) => tags.clone(),
        _ => Map::new(),
    }
}

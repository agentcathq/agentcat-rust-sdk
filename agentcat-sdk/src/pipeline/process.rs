//! The per-event pipeline of the delivery queue: [`process`] takes one [`EventJob`] through every
//! stage, in a fixed order, and gives the event to publish, or nothing when a stage drops it.
//! Each stage's failure is a log line with the stage's text.
//!
//! The customer's redaction hooks run on the job's runtime, the one the call (or the caller of
//! `publish_custom_event`) ran on, not on the delivery worker's. Without that runtime, or when the
//! hook's task panics or is cancelled, the event is dropped with the stage's line. The hook's task
//! is aborted when the pipeline is dropped before it ends.
use std::time::Duration;

use serde_json::Value;
use tokio::runtime::Handle;

use super::EventObject;
use super::redact::{apply_event_redaction, redact_strings, visit_strings};
use super::sanitize::sanitize_event;
use super::settle::{apply, failure_message, settle};
use super::truncate::truncate_event;
use crate::config::{EventRedactor, HookError, StringRedactor};
use crate::event::Event;
use crate::event::assemble::{EventJob, render_backtrace};
use crate::handles::new_event_id;
use crate::hooks::AbortOnDrop;

#[cfg(test)]
mod tests;

/// The text of a redaction stage's line when the job has no runtime for the customer's hook.
const NO_RUNTIME: &str = "no tokio runtime";

/// The customer's redaction hooks, each `None` when it is not set.
#[derive(Clone, Debug, Default)]
pub(crate) struct Redactors {
    /// The hook of `redact_sensitive_information`, which gets one string at a time.
    pub(crate) string: Option<StringRedactor>,
    /// The hook of `redact_event`, which gets the whole event.
    pub(crate) event: Option<EventRedactor>,
}

/// The event `job` publishes, after the stages below in their order, or `None` when a stage drops
/// it. Each stage's log lines go to `log`.
///
/// 1. The backtrace of a captured error becomes `error.stack` ([`render_backtrace`]).
/// 2. The call hooks settle, each within `hook_budget` ([`settle`]), and their results go into the
///    event ([`apply`]). When `apply` cannot mint a hook-mode session, `Failed to resolve pending
///    hook results: could not generate a session ID`, and the event goes on.
/// 3. With `job.event_redaction` and an event hook: the hook gets the event as an [`Event`] on the
///    job's runtime, and the system fields come back from the original
///    ([`apply_event_redaction`]). The hook's `None` drops the event with `Event dropped by
///    redact_event hook`; its error, its task's panic or no runtime drops it with `Failed to redact
///    event (event-level hook): <why>`.
/// 4. With `job.string_redaction` and a string hook: the hook gets each string of the walk of
///    [`redact_strings`] in walk order, all in one task on the job's runtime, and its results
///    replace the strings; the first error, the task's panic or no runtime drops the event with
///    `Failed to redact event: <why>`.
/// 5. [`sanitize_event`], then [`truncate_event`], each merged into the event with [`assign`]; a
///    truncation error drops it with `Failed to truncate event: <error>` (sanitization cannot
///    fail, so it has no line).
/// 6. An event without an `id` (or with `""`) gets a new `evt_` ID; when none can be made, the
///    event is dropped with `Failed to deliver AgentCat event after retries: could not generate an
///    event ID`, the line of a failed delivery.
///
/// Runs on a runtime with the time driver (the hook budget). Dropping the future aborts the
/// hook tasks of the job and a redaction task that runs.
pub(crate) async fn process(
    job: EventJob,
    redactors: &Redactors,
    hook_budget: Duration,
    log: &mut Vec<String>,
) -> Option<EventObject> {
    process_with_ids(job, redactors, hook_budget, new_event_id, log).await
}

/// [`process`] with `new_id` as the source of the event ID of stage 6, so a test can make it fail.
async fn process_with_ids(
    job: EventJob,
    redactors: &Redactors,
    hook_budget: Duration,
    new_id: fn() -> Option<String>,
    log: &mut Vec<String>,
) -> Option<EventObject> {
    let EventJob {
        mut event,
        hooks,
        hook_mode,
        runtime,
        string_redaction,
        event_redaction,
        backtrace,
    } = job;
    if let Some(backtrace) = &backtrace {
        render_backtrace(&mut event, backtrace);
    }

    let results = settle(hooks, hook_budget, log).await;
    if let Err(error) = apply(&mut event, results, hook_mode) {
        log.push(format!("Failed to resolve pending hook results: {error}"));
    }

    if event_redaction && let Some(hook) = &redactors.event {
        match redact_event(hook, &event, runtime.as_ref()).await {
            Ok(result) => {
                let Some(redacted) = apply_event_redaction(&event, result.map(Event::into_object))
                else {
                    log.push("Event dropped by redact_event hook".to_owned());
                    return None;
                };
                event = redacted;
            }
            Err(why) => {
                log.push(format!("Failed to redact event (event-level hook): {why}"));
                return None;
            }
        }
    }

    if string_redaction && let Some(hook) = &redactors.string {
        match redact_event_strings(hook, &event, runtime.as_ref()).await {
            Ok(redacted) => assign(&mut event, redacted),
            Err(why) => {
                log.push(format!("Failed to redact event: {why}"));
                return None;
            }
        }
    }

    let sanitized = sanitize_event(&event);
    assign(&mut event, sanitized);
    event = match truncate_event(event) {
        Ok(truncated) => truncated,
        Err(error) => {
            log.push(format!("Failed to truncate event: {error}"));
            return None;
        }
    };

    let has_id = event
        .get("id")
        .and_then(Value::as_str)
        .is_some_and(|id| !id.is_empty());
    if !has_id {
        let Some(id) = new_id() else {
            log.push(
                "Failed to deliver AgentCat event after retries: could not generate an event ID"
                    .to_owned(),
            );
            return None;
        };
        event.insert("id".to_owned(), Value::String(id));
    }
    Some(event)
}

/// Merges a stage's result into the event: each key of `stage` sets its value, in place when
/// `event` has the key and last when it does not; a key `stage` lacks stays.
pub(crate) fn assign(event: &mut EventObject, stage: EventObject) {
    event.extend(stage);
}

/// What the event-level `hook` gives for `event`, run on `runtime`: `Ok(None)` drops the
/// event; `Err` holds why it could not run (no runtime, its error, its task's panic or
/// cancellation).
async fn redact_event(
    hook: &EventRedactor,
    event: &EventObject,
    runtime: Option<&Handle>,
) -> Result<Option<Event>, String> {
    let runtime = runtime.ok_or_else(|| NO_RUNTIME.to_owned())?;
    match AbortOnDrop::new(runtime.spawn(hook.call(Event::from_object(event)))).await {
        Ok(result) => result.map_err(|error| error.to_string()),
        Err(error) => Err(failure_message(error)),
    }
}

/// `event` with each string the walk of [`redact_strings`] visits replaced by what the string
/// `hook` gives for it. The walk's own rule decides which strings and in what order:
/// [`visit_strings`] collects them from the borrowed event, one task on `runtime` awaits the hook
/// on each in that order and stops at its first error, and [`redact_strings`] puts the results in.
/// `Err` holds why it could not run (no runtime, the hook's error, the task's panic or
/// cancellation).
async fn redact_event_strings(
    hook: &StringRedactor,
    event: &EventObject,
    runtime: Option<&Handle>,
) -> Result<EventObject, String> {
    let runtime = runtime.ok_or_else(|| NO_RUNTIME.to_owned())?;
    let mut strings = Vec::new();
    visit_strings(event, |text| strings.push(text.to_owned()));
    let hook = hook.clone();
    let task = AbortOnDrop::new(runtime.spawn(async move {
        let mut redacted = Vec::with_capacity(strings.len());
        for text in strings {
            redacted.push(hook.call(text).await?);
        }
        Ok::<_, HookError>(redacted)
    }));
    let redacted = match task.await {
        Ok(result) => result.map_err(|error| error.to_string())?,
        Err(error) => return Err(failure_message(error)),
    };
    let mut redacted = redacted.into_iter();
    redact_strings(event, |_| {
        redacted
            .next()
            .ok_or_else(|| "the event changed during redaction".to_owned())
    })
}

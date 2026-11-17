//! The send of a server info heartbeat, on the delivery worker: the body is built from the
//! [`Snapshot`], the customer's string-redaction hook runs on its text, and one POST goes out
//! through the [`Publisher`]. Nothing is sent a second time, except the smaller body after a
//! `413`. [`deliver`] gives the one log line of the attempt.
use std::time::{Duration, Instant};

use agentcat_api::models::PublishServerInfoRequest;
use tokio::runtime::Handle;

use super::body::{Snapshot, body, tool_count, without_schemas};
use super::{not_sent_line, sent_line};
use crate::config::{HookError, StringRedactor};
use crate::delivery::{Publisher, ServerInfoAnswer};
use crate::hooks::AbortOnDrop;
use crate::pipeline::redact::server_info_strings;

#[cfg(test)]
mod tests;

/// How long the read of the lists and the redaction may take together.
pub(crate) const BUILD_TIMEOUT: Duration = Duration::from_secs(30);

/// What a heartbeat that went out held.
struct Sent {
    /// How many tools.
    tools: usize,
    /// The size of the body, as JSON text.
    bytes: usize,
    /// The lists that failed and were left out.
    left_out: Vec<&'static str>,
}

/// Sends the heartbeat of `snapshot` for `project_id` through `publisher`, after `redactor` ran
/// on its text, and gives the log line of the attempt: the line of a sent heartbeat with its
/// counts, or the line of one that was not sent with the reason. The lines of the publisher's
/// [`prepare`](Publisher::prepare) go to `log`.
///
/// Runs on a runtime with the time driver. Dropping the future aborts a redaction task that runs.
pub(crate) async fn deliver(
    project_id: &str,
    redactor: Option<&StringRedactor>,
    publisher: &dyn Publisher,
    snapshot: Snapshot,
    log: &mut Vec<String>,
) -> String {
    match attempt(project_id, redactor, publisher, snapshot, log).await {
        Ok(sent) => sent_line(sent.tools, sent.bytes, &sent.left_out),
        Err(why) => not_sent_line(&why),
    }
}

/// The attempt of [`deliver`]; `Err` holds why nothing was sent.
async fn attempt(
    project_id: &str,
    redactor: Option<&StringRedactor>,
    publisher: &dyn Publisher,
    snapshot: Snapshot,
    log: &mut Vec<String>,
) -> Result<Sent, String> {
    let Snapshot {
        server,
        instructions,
        mut lists,
        inject,
        mut state,
        runtime,
        started,
    } = snapshot;
    let left_out = std::mem::take(&mut lists.left_out);
    let mut body = body(project_id, server, instructions, lists, &inject, &mut state)?;
    if let Some(redactor) = redactor {
        let deadline = started.checked_add(BUILD_TIMEOUT).unwrap_or(started);
        redact(redactor, &mut body, runtime.as_ref(), deadline)
            .await
            .map_err(|why| format!("the redaction did not work: {why}"))?;
    }
    publisher.prepare(log)?;
    let mut answer = publisher.publish_server_info(body.clone()).await?;
    if answer == ServerInfoAnswer::TooLarge {
        without_schemas(&mut body);
        answer = publisher.publish_server_info(body.clone()).await?;
    }
    match answer {
        ServerInfoAnswer::Accepted => Ok(Sent {
            tools: tool_count(&body),
            bytes: serde_json::to_vec(&body).map_or(0, |text| text.len()),
            left_out,
        }),
        ServerInfoAnswer::TooLarge => {
            Err("the body is too large for the API, also without the tool schemas".to_owned())
        }
    }
}

/// Replaces each string of `body` that the walk of [`server_info_strings`] visits by what `hook`
/// gives for it. One task on `runtime` awaits the hook on each string in walk order and stops at
/// its first error. `Err` holds why the body must not be sent: no runtime, the hook's error, the
/// task's panic or cancellation, or `deadline` passed first (the task is then aborted). It is a
/// fixed text, never the message of the hook's error or panic.
async fn redact(
    hook: &StringRedactor,
    body: &mut PublishServerInfoRequest,
    runtime: Option<&Handle>,
    deadline: Instant,
) -> Result<(), String> {
    let runtime = runtime.ok_or_else(|| "no tokio runtime".to_owned())?;
    let mut strings = Vec::new();
    server_info_strings(body, &mut |text| strings.push(text.clone()));
    let hook = hook.clone();
    let task = AbortOnDrop::new(runtime.spawn(async move {
        let mut redacted = Vec::with_capacity(strings.len());
        for text in strings {
            redacted.push(hook.call(text).await?);
        }
        Ok::<_, HookError>(redacted)
    }));
    let redacted = match tokio::time::timeout_at(deadline.into(), task).await {
        Ok(Ok(result)) => result.map_err(|_error| "the hook gave an error".to_owned())?,
        Ok(Err(error)) if error.is_panic() => return Err("the hook panicked".to_owned()),
        Ok(Err(_cancelled)) => return Err("the task of the hook was cancelled".to_owned()),
        Err(_elapsed) => {
            return Err(format!(
                "it did not end within {}ms of the start of the read",
                BUILD_TIMEOUT.as_millis()
            ));
        }
    };
    let mut redacted = redacted.into_iter();
    server_info_strings(body, &mut |text| {
        if let Some(new) = redacted.next() {
            *text = new;
        }
    });
    Ok(())
}

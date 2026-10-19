//! Customer redaction: the event-level hook and the string-level hook.

use futures::FutureExt;
use serde_json::Value;

use crate::event::Event;
use crate::hooks::{EventRedactor, StringRedactor};

/// Runs the event-level hook. `None` drops the event (the hook returned
/// `None` or panicked). System fields are restored from the original: the
/// hook cannot change `id`, `session_id`, `project_id`, `event_type` or
/// `timestamp`.
pub(crate) async fn apply_event_redaction(
    event: Event,
    hook: &EventRedactor,
) -> Result<Option<Event>, String> {
    let original = (
        event.id.clone(),
        event.session_id.clone(),
        event.project_id.clone(),
        event.event_type.clone(),
        event.timestamp,
    );
    let result = match hook {
        EventRedactor::Sync(f) => std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(event)))
            .map_err(|p| crate::util::panic_message(&*p))?,
        EventRedactor::Async(f) => {
            let fut = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(event)))
                .map_err(|p| crate::util::panic_message(&*p))?;
            std::panic::AssertUnwindSafe(fut)
                .catch_unwind()
                .await
                .map_err(|p| crate::util::panic_message(&*p))?
        }
    };
    Ok(result.map(|mut redacted| {
        (
            redacted.id,
            redacted.session_id,
            redacted.project_id,
            redacted.event_type,
            redacted.timestamp,
        ) = original;
        redacted
    }))
}

/// Where a string sits, so the async path can write results back.
enum Step {
    Key(String),
    Index(usize),
}

/// Applies the string-level hook to every string of the fields that can carry
/// customer data: `parameters`, `response`, `user_intent`, `error`,
/// `event_id` and `ip_address`.
///
/// Protected (never redacted): IDs, `event_type`, server and client names and
/// versions, actor ID and name, `identify_data`, `resource_name`, `tags`,
/// `properties`, and the SDK's own `sdk_language` and `agentcat_version`. The
/// `type` discriminator of response content blocks is skipped, so the
/// sanitizer still recognizes the blocks.
pub(crate) async fn redact_strings(event: &mut Event, redactor: &StringRedactor) -> Result<(), String> {
    match redactor {
        StringRedactor::Sync(f) => std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let f = |s: &str| f(s);
            if let Some(v) = event.parameters.as_mut() {
                walk_sync(v, &f, false);
            }
            if let Some(v) = event.response.as_mut() {
                walk_response_sync(v, &f);
            }
            {
                let field = &mut event.user_intent;
                if let Some(s) = field.as_mut() {
                    *s = f(s);
                }
            }
            if let Some(v) = event.error.as_mut() {
                walk_sync(v, &f, false);
            }
            for field in [&mut event.event_id, &mut event.ip_address] {
                if let Some(s) = field.as_mut() {
                    *s = f(s);
                }
            }
        }))
        .map_err(|p| crate::util::panic_message(&*p)),
        StringRedactor::Async(f) => {
            let call = |s: String| {
                let f = f.clone();
                async move {
                    let fut = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(s)))
                        .map_err(|p| crate::util::panic_message(&*p))?;
                    std::panic::AssertUnwindSafe(fut)
                        .catch_unwind()
                        .await
                        .map_err(|p| crate::util::panic_message(&*p))
                }
            };
            if let Some(v) = event.parameters.as_mut() {
                redact_value_async(v, &call, false).await?;
            }
            if let Some(v) = event.response.as_mut() {
                redact_value_async(v, &call, true).await?;
            }
            if let Some(s) = event.user_intent.take() {
                event.user_intent = Some(call(s).await?);
            }
            if let Some(v) = event.error.as_mut() {
                redact_value_async(v, &call, false).await?;
            }
            if let Some(s) = event.event_id.take() {
                event.event_id = Some(call(s).await?);
            }
            if let Some(s) = event.ip_address.take() {
                event.ip_address = Some(call(s).await?);
            }
            Ok(())
        }
    }
}

fn walk_sync(value: &mut Value, f: &dyn Fn(&str) -> String, skip_type_key: bool) {
    match value {
        Value::String(s) => *s = f(s),
        Value::Array(items) => items.iter_mut().for_each(|v| walk_sync(v, f, false)),
        Value::Object(map) => {
            for (key, v) in map.iter_mut() {
                if skip_type_key && key == "type" {
                    continue;
                }
                walk_sync(v, f, false);
            }
        }
        _ => {}
    }
}

fn walk_response_sync(response: &mut Value, f: &dyn Fn(&str) -> String) {
    let Value::Object(map) = response else {
        walk_sync(response, f, false);
        return;
    };
    for (key, v) in map.iter_mut() {
        if key == "content"
            && let Value::Array(blocks) = v
        {
            for block in blocks.iter_mut() {
                walk_sync(block, f, true);
            }
            continue;
        }
        walk_sync(v, f, false);
    }
}

fn collect(
    value: &Value,
    path: &mut Vec<Step>,
    out: &mut Vec<(Vec<Step>, String)>,
    skip_type_key: bool,
    is_response_root: bool,
) {
    match value {
        Value::String(s) => out.push((path.iter().map(Step::clone_step).collect(), s.clone())),
        Value::Array(items) => {
            for (i, v) in items.iter().enumerate() {
                path.push(Step::Index(i));
                collect(v, path, out, false, false);
                path.pop();
            }
        }
        Value::Object(map) => {
            for (key, v) in map {
                if skip_type_key && key == "type" {
                    continue;
                }
                path.push(Step::Key(key.clone()));
                if is_response_root && key == "content" && v.is_array() {
                    if let Value::Array(blocks) = v {
                        for (i, block) in blocks.iter().enumerate() {
                            path.push(Step::Index(i));
                            collect(block, path, out, true, false);
                            path.pop();
                        }
                    }
                } else {
                    collect(v, path, out, false, false);
                }
                path.pop();
            }
        }
        _ => {}
    }
}

impl Step {
    fn clone_step(&self) -> Step {
        match self {
            Step::Key(k) => Step::Key(k.clone()),
            Step::Index(i) => Step::Index(*i),
        }
    }
}

fn locate<'a>(root: &'a mut Value, path: &[Step]) -> Option<&'a mut Value> {
    let mut cur = root;
    for step in path {
        cur = match step {
            Step::Key(k) => cur.get_mut(k.as_str())?,
            Step::Index(i) => cur.get_mut(*i)?,
        };
    }
    Some(cur)
}

async fn redact_value_async<F, Fut>(value: &mut Value, call: &F, is_response: bool) -> Result<(), String>
where
    F: Fn(String) -> Fut,
    Fut: std::future::Future<Output = Result<String, String>>,
{
    let mut found = Vec::new();
    collect(value, &mut Vec::new(), &mut found, false, is_response);
    for (path, original) in found {
        let redacted = call(original).await?;
        if let Some(slot) = locate(value, &path) {
            *slot = Value::String(redacted);
        }
    }
    Ok(())
}

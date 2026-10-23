//! The event of a tool call as TS holds it in memory when its queue takes it (B-077):
//! [`from_call`] builds it from a [`CallRecord`], in TS's key order
//! ([`TOOL_CALL_KEYS`](crate::pipeline::TOOL_CALL_KEYS)), as the [`EventJob`] the delivery
//! pipeline processes. [`render_backtrace`] adds the backtrace of a captured error to it.
use std::backtrace::{Backtrace, BacktraceStatus};
use std::fmt;

use serde_json::{Map, Value};
use tokio::runtime::Handle;

use super::{CallRecord, redact_extra};
use crate::hooks::PendingHooks;
use crate::js;
use crate::pipeline::TsEvent;

#[cfg(test)]
mod tests;

/// The event type of a tool call (B-078).
const TOOL_CALL_EVENT_TYPE: &str = "mcp:tools/call";

/// The `sdkLanguage` of every event (D01, RD-14).
const SDK_LANGUAGE: &str = "Rust";

/// The key rmcp gives a tool result for its result type.
const RESULT_TYPE_KEY: &str = "resultType";

/// The result type rmcp writes into every complete tool result.
const RESULT_TYPE_COMPLETE: &str = "complete";

/// One event for the delivery pipeline, with what it needs to finish it: the hooks to settle and
/// the runtime and the redaction stages of the customer's hooks. Its `Debug` output hides the
/// values of the credential headers in `parameters.extra`, as the record's does.
pub(crate) struct EventJob {
    /// The event, in TS's key order.
    pub(crate) event: TsEvent,
    /// The customer hooks of the call, settled at stage 0.
    pub(crate) hooks: PendingHooks,
    /// Whether the session comes from the `resolve_session_id` hook.
    pub(crate) hook_mode: bool,
    /// The runtime of the customer's redaction hooks (C38).
    pub(crate) runtime: Option<Handle>,
    /// Whether the string-redaction hook runs on the event (tool calls only, K-03).
    pub(crate) string_redaction: bool,
    /// Whether the event-level redaction hook runs on the event.
    pub(crate) event_redaction: bool,
    /// The backtrace of a captured error, for [`render_backtrace`].
    pub(crate) backtrace: Option<Backtrace>,
}

impl fmt::Debug for EventJob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EventJob")
            .field("event", &redact_event(&self.event))
            .field("hooks", &self.hooks)
            .field("hook_mode", &self.hook_mode)
            .field("runtime", &self.runtime.is_some())
            .field("string_redaction", &self.string_redaction)
            .field("event_redaction", &self.event_redaction)
            .field("backtrace", &self.backtrace.as_ref().map(Backtrace::status))
            .finish()
    }
}

/// `event` for `Debug` output: its `parameters.extra` through [`redact_extra`].
fn redact_event(event: &TsEvent) -> TsEvent {
    let mut event = event.clone();
    if let Some(extra) = event.get_mut("parameters").and_then(|p| p.get_mut("extra")) {
        *extra = redact_extra(extra);
    }
    event
}

/// The TS in-memory event of the tool call `record` (B-077) for the project `project_id`, as
/// `publishEvent` builds it, with the keys in TS's order and a key absent where TS's value is
/// undefined:
///
/// `id` (`""` until the queue gives the event its ID), `sessionId` (`""` without a session),
/// `projectId`, `eventType` (`mcp:tools/call`), `timestamp` (the start of the call as
/// `toISOString` writes it), `duration`, `inputTokens`, `outputTokens`, `sdkLanguage` (`Rust`),
/// `agentcatVersion` (the crate version), `serverName`, `serverVersion`, `clientName`,
/// `clientVersion`, `identifyActorData` (`{}` until the `identify` hook settles),
/// `resourceName`, `parameters` (`{request, extra}`), `response` (the customer's result, without
/// the `resultType: "complete"` rmcp writes into it), `userIntent`, `isError` (only `true`),
/// `error` and `tags` (the SDK's tags). `identifyActorGivenId`, `identifyActorName` and
/// `properties` come from the hooks ([`apply`](crate::pipeline::settle::apply)).
///
/// The job takes the record's hooks, runtime and backtrace, with both redaction stages on.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "read by the delivery worker (plan 4 task 6)")
)]
pub(crate) fn from_call(record: CallRecord, project_id: &str, hook_mode: bool) -> EventJob {
    let CallRecord {
        started_at,
        duration_ms,
        resolution,
        resource_name,
        request,
        extra,
        input_tokens,
        output_tokens,
        tags,
        user_intent,
        response,
        is_error,
        error,
        server_name,
        server_version,
        client,
        pending,
        runtime,
        backtrace,
    } = record;
    let (client_name, client_version) =
        client.map_or((None, None), |client| (client.name, client.version));
    let text = |value: &str| Value::String(value.to_owned());

    let mut event = TsEvent::new();
    let mut put = |key: &str, value: Option<Value>| {
        if let Some(value) = value {
            event.insert(key.to_owned(), value);
        }
    };
    put("id", Some(text("")));
    put("sessionId", Some(Value::String(resolution.session_id)));
    put("projectId", Some(text(project_id)));
    put("eventType", Some(text(TOOL_CALL_EVENT_TYPE)));
    put("timestamp", Some(Value::String(js::iso_millis(started_at))));
    put("duration", Some(Value::from(duration_ms)));
    put("inputTokens", input_tokens.map(Value::from));
    put("outputTokens", output_tokens.map(Value::from));
    put("sdkLanguage", Some(text(SDK_LANGUAGE)));
    put("agentcatVersion", Some(text(env!("CARGO_PKG_VERSION"))));
    put("serverName", server_name.map(Value::String));
    put("serverVersion", server_version.map(Value::String));
    put("clientName", client_name.map(Value::String));
    put("clientVersion", client_version.map(Value::String));
    put("identifyActorData", Some(Value::Object(Map::new())));
    put("resourceName", Some(Value::String(resource_name)));
    let mut parameters = Map::new();
    parameters.insert("request".to_owned(), request);
    parameters.insert("extra".to_owned(), extra);
    put("parameters", Some(Value::Object(parameters)));
    put("response", response.map(without_complete_type));
    put("userIntent", user_intent);
    put("isError", is_error.then_some(Value::Bool(true)));
    put("error", error.map(Value::Object));
    put("tags", Some(Value::Object(tags)));

    EventJob {
        event,
        hooks: pending,
        hook_mode,
        runtime,
        string_redaction: true,
        event_redaction: true,
        backtrace,
    }
}

/// `response` without the `resultType: "complete"` rmcp writes into a complete tool result: the
/// customer's result in a TS event has none. Another result type, such as `input_required`,
/// stays, as the customer's result in TS carries it (B-025).
fn without_complete_type(response: Value) -> Value {
    match response {
        Value::Object(mut fields) => {
            if fields.get(RESULT_TYPE_KEY).and_then(Value::as_str) == Some(RESULT_TYPE_COMPLETE) {
                fields.shift_remove(RESULT_TYPE_KEY);
            }
            Value::Object(fields)
        }
        other => other,
    }
}

/// Sets `error.stack` of `event` to the text of `backtrace` when it was captured (RD-06): after
/// `platform` and before `chained_errors`, the place `capture_error` gives it. An event without
/// an `error` object is left as it is. Rendering resolves symbols, so the delivery worker calls
/// this, never the call path.
pub(crate) fn render_backtrace(event: &mut TsEvent, backtrace: &Backtrace) {
    if backtrace.status() != BacktraceStatus::Captured {
        return;
    }
    let Some(Value::Object(error)) = event.get_mut("error") else {
        return;
    };
    let index_of = |name: &str| error.keys().position(|key| key == name);
    let index = index_of("platform")
        .map(|platform| platform + 1)
        .or_else(|| index_of("chained_errors"))
        .unwrap_or(error.len());
    error.shift_insert(
        index,
        "stack".to_owned(),
        Value::String(backtrace.to_string()),
    );
}

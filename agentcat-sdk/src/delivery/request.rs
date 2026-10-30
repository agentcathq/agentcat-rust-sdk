//! The request that publishes one event: [`publish_request`] turns the event the pipeline
//! finished into `agentcat-api`'s `PublishEventRequest`.
use agentcat_api::models::PublishEventRequest;
use agentcat_api::models::publish_event_request::EventType;
use chrono::DateTime;
use serde_json::{Number, Value};

use crate::pipeline::EventObject;
use crate::wire;

#[cfg(test)]
mod tests;

/// The request that publishes `event`:
///
/// - `id`, `project_id` (`""` without one), the string fields (`resource_name`,
///   `identify_actor_given_id`, `identify_actor_name`, `sdk_language`, `agentcat_version`, the
///   server's and the client's name and version) and `is_error`, as the event holds them.
/// - `session_id` is `null` when the event has no session (`""` or no key).
/// - `timestamp` is the event's ISO-8601 text read as a date, its milliseconds kept (chrono writes
///   it without a zero fraction).
/// - `duration`, `input_tokens` and `output_tokens` are the event's numbers, saturated to `i32`.
/// - `event_type` is the variant of `mcp:tools/call` or `agentcat:custom`; any other text has
///   none and is left out.
/// - `parameters`, `response`, `error` and `identify_data` (the event's `identifyActorData`) are
///   the event's values through [`wire::as_published`], so the numbers and key orders are those of
///   [`wire::stringify`]; `properties` too, left out when it is `null` or absent.
/// - `user_intent` is a string `userIntent` as it is, any other value as its [`wire::stringify`]
///   text, and `null` as no value.
/// - `tags` are the event's string tags, left out when the event's `tags` is not an object.
/// - `actor_id` is `identify_actor_given_id`: an event has no actor ID of its own. `ip_address`
///   and `event_id` are never set.
pub(crate) fn publish_request(event: &EventObject) -> PublishEventRequest {
    let text = |key: &str| event.get(key).and_then(Value::as_str).map(str::to_owned);
    let json = |key: &str| event.get(key).map(wire::as_published);
    let number = |key: &str| event.get(key).and_then(Value::as_number).map(saturate);
    let identify_actor_given_id = text("identifyActorGivenId");
    PublishEventRequest {
        id: text("id"),
        project_id: text("projectId").unwrap_or_default(),
        session_id: Some(text("sessionId").filter(|session| !session.is_empty())),
        actor_id: identify_actor_given_id.clone(),
        event_id: None,
        event_type: event
            .get("eventType")
            .and_then(Value::as_str)
            .and_then(event_type),
        is_error: event.get("isError").and_then(Value::as_bool),
        error: json("error"),
        resource_name: text("resourceName"),
        duration: number("duration"),
        input_tokens: number("inputTokens"),
        output_tokens: number("outputTokens"),
        timestamp: event
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(|time| DateTime::parse_from_rfc3339(time).ok()),
        user_intent: event.get("userIntent").and_then(user_intent),
        parameters: json("parameters"),
        response: json("response"),
        identify_actor_given_id,
        identify_actor_name: text("identifyActorName"),
        identify_data: json("identifyActorData"),
        tags: event.get("tags").and_then(Value::as_object).map(|tags| {
            tags.iter()
                .filter_map(|(key, tag)| Some((key.clone(), tag.as_str()?.to_owned())))
                .collect()
        }),
        properties: event
            .get("properties")
            .filter(|properties| !properties.is_null())
            .map(wire::as_published),
        ip_address: None,
        sdk_language: text("sdkLanguage"),
        agentcat_version: text("agentcatVersion"),
        server_name: text("serverName"),
        server_version: text("serverVersion"),
        client_name: text("clientName"),
        client_version: text("clientVersion"),
    }
}

/// The `agentcat-api` variant of the event type `text`, for the two types the SDK publishes.
fn event_type(text: &str) -> Option<EventType> {
    match text {
        "mcp:tools/call" => Some(EventType::McpColonToolsSlashCall),
        "agentcat:custom" => Some(EventType::AgentcatColonCustom),
        _ => None,
    }
}

/// `number` saturated to the `i32` range; a float loses its fraction (the SDK's numbers here are
/// whole milliseconds and token counts).
#[expect(
    clippy::cast_possible_truncation,
    reason = "a float-to-int `as` saturates, which is the intent"
)]
fn saturate(number: &Number) -> i32 {
    if let Some(integer) = number.as_i64() {
        let clamped = integer.clamp(i64::from(i32::MIN), i64::from(i32::MAX));
        return i32::try_from(clamped).unwrap_or_default();
    }
    if number.is_u64() {
        return i32::MAX;
    }
    number.as_f64().map_or(0, |float| float as i32)
}

/// The `user_intent` of the event's `userIntent`: a string as it is, `null` as none, another
/// value as its [`wire::stringify`] text.
fn user_intent(intent: &Value) -> Option<String> {
    match intent {
        Value::Null => None,
        Value::String(text) => Some(text.clone()),
        other => Some(wire::stringify(other)),
    }
}

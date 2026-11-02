//! The judges of the examples check: pure functions from what the client and the mock saw to
//! the failure texts.
use std::time::Instant;

use serde_json::{Value, json};

use super::mock::Recorded;
use super::{Call, PROJECT_ID};

/// The failures of a `tools/list` answer (`tools`, a JSON list of tools) that misses a tool of
/// `expected`.
pub(super) fn judge_tools(tools: &Value, expected: &[&str]) -> Vec<String> {
    let names: Vec<&str> = tools
        .as_array()
        .map(|tools| {
            tools
                .iter()
                .filter_map(|tool| tool["name"].as_str())
                .collect()
        })
        .unwrap_or_default();
    expected
        .iter()
        .filter(|tool| !names.contains(tool))
        .map(|tool| format!("tools/list does not list {tool}: {}", json!(names)))
        .collect()
}

/// The ID in a text block that issues a session: the `ses_` word after `session_id: `.
pub(super) fn issued_session_id(text: &str) -> Option<String> {
    const MARK: &str = "session_id: ses_";
    text.match_indices(MARK).find_map(|(at, _)| {
        let rest = &text[at + MARK.len()..];
        let end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        (end > 0).then(|| format!("ses_{}", &rest[..end]))
    })
}

/// The failures of the result of `call` (`result`, a JSON `CallToolResult`).
///
/// The first call of a server has no session ID (`session` is `None`). The SDK puts a text block
/// that holds the ID it issues before the tool's own result, which it keeps; the later calls pass
/// that ID back. So the tool's text is the first block of a later call, and the block after the
/// issued one on the first call. After the first call, `session` holds the issued ID, or an empty
/// text when the call issued none.
pub(super) fn judge_call(call: &Call, result: &Value, session: &mut Option<String>) -> Vec<String> {
    let mut failures = Vec::new();
    let content: &[Value] = result["content"].as_array().map_or(&[], Vec::as_slice);
    let mut rest = content;
    if session.is_none() {
        let first = content.first().and_then(|block| block["text"].as_str());
        if let Some(id) = first.and_then(issued_session_id) {
            *session = Some(id);
            rest = &content[1..];
        } else {
            failures.push(format!(
                "the first call ({}) did not issue a session ID: {}",
                call.tool,
                json!(first)
            ));
            *session = Some(String::new());
        }
    }
    let texts: Vec<Value> = rest.iter().map(|block| block["text"].clone()).collect();
    let got = match texts.as_slice() {
        [one] => one.clone(),
        _ => Value::Array(texts),
    };
    let is_error = is_truthy(&result["isError"]);
    if got != call.text || is_error != call.is_error {
        failures.push(format!(
            "{} returned {got} (isError {is_error}), expected {} (isError {})",
            call.tool,
            json!(call.text),
            call.is_error
        ));
    }
    failures
}

/// Whether a JSON value counts as true: not `null`, `false`, `0` or an empty text.
pub(super) fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|n| n != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// The field `name` of a publish body; `None` when it is absent or null.
pub(super) fn field<'a>(event: &'a Value, name: &str) -> Option<&'a Value> {
    event.get(name).filter(|value| !value.is_null())
}

/// The event type of a publish body.
pub(super) fn event_type(event: &Value) -> Option<&str> {
    field(event, "event_type").and_then(Value::as_str)
}

/// The resource name of a publish body.
pub(super) fn resource_name(event: &Value) -> Option<&str> {
    field(event, "resource_name").and_then(Value::as_str)
}

/// The failures of the requests the mock received: one tool-call publish per call (`calls`),
/// every publish for [`PROJECT_ID`], no request other than a publish, and the publish of the
/// last call before the exit at `exited_at` (`None` when the example did not exit in time).
pub(super) fn judge_publishes(
    requests: &[Recorded],
    calls: &[Call],
    exited_at: Option<Instant>,
) -> Vec<String> {
    let mut failures = Vec::new();
    let (publishes, stray): (Vec<&Recorded>, Vec<&Recorded>) =
        requests.iter().partition(|request| request.is_publish());
    if !stray.is_empty() {
        failures.push(format!(
            "the mock received {} request(s) that are not publishes",
            stray.len()
        ));
    }
    let tool_calls: Vec<&Recorded> = publishes
        .iter()
        .copied()
        .filter(|request| event_type(&request.body) == Some("mcp:tools/call"))
        .collect();
    let got: Vec<Option<&str>> = tool_calls
        .iter()
        .map(|request| resource_name(&request.body))
        .collect();
    let want: Vec<&str> = calls.iter().map(|call| call.tool).collect();
    let mut got_sorted = got.clone();
    got_sorted.sort_unstable();
    let mut want_sorted: Vec<Option<&str>> = want.iter().copied().map(Some).collect();
    want_sorted.sort_unstable();
    if got_sorted != want_sorted {
        failures.push(format!(
            "publishes for the tool calls: got {}, expected {}",
            json!(got),
            json!(want)
        ));
    }
    let last = want.last().copied();
    let last_call = tool_calls
        .iter()
        .rev()
        .find(|request| resource_name(&request.body) == last);
    if last_call
        .zip(exited_at)
        .is_some_and(|(request, exited_at)| request.received > exited_at)
    {
        failures.push("the publish of the last call arrived after the example exited".into());
    }
    for request in &publishes {
        let project = field(&request.body, "project_id");
        if project.and_then(Value::as_str) != Some(PROJECT_ID) {
            failures.push(format!(
                "a publish names the project {}, expected {PROJECT_ID}",
                project.unwrap_or(&Value::Null)
            ));
        }
    }
    failures
}

/// The failures of the hook values of the `hooks` example in the publishes of `requests`.
pub(super) fn judge_hooks(requests: &[Recorded]) -> Vec<String> {
    let mut failures = Vec::new();
    let publishes: Vec<&Recorded> = requests.iter().filter(|r| r.is_publish()).collect();
    let tool_events: Vec<&Value> = publishes
        .iter()
        .map(|request| &request.body)
        .filter(|event| event_type(event) == Some("mcp:tools/call"))
        .collect();
    for event in &tool_events {
        let tool = resource_name(event).unwrap_or("<absent>");
        let text = |name: &str| field(event, name).and_then(Value::as_str);
        if text("identify_actor_given_id") != Some("demo-user") {
            failures.push(format!("{tool}: identify user_id is not \"demo-user\""));
        }
        if text("identify_actor_name") != Some("Demo User") {
            failures.push(format!("{tool}: identify user_name is not \"Demo User\""));
        }
        if event["tags"]["example"] != "hooks" {
            failures.push(format!("{tool}: the tag example is not \"hooks\""));
        }
        if event["properties"]["source"] != "hooks-example" {
            failures.push(format!(
                "{tool}: the property source is not \"hooks-example\""
            ));
        }
        if event["properties"]["redacted_by"] != "example" {
            failures.push(format!("{tool}: the redact_event hook did not run"));
        }
    }
    if publishes.iter().any(|r| r.raw.contains("secret-token")) {
        failures.push("a publish still holds secret-token".to_owned());
    }
    if !publishes.iter().any(|r| r.raw.contains("[REDACTED]")) {
        failures.push("no publish holds [REDACTED]".to_owned());
    }
    let fail_event = tool_events
        .iter()
        .find(|event| resource_name(event) == Some("fail"));
    let recorded = fail_event.is_some_and(|event| {
        let error = &event["error"];
        let error_type = field(error, "type").unwrap_or(&Value::Null);
        field(event, "is_error") == Some(&Value::Bool(true)) && is_truthy(error_type)
    });
    if !recorded {
        failures.push("the fail call has no error recorded by capture_error".to_owned());
    }
    let customs: Vec<&Value> = publishes
        .iter()
        .map(|request| &request.body)
        .filter(|event| event_type(event) == Some("agentcat:custom"))
        .collect();
    match customs.as_slice() {
        [custom] => {
            let name = field(custom, "resource_name");
            if name.and_then(Value::as_str) != Some("echo-called") {
                failures.push(format!(
                    "the custom event has resource {}, expected \"echo-called\"",
                    name.unwrap_or(&Value::Null)
                ));
            }
        }
        _ => failures.push(format!("expected 1 custom event, got {}", customs.len())),
    }
    failures
}

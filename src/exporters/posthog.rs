//! PostHog exporter: a capture event per AgentCat event, an `$exception` for
//! failures, and an optional `$ai_span` for tool calls.

use std::sync::Arc;

use serde_json::{Map, Value, json};

use super::Exporter;
use crate::constants::{AGENTCAT_SOURCE, MCP_TOOLS_CALL_EVENT_TYPE};
use crate::event::Event;
use crate::hooks::BoxFuture;
use crate::ksuid::Ksuid;
use crate::logging::write_to_log;
use crate::util::{hex, iso_millis, sha256};

/// Sends events to PostHog's batch API.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct PostHogExporterConfig {
    /// Project API key (`phc_…`).
    pub api_key: String,
    /// PostHog host. Default `https://us.i.posthog.com`.
    pub host: Option<String>,
    /// Also send `$ai_span` events for tool calls (PostHog AI observability).
    /// Customer tags and properties are spread onto the span and can
    /// override any default. Default false.
    pub enable_ai_tracing: bool,
}

impl PostHogExporterConfig {
    /// An exporter with the project API key.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            host: None,
            enable_ai_tracing: false,
        }
    }

    /// Sets the host (EU cloud or self-hosted).
    pub fn host(mut self, host: impl Into<String>) -> Self {
        self.host = Some(host.into());
        self
    }

    /// Turns `$ai_span` events on or off.
    pub fn enable_ai_tracing(mut self, enabled: bool) -> Self {
        self.enable_ai_tracing = enabled;
        self
    }
}

/// A deterministic UUIDv7 from a prefixed KSUID (`ses_…`): the KSUID's time
/// for the timestamp bits and SHA-256 of the full ID for the rest. Inputs that
/// are not KSUIDs take their timestamp from the hash too, so the same input
/// always gives the same UUID.
pub(crate) fn to_uuid_v7(prefixed_id: &str) -> String {
    let hash = sha256(prefixed_id.as_bytes());
    let bare = strip_lowercase_prefix(prefixed_id);
    let timestamp_ms = match Ksuid::parse(bare) {
        Some(k) => k.time_ms(),
        None => hash[10..16].iter().fold(0u64, |acc, &b| (acc << 8) | u64::from(b)),
    };
    let mut buf = [0u8; 16];
    buf[..6].copy_from_slice(&timestamp_ms.to_be_bytes()[2..8]);
    buf[6] = 0x70 | (hash[0] & 0x0f);
    buf[7] = hash[1];
    buf[8] = 0x80 | (hash[2] & 0x3f);
    buf[9..16].copy_from_slice(&hash[3..10]);
    let h = hex(&buf);
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

/// Removes a leading `^[a-z]+_` prefix.
fn strip_lowercase_prefix(id: &str) -> &str {
    let letters = id.bytes().take_while(u8::is_ascii_lowercase).count();
    if letters > 0 && id.as_bytes().get(letters) == Some(&b'_') {
        &id[letters + 1..]
    } else {
        id
    }
}

fn distinct_id(event: &Event) -> String {
    event
        .identify_actor_given_id
        .clone()
        .filter(|s| !s.is_empty())
        .or_else(|| (!event.session_id.is_empty()).then(|| event.session_id.clone()))
        .unwrap_or_else(|| "anonymous".into())
}

pub(crate) fn map_event_type(event_type: &str) -> String {
    match event_type {
        "mcp:tools/call" => "mcp_tool_call".into(),
        "mcp:tools/list" => "mcp_tools_list".into(),
        "mcp:initialize" => "mcp_initialize".into(),
        "mcp:resources/read" => "mcp_resource_read".into(),
        "mcp:resources/list" => "mcp_resources_list".into(),
        "mcp:prompts/get" => "mcp_prompt_get".into(),
        "mcp:prompts/list" => "mcp_prompts_list".into(),
        other => format!(
            "mcp_{}",
            other.strip_prefix("mcp:").unwrap_or(other).replace('/', "_")
        ),
    }
}

fn spread_customer(properties: &mut Map<String, Value>, event: &Event) {
    if let Some(tags) = &event.tags {
        for (k, v) in tags {
            properties.insert(k.clone(), json!(v));
        }
    }
    if let Some(props) = &event.properties {
        for (k, v) in props {
            properties.insert(k.clone(), v.clone());
        }
    }
}

fn nonempty(v: &Option<String>) -> Option<&str> {
    v.as_deref().filter(|s| !s.is_empty())
}

pub(crate) fn capture_event(event: &Event) -> Value {
    let mut p = Map::new();
    p.insert("source".into(), json!(AGENTCAT_SOURCE));
    // Sessionless events get no $session_id: a synthesized one would group
    // unrelated events into a fake session.
    if !event.session_id.is_empty() {
        p.insert("$session_id".into(), json!(to_uuid_v7(&event.session_id)));
    }
    if let Some(r) = nonempty(&event.resource_name) {
        p.insert("resource_name".into(), json!(r));
        if event.event_type == MCP_TOOLS_CALL_EVENT_TYPE {
            p.insert("tool_name".into(), json!(r));
        }
    }
    if let Some(d) = event.duration {
        p.insert("duration_ms".into(), json!(d));
    }
    for (k, v) in [
        ("server_name", &event.server_name),
        ("server_version", &event.server_version),
        ("client_name", &event.client_name),
        ("client_version", &event.client_version),
        ("project_id", &event.project_id),
        ("user_intent", &event.user_intent),
    ] {
        if let Some(v) = nonempty(v) {
            p.insert(k.into(), json!(v));
        }
    }
    if let Some(e) = event.is_error {
        p.insert("is_error".into(), json!(e));
    }
    if let Some(v) = &event.parameters {
        p.insert("parameters".into(), v.clone());
    }
    if let Some(v) = &event.response {
        p.insert("response".into(), v.clone());
    }
    let mut set = Map::new();
    if let Some(name) = nonempty(&event.identify_actor_name) {
        set.insert("name".into(), json!(name));
    }
    if let Some(Value::Object(data)) = &event.identify_actor_data {
        for (k, v) in data {
            set.insert(k.clone(), v.clone());
        }
    }
    if !set.is_empty() {
        p.insert("$set".into(), Value::Object(set));
    }
    spread_customer(&mut p, event);
    json!({
        "event": map_event_type(&event.event_type),
        "distinct_id": distinct_id(event),
        "properties": p,
        "timestamp": iso_millis(&event.timestamp),
        "type": "capture",
    })
}

pub(crate) fn exception_event(event: &Event) -> Value {
    let mut p = Map::new();
    p.insert("$exception_source".into(), json!("backend"));
    if !event.session_id.is_empty() {
        p.insert("$session_id".into(), json!(to_uuid_v7(&event.session_id)));
    }
    if let Some(Value::Object(error)) = &event.error {
        for (src, dst) in [
            ("message", "$exception_message"),
            ("type", "$exception_type"),
            ("stack", "$exception_stacktrace"),
        ] {
            if let Some(v) = error.get(src).filter(|v| match v {
                Value::Null | Value::Bool(false) => false,
                Value::String(s) => !s.is_empty(),
                _ => true,
            }) {
                p.insert(dst.into(), v.clone());
            }
        }
    }
    if let Some(r) = nonempty(&event.resource_name) {
        p.insert("resource_name".into(), json!(r));
        if event.event_type == MCP_TOOLS_CALL_EVENT_TYPE {
            p.insert("tool_name".into(), json!(r));
        }
    }
    for (k, v) in [
        ("server_name", &event.server_name),
        ("server_version", &event.server_version),
        ("client_name", &event.client_name),
        ("client_version", &event.client_version),
    ] {
        if let Some(v) = nonempty(v) {
            p.insert(k.into(), json!(v));
        }
    }
    json!({
        "event": "$exception",
        "distinct_id": distinct_id(event),
        "properties": p,
        "timestamp": iso_millis(&event.timestamp),
        "type": "capture",
    })
}

pub(crate) fn ai_span_event(event: &Event) -> Value {
    let trace_source = if event.session_id.is_empty() { &event.id } else { &event.session_id };
    let mut p = Map::new();
    p.insert("$ai_trace_id".into(), json!(to_uuid_v7(trace_source)));
    p.insert("$ai_span_id".into(), json!(to_uuid_v7(&event.id)));
    p.insert(
        "$ai_span_name".into(),
        json!(nonempty(&event.resource_name).unwrap_or("unknown_tool")),
    );
    p.insert("$ai_is_error".into(), json!(event.is_error.unwrap_or(false)));
    p.insert("source".into(), json!(AGENTCAT_SOURCE));
    if !event.session_id.is_empty() {
        p.insert("$ai_session_id".into(), json!(format!("agentcat_{}", event.session_id)));
        p.insert("$session_id".into(), json!(to_uuid_v7(&event.session_id)));
    }
    if let Some(d) = event.duration {
        p.insert("$ai_latency".into(), json!(d as f64 / 1000.0));
    }
    if event.is_error == Some(true) {
        if let Some(e) = &event.error {
            p.insert("$ai_error".into(), e.clone());
        }
    }
    if let Some(v) = &event.parameters {
        p.insert("$ai_input_state".into(), v.clone());
    }
    if let Some(v) = &event.response {
        p.insert("$ai_output_state".into(), v.clone());
    }
    if let Some(v) = nonempty(&event.server_name) {
        p.insert("server_name".into(), json!(v));
    }
    if let Some(v) = nonempty(&event.client_name) {
        p.insert("client_name".into(), json!(v));
    }
    spread_customer(&mut p, event);
    json!({
        "event": "$ai_span",
        "distinct_id": distinct_id(event),
        "properties": p,
        "timestamp": iso_millis(&event.timestamp),
        "type": "capture",
    })
}

pub(crate) struct PostHogExporter {
    config: PostHogExporterConfig,
    batch_url: String,
}

impl PostHogExporter {
    pub(crate) fn new(config: &PostHogExporterConfig) -> Self {
        let host = config.host.as_deref().unwrap_or("https://us.i.posthog.com");
        let host = host.strip_suffix('/').unwrap_or(host);
        let batch_url = format!("{host}/batch");
        write_to_log(&format!("PostHogExporter: Initialized with endpoint {batch_url}"));
        Self {
            config: config.clone(),
            batch_url,
        }
    }

    pub(crate) fn batch(&self, event: &Event) -> Vec<Value> {
        let mut batch = vec![capture_event(event)];
        if event.is_error == Some(true) && event.error.is_some() {
            batch.push(exception_event(event));
        }
        if self.config.enable_ai_tracing && event.event_type == MCP_TOOLS_CALL_EVENT_TYPE {
            batch.push(ai_span_event(event));
        }
        batch
    }
}

impl Exporter for PostHogExporter {
    fn export(&self, client: reqwest::Client, event: Arc<Event>) -> BoxFuture<Result<(), String>> {
        let batch = self.batch(&event);
        let url = self.batch_url.clone();
        let api_key = self.config.api_key.clone();
        Box::pin(async move {
            write_to_log(&format!(
                "PostHogExporter: Sending {} event(s) for {}",
                batch.len(),
                event.id
            ));
            let response = client
                .post(&url)
                .json(&json!({"api_key": api_key, "batch": batch}))
                .send()
                .await;
            match response {
                Ok(r) if r.status().is_success() => {
                    write_to_log(&format!("PostHog export success - Event: {}", event.id))
                }
                Ok(r) => {
                    let status = r.status().as_u16();
                    let body = r.text().await.unwrap_or_default();
                    write_to_log(&format!("PostHog export failed - Status: {status}, Body: {body}"));
                }
                Err(e) => write_to_log(&format!("PostHog export error: {e}")),
            }
            Ok(())
        })
    }
}

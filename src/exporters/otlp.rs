//! OTLP/HTTP JSON trace exporter: one server span per event.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Value, json};

use super::{Exporter, check, trace_context};
use crate::constants::AGENTCAT_SOURCE;
use crate::event::Event;
use crate::hooks::BoxFuture;
use crate::logging::write_to_log;

/// Sends each event as an OTLP span to an OpenTelemetry collector.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct OtlpExporterConfig {
    /// Collector URL. `/v1/traces` is appended when missing.
    pub endpoint: String,
    /// Extra request headers, for example an API key.
    pub headers: BTreeMap<String, String>,
}

impl OtlpExporterConfig {
    /// An exporter to `endpoint`, for example `http://localhost:4318`.
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            headers: BTreeMap::new(),
        }
    }

    /// Adds a request header.
    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(name.into(), value.into());
        self
    }
}

pub(crate) struct OtlpExporter {
    endpoint: String,
    headers: BTreeMap<String, String>,
}

impl OtlpExporter {
    pub(crate) fn new(config: &OtlpExporterConfig) -> Self {
        let url = config.endpoint.trim_end_matches('/');
        let endpoint = if url.ends_with("/v1/traces") {
            url.to_owned()
        } else {
            format!("{url}/v1/traces")
        };
        Self {
            endpoint,
            headers: config.headers.clone(),
        }
    }
}

fn attr(key: &str, value: &str) -> Value {
    json!({"key": key, "value": {"stringValue": value}})
}

pub(crate) fn span(event: &Event) -> Value {
    let start = event.timestamp.timestamp_millis() as i128 * 1_000_000;
    let end = match event.duration {
        Some(d) if d > 0 => start + d as i128 * 1_000_000,
        _ => start,
    };
    let opt = |v: &Option<String>| v.clone().unwrap_or_default();
    let mut attributes = vec![
        attr("source", AGENTCAT_SOURCE),
        attr("mcp.event_type", &event.event_type),
        attr("mcp.session_id", &event.session_id),
        attr("mcp.project_id", &opt(&event.project_id)),
        attr("mcp.resource_name", &opt(&event.resource_name)),
        attr("mcp.user_intent", &opt(&event.user_intent)),
        attr("mcp.actor_id", &opt(&event.identify_actor_given_id)),
        attr("mcp.actor_name", &opt(&event.identify_actor_name)),
        attr("mcp.client_name", &opt(&event.client_name)),
        attr("mcp.client_version", &opt(&event.client_version)),
    ];
    if let Some(tags) = &event.tags {
        for (key, value) in tags {
            attributes.push(attr(&format!("agentcat.tag.{key}"), value));
        }
    }
    if let Some(props) = &event.properties {
        let json = serde_json::to_string(props).unwrap_or_default();
        attributes.push(attr("agentcat.properties", &json));
    }
    // Empty attributes are dropped.
    attributes.retain(|a| a["value"]["stringValue"].as_str().is_some_and(|s| !s.is_empty()));

    json!({
        "traceId": trace_context::trace_id(&event.session_id),
        "spanId": trace_context::span_id(&event.id),
        "name": if event.event_type.is_empty() { "mcp.event" } else { event.event_type.as_str() },
        "kind": 2,
        "startTimeUnixNano": start.to_string(),
        "endTimeUnixNano": end.to_string(),
        "attributes": attributes,
        "status": {"code": if event.is_error == Some(true) { 2 } else { 1 }},
    })
}

pub(crate) fn request_body(event: &Event) -> Value {
    json!({
        "resourceSpans": [{
            "resource": {"attributes": [
                attr("service.name", event.server_name.as_deref().filter(|s| !s.is_empty()).unwrap_or("mcp-server")),
                attr("service.version", event.server_version.as_deref().filter(|s| !s.is_empty()).unwrap_or("unknown")),
            ]},
            "scopeSpans": [{
                "scope": {"name": "agentcat", "version": event.agentcat_version.as_deref().filter(|s| !s.is_empty()).unwrap_or("unknown")},
                "spans": [span(event)],
            }],
        }],
    })
}

impl Exporter for OtlpExporter {
    fn export(&self, client: reqwest::Client, event: Arc<Event>) -> BoxFuture<Result<(), String>> {
        let endpoint = self.endpoint.clone();
        let headers = self.headers.clone();
        Box::pin(async move {
            let mut req = client.post(&endpoint).json(&request_body(&event));
            for (k, v) in &headers {
                req = req.header(k, v);
            }
            check(req.send().await, "OTLP export")
                .await
                .map_err(|e| format!("OTLP export error: {e}"))?;
            write_to_log(&format!("Successfully exported event to OTLP: {}", event.id));
            Ok(())
        })
    }
}

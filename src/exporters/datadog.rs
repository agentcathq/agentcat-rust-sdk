//! Datadog exporter: one log entry and up to three metric series per event.

use std::sync::Arc;

use serde_json::{Map, Value, json};

use super::{Exporter, trace_context};
use crate::constants::AGENTCAT_SOURCE;
use crate::event::Event;
use crate::hooks::BoxFuture;
use crate::logging::write_to_log;

/// Sends each event to Datadog's log intake and metrics API.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct DatadogExporterConfig {
    /// Datadog API key.
    pub api_key: String,
    /// Datadog site, for example `datadoghq.com` or `datadoghq.eu`.
    pub site: String,
    /// The service name, usually the MCP server's name.
    pub service: String,
    /// Optional `env` tag.
    pub env: Option<String>,
}

impl DatadogExporterConfig {
    /// An exporter for `service` on `site`.
    pub fn new(api_key: impl Into<String>, site: impl Into<String>, service: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            site: site.into(),
            service: service.into(),
            env: None,
        }
    }

    /// Sets the `env` tag.
    pub fn env(mut self, env: impl Into<String>) -> Self {
        self.env = Some(env.into());
        self
    }
}

pub(crate) struct DatadogExporter {
    config: DatadogExporterConfig,
    logs_url: String,
    metrics_url: String,
}

impl DatadogExporter {
    pub(crate) fn new(config: &DatadogExporterConfig) -> Self {
        let site = config.site.trim_start_matches("https://").trim_start_matches("http://");
        let site = site.strip_suffix('/').unwrap_or(site);
        Self {
            config: config.clone(),
            logs_url: format!("https://http-intake.logs.{site}/api/v2/logs"),
            metrics_url: format!("https://api.{site}/api/v1/series"),
        }
    }

    fn common_tags(&self, event: &Event) -> Vec<String> {
        let mut tags = Vec::new();
        if let Some(env) = &self.config.env {
            tags.push(format!("env:{env}"));
        }
        if !event.event_type.is_empty() {
            tags.push(format!("event_type:{}", event.event_type.replace('/', ".")));
        }
        if let Some(r) = event.resource_name.as_deref().filter(|r| !r.is_empty()) {
            tags.push(format!("resource:{r}"));
        }
        tags
    }

    pub(crate) fn log(&self, event: &Event) -> Value {
        let mut tags = self.common_tags(event);
        if event.is_error == Some(true) {
            tags.push("error:true".into());
        }
        tags.push(format!("source:{AGENTCAT_SOURCE}"));
        // Customer tags, namespaced away from Datadog's reserved tags.
        if let Some(customer) = &event.tags {
            for (key, value) in customer {
                let mut sanitized = String::with_capacity(key.len());
                let mut in_run = false;
                for c in key.to_lowercase().chars() {
                    if c.is_whitespace() || c == ':' || c == ',' {
                        if !in_run {
                            sanitized.push('_');
                        }
                        in_run = true;
                    } else {
                        sanitized.push(c);
                        in_run = false;
                    }
                }
                tags.push(format!("agentcat.{sanitized}:{}", value.replace(',', "_")));
            }
        }

        let mut mcp = Map::new();
        let mut put = |k: &str, v: Option<Value>| {
            if let Some(v) = v {
                mcp.insert(k.to_owned(), v);
            }
        };
        let s = |v: &Option<String>| v.clone().map(Value::String);
        put("session_id", Some(Value::String(event.session_id.clone())));
        put("event_id", Some(Value::String(event.id.clone())));
        put("event_type", Some(Value::String(event.event_type.clone())));
        put("resource", s(&event.resource_name));
        put("duration_ms", event.duration.map(Value::from));
        put("user_intent", s(&event.user_intent));
        put("actor_id", s(&event.identify_actor_given_id));
        put("actor_name", s(&event.identify_actor_name));
        put("client_name", s(&event.client_name));
        put("client_version", s(&event.client_version));
        put("server_name", s(&event.server_name));
        put("server_version", s(&event.server_version));
        put("is_error", event.is_error.map(Value::Bool));
        put("error", event.error.clone());
        put("tags", event.tags.as_ref().map(|t| json!(t)));
        put("properties", event.properties.clone().map(Value::Object));

        let event_type = if event.event_type.is_empty() { "unknown" } else { &event.event_type };
        let resource = event.resource_name.as_deref().filter(|r| !r.is_empty()).unwrap_or("unknown");
        let mut log = json!({
            "message": format!("{event_type} - {resource}"),
            "service": self.config.service,
            "ddsource": AGENTCAT_SOURCE,
            "ddtags": tags.join(","),
            "timestamp": event.timestamp.timestamp_millis(),
            "status": if event.is_error == Some(true) { "error" } else { "info" },
            "dd": {
                "trace_id": trace_context::datadog_trace_id(&event.session_id),
                "span_id": trace_context::datadog_span_id(&event.id),
            },
            "mcp": mcp,
        });
        if event.is_error == Some(true) {
            if let Some(error) = &event.error {
                let message = match error {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                log["error"] = json!({"message": message});
            }
        }
        log
    }

    pub(crate) fn metrics(&self, event: &Event) -> Vec<Value> {
        let ts = event.timestamp.timestamp_millis().div_euclid(1000);
        let mut tags = vec![format!("service:{}", self.config.service)];
        tags.extend(self.common_tags(event));
        let mut out = vec![json!({"metric": "mcp.events.count", "type": "count", "points": [[ts, 1]], "tags": tags})];
        if let Some(d) = event.duration.filter(|d| *d > 0) {
            out.push(json!({"metric": "mcp.event.duration", "type": "gauge", "points": [[ts, d]], "tags": tags}));
        }
        if event.is_error == Some(true) {
            out.push(json!({"metric": "mcp.errors.count", "type": "count", "points": [[ts, 1]], "tags": tags}));
        }
        out
    }
}

impl Exporter for DatadogExporter {
    fn export(&self, client: reqwest::Client, event: Arc<Event>) -> BoxFuture<Result<(), String>> {
        let log = self.log(&event);
        let metrics = self.metrics(&event);
        let logs_url = self.logs_url.clone();
        let metrics_url = self.metrics_url.clone();
        let api_key = self.config.api_key.clone();
        Box::pin(async move {
            write_to_log(&format!("DatadogExporter: Sending {} metric series", metrics.len()));
            let logs = client
                .post(&logs_url)
                .header("DD-API-KEY", &api_key)
                .json(&vec![log])
                .send();
            let series = client
                .post(&metrics_url)
                .header("DD-API-KEY", &api_key)
                .json(&json!({"series": metrics}))
                .send();
            let (logs, series) = tokio::join!(logs, series);
            for (what, result) in [("logs", logs), ("metrics", series)] {
                match result {
                    Ok(r) if r.status().is_success() => {
                        write_to_log(&format!("Datadog {what} success - Status: {}", r.status().as_u16()))
                    }
                    Ok(r) => write_to_log(&format!("Datadog {what} failed - Status: {}", r.status().as_u16())),
                    Err(e) => write_to_log(&format!("Datadog {what} network error: {e}")),
                }
            }
            Ok(())
        })
    }
}

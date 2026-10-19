//! Sentry exporter: a structured log for every event, an error event for
//! failures, and (optionally) a performance transaction.
//!
//! Envelopes are written by hand; the `sentry` crate would install a global
//! hub and panic hook, which a library must not do.

use std::sync::Arc;

use serde_json::{Map, Value, json};

use super::{Exporter, epoch_seconds, trace_context};
use crate::constants::{AGENTCAT_SOURCE, SDK_VERSION};
use crate::event::Event;
use crate::hooks::BoxFuture;
use crate::logging::write_to_log;

/// Sends events to Sentry.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct SentryExporterConfig {
    /// The project DSN, `https://<key>@<host>/<project id>`.
    pub dsn: String,
    /// Optional `environment` tag.
    pub environment: Option<String>,
    /// Optional `release` tag.
    pub release: Option<String>,
    /// Also send a transaction per event (performance monitoring). Default false.
    pub enable_tracing: bool,
}

impl SentryExporterConfig {
    /// An exporter for `dsn`.
    pub fn new(dsn: impl Into<String>) -> Self {
        Self {
            dsn: dsn.into(),
            environment: None,
            release: None,
            enable_tracing: false,
        }
    }

    /// Sets the `environment` tag.
    pub fn environment(mut self, environment: impl Into<String>) -> Self {
        self.environment = Some(environment.into());
        self
    }

    /// Sets the `release` tag.
    pub fn release(mut self, release: impl Into<String>) -> Self {
        self.release = Some(release.into());
        self
    }

    /// Turns transactions on or off.
    pub fn enable_tracing(mut self, enabled: bool) -> Self {
        self.enable_tracing = enabled;
        self
    }
}

struct ParsedDsn {
    protocol: String,
    public_key: String,
    host: String,
    port: Option<String>,
    path: String,
    project_id: String,
}

/// Parses `protocol://publicKey@host[:port][/path]/projectId`.
fn parse_dsn(dsn: &str) -> Option<ParsedDsn> {
    let (protocol, rest) = dsn.split_once("://")?;
    if protocol != "http" && protocol != "https" {
        return None;
    }
    let (public_key, rest) = rest.split_once('@')?;
    if public_key.is_empty() || !public_key.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return None;
    }
    let (authority, path_and_project) = {
        let i = rest.find('/')?;
        (&rest[..i], &rest[i..])
    };
    let (host, port) = match authority.split_once(':') {
        Some((h, p)) if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) => (h, Some(p.to_owned())),
        Some(_) => return None,
        None => (authority, None),
    };
    if host.is_empty()
        || !host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
    {
        return None;
    }
    let (path, project_id) = path_and_project.rsplit_once('/')?;
    if project_id.is_empty() || !project_id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(ParsedDsn {
        protocol: protocol.to_owned(),
        public_key: public_key.to_owned(),
        host: host.to_owned(),
        port,
        path: path.to_owned(),
        project_id: project_id.to_owned(),
    })
}

pub(crate) struct SentryExporter {
    config: SentryExporterConfig,
    endpoint: String,
    auth_header: String,
}

fn random_hex16() -> String {
    trace_context::span_id("")
}

impl SentryExporter {
    pub(crate) fn new(config: &SentryExporterConfig) -> Result<Self, String> {
        let dsn = parse_dsn(&config.dsn).ok_or_else(|| format!("Invalid Sentry DSN: {}", config.dsn))?;
        let endpoint = format!(
            "{}://{}{}{}/api/{}/envelope/",
            dsn.protocol,
            dsn.host,
            dsn.port.as_ref().map(|p| format!(":{p}")).unwrap_or_default(),
            dsn.path,
            dsn.project_id
        );
        write_to_log(&format!("SentryExporter: Initialized with endpoint {endpoint}"));
        Ok(Self {
            config: config.clone(),
            endpoint,
            auth_header: format!(
                "Sentry sentry_version=7, sentry_client=agentcat-rust/{SDK_VERSION}, sentry_key={}",
                dsn.public_key
            ),
        })
    }

    fn message(event: &Event) -> String {
        let event_type = if event.event_type.is_empty() {
            "event"
        } else {
            &event.event_type
        };
        match event.resource_name.as_deref().filter(|r| !r.is_empty()) {
            Some(r) => format!("MCP {event_type}: {r}"),
            None => format!("MCP {event_type}"),
        }
    }

    fn transaction_name(event: &Event) -> String {
        match event.resource_name.as_deref().filter(|r| !r.is_empty()) {
            Some(r) => format!(
                "{} - {r}",
                if event.event_type.is_empty() {
                    "mcp"
                } else {
                    &event.event_type
                }
            ),
            None if !event.event_type.is_empty() => event.event_type.clone(),
            None => "mcp.event".into(),
        }
    }

    fn op(event: &Event) -> String {
        if event.event_type.is_empty() {
            "mcp.event".into()
        } else {
            event.event_type.clone()
        }
    }

    pub(crate) fn log(event: &Event) -> Value {
        let mut attrs = Map::new();
        let mut put = |key: &str, value: Value, ty: &str| {
            attrs.insert(key.to_owned(), json!({"value": value, "type": ty}));
        };
        let nonempty = |v: &Option<String>| v.clone().filter(|s| !s.is_empty());
        if !event.event_type.is_empty() {
            put("eventType", json!(event.event_type), "string");
        }
        if let Some(v) = nonempty(&event.resource_name) {
            put("resourceName", json!(v), "string");
        }
        if let Some(v) = nonempty(&event.server_name) {
            put("serverName", json!(v), "string");
        }
        if let Some(v) = nonempty(&event.client_name) {
            put("clientName", json!(v), "string");
        }
        if !event.session_id.is_empty() {
            put("sessionId", json!(event.session_id), "string");
        }
        if let Some(v) = nonempty(&event.project_id) {
            put("projectId", json!(v), "string");
        }
        if let Some(d) = event.duration {
            put("duration_ms", json!(d), "double");
        }
        if let Some(v) = nonempty(&event.identify_actor_given_id) {
            put("actorId", json!(v), "string");
        }
        if let Some(v) = nonempty(&event.identify_actor_name) {
            put("actorName", json!(v), "string");
        }
        if let Some(v) = nonempty(&event.user_intent) {
            put("userIntent", json!(v), "string");
        }
        if let Some(v) = nonempty(&event.server_version) {
            put("serverVersion", json!(v), "string");
        }
        if let Some(v) = nonempty(&event.client_version) {
            put("clientVersion", json!(v), "string");
        }
        if let Some(e) = event.is_error {
            put("isError", json!(e), "boolean");
        }
        let span = trace_context::span_id(&event.id);
        json!({
            "timestamp": epoch_seconds(event),
            "trace_id": trace_context::trace_id(&event.session_id),
            "event_id": format!("{span}{span}"),
            "level": if event.is_error == Some(true) { "error" } else { "info" },
            "body": Self::message(event),
            "attributes": attrs,
        })
    }

    fn tags(&self, event: &Event) -> Map<String, Value> {
        let mut tags = Map::new();
        tags.insert("source".into(), json!(AGENTCAT_SOURCE));
        if let Some(env) = &self.config.environment {
            tags.insert("environment".into(), json!(env));
        }
        if let Some(release) = &self.config.release {
            tags.insert("release".into(), json!(release));
        }
        if !event.event_type.is_empty() {
            tags.insert("event_type".into(), json!(event.event_type));
        }
        let mut opt = |k: &str, v: &Option<String>| {
            if let Some(v) = v.as_ref().filter(|s| !s.is_empty()) {
                tags.insert(k.into(), json!(v));
            }
        };
        opt("resource", &event.resource_name);
        opt("server_name", &event.server_name);
        opt("client_name", &event.client_name);
        opt("actor_id", &event.identify_actor_given_id);
        if let Some(customer) = &event.tags {
            for (k, v) in customer {
                tags.insert(format!("agentcat.{k}"), json!(v));
            }
        }
        tags
    }

    fn extra(event: &Event) -> Map<String, Value> {
        let mut extra = Map::new();
        if !event.session_id.is_empty() {
            extra.insert("session_id".into(), json!(event.session_id));
        }
        let mut opt = |k: &str, v: &Option<String>| {
            if let Some(v) = v.as_ref().filter(|s| !s.is_empty()) {
                extra.insert(k.into(), json!(v));
            }
        };
        opt("project_id", &event.project_id);
        opt("user_intent", &event.user_intent);
        opt("actor_name", &event.identify_actor_name);
        opt("server_version", &event.server_version);
        opt("client_version", &event.client_version);
        if let Some(d) = event.duration {
            extra.insert("duration_ms".into(), json!(d));
        }
        if let Some(e) = &event.error {
            extra.insert("error".into(), e.clone());
        }
        extra
    }

    fn contexts(event: &Event, trace: Value) -> Map<String, Value> {
        let mut contexts = Map::new();
        contexts.insert("trace".into(), trace);
        if let Some(props) = &event.properties {
            contexts.insert("agentcat".into(), Value::Object(props.clone()));
        }
        contexts
    }

    /// The transaction spans the call: it starts at the event timestamp (the
    /// request start) and ends one duration later.
    pub(crate) fn transaction(&self, event: &Event) -> Value {
        let start = epoch_seconds(event);
        let end = start + event.duration.unwrap_or(0) as f64 / 1000.0;
        let trace = json!({
            "trace_id": trace_context::trace_id(&event.session_id),
            "span_id": trace_context::span_id(&event.id),
            "op": Self::op(event),
            "status": if event.is_error == Some(true) { "internal_error" } else { "ok" },
        });
        json!({
            "type": "transaction",
            "event_id": format!("{}{}", trace_context::span_id(&event.id), random_hex16()),
            "timestamp": end,
            "start_timestamp": start,
            "transaction": Self::transaction_name(event),
            "contexts": Self::contexts(event, trace),
            "tags": self.tags(event),
            "extra": Self::extra(event),
        })
    }

    pub(crate) fn error_event(&self, event: &Event, transaction: Option<&Value>) -> Value {
        let (mut message, mut error_type) = ("Unknown error".to_owned(), "ToolCallError".to_owned());
        match &event.error {
            Some(Value::String(s)) => message = s.clone(),
            Some(Value::Object(obj)) => {
                message = match obj.get("message") {
                    Some(Value::String(s)) => s.clone(),
                    Some(other) => other.to_string(),
                    None => Value::Object(obj.clone()).to_string(),
                };
                if let Some(t) = obj.get("type") {
                    error_type = match t {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                }
            }
            _ => {}
        }
        let trace_id = transaction
            .and_then(|t| t["contexts"]["trace"]["trace_id"].as_str().map(str::to_owned))
            .unwrap_or_else(|| trace_context::trace_id(&event.session_id));
        let timestamp = transaction
            .and_then(|t| t["timestamp"].as_f64())
            .unwrap_or_else(|| epoch_seconds(event));
        // The error happened inside the call's own span: same span ID, no parent.
        let trace = json!({
            "trace_id": trace_id,
            "span_id": trace_context::span_id(&event.id),
            "op": Self::op(event),
        });
        let mut contexts = Self::contexts(event, trace);
        contexts.insert(
            "mcp".into(),
            json!({
                "resource_name": event.resource_name,
                "session_id": event.session_id,
                "event_type": event.event_type,
                "user_intent": event.user_intent,
            }),
        );
        json!({
            "type": "event",
            "event_id": format!("{}{}", trace_context::span_id(&event.id), random_hex16()),
            "timestamp": timestamp,
            "level": "error",
            "exception": {"values": [{
                "type": error_type,
                "value": message,
                "mechanism": {"type": "mcp_tool_call", "handled": false},
            }]},
            "contexts": contexts,
            "tags": self.tags(event),
            "extra": Self::extra(event),
            "transaction": Self::transaction_name(event),
        })
    }
}

fn envelope(event_id: &str, item_header: Value, payload: &Value, trailing_newline: bool) -> String {
    let header = json!({"event_id": event_id, "sent_at": crate::util::iso_millis(&chrono::Utc::now())});
    let mut out = format!("{header}\n{item_header}\n{payload}");
    if trailing_newline {
        out.push('\n');
    }
    out
}

impl Exporter for SentryExporter {
    fn export(&self, client: reqwest::Client, event: Arc<Event>) -> BoxFuture<Result<(), String>> {
        let log = Self::log(&event);
        let log_envelope = envelope(
            log["event_id"].as_str().unwrap_or_default(),
            json!({"type": "log", "item_count": 1, "content_type": "application/vnd.sentry.items.log+json"}),
            &json!({"items": [log]}),
            true,
        );
        let transaction = self.config.enable_tracing.then(|| self.transaction(&event));
        let transaction_envelope = transaction.as_ref().map(|t| {
            envelope(
                t["event_id"].as_str().unwrap_or_default(),
                json!({"type": "transaction"}),
                t,
                false,
            )
        });
        let error_envelope = (event.is_error == Some(true)).then(|| {
            let e = self.error_event(&event, transaction.as_ref());
            envelope(
                e["event_id"].as_str().unwrap_or_default(),
                json!({"type": "event", "content_type": "application/json"}),
                &e,
                false,
            )
        });
        let endpoint = self.endpoint.clone();
        let auth = self.auth_header.clone();
        Box::pin(async move {
            let send = |body: String, what: &'static str| {
                let request = client
                    .post(&endpoint)
                    .header("X-Sentry-Auth", &auth)
                    .header("Content-Type", "application/x-sentry-envelope")
                    .body(body);
                let id = event.id.clone();
                async move {
                    match request.send().await {
                        Ok(r) if r.status().is_success() => {
                            write_to_log(&format!("Sentry {what} export success - Event: {id}"))
                        }
                        Ok(r) => {
                            let status = r.status().as_u16();
                            let body = r.text().await.unwrap_or_default();
                            write_to_log(&format!(
                                "Sentry {what} export failed - Status: {status}, Body: {body}"
                            ));
                        }
                        Err(e) => write_to_log(&format!("Sentry export error: {e}")),
                    }
                }
            };
            send(log_envelope, "log").await;
            if let Some(body) = transaction_envelope {
                send(body, "transaction").await;
            }
            if let Some(body) = error_envelope {
                send(body, "error").await;
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dsns() {
        let d = parse_dsn("https://abc123@o1.ingest.sentry.io/42").unwrap();
        assert_eq!(
            (d.host.as_str(), d.project_id.as_str(), d.path.as_str()),
            ("o1.ingest.sentry.io", "42", "")
        );
        let d = parse_dsn("http://abc@localhost:9000/sub/path/7").unwrap();
        assert_eq!(d.port.as_deref(), Some("9000"));
        assert_eq!(d.path, "/sub/path");
        assert!(parse_dsn("https://ABC@host/1").is_none());
        assert!(parse_dsn("https://abc@host/x").is_none());
        assert!(parse_dsn("ftp://abc@host/1").is_none());
    }

    #[test]
    fn transaction_starts_at_the_event_timestamp() {
        let exporter = SentryExporter::new(&SentryExporterConfig::new("https://abc@h/1")).unwrap();
        let event = Event {
            timestamp: chrono::DateTime::from_timestamp_millis(1_000_000).unwrap(),
            duration: Some(250),
            ..Event::default()
        };
        let t = exporter.transaction(&event);
        assert_eq!(t["start_timestamp"], json!(1000.0));
        assert_eq!(t["timestamp"], json!(1000.25));
        let e = exporter.error_event(&event, Some(&t));
        assert!(e["contexts"]["trace"].get("parent_span_id").is_none());
    }
}

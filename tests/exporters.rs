//! Telemetry exporters against a mock collector. Exporters are process-wide
//! (first configuration wins), so this binary configures them once.

mod common;

use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use agentcat::exporters::{OtlpExporterConfig, PostHogExporterConfig, SentryExporterConfig};
use common::*;
use serde_json::{Value, json};

#[derive(Clone, Debug)]
struct Received {
    path: String,
    headers: Vec<(String, String)>,
    body: String,
}

static COLLECTOR: OnceLock<(String, Arc<Mutex<Vec<Received>>>)> = OnceLock::new();

fn collector() -> &'static (String, Arc<Mutex<Vec<Received>>>) {
    COLLECTOR.get_or_init(|| {
        let store: Arc<Mutex<Vec<Received>>> = Arc::default();
        let s = store.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async move {
                let app = axum::Router::new().fallback(move |req: axum::extract::Request| {
                    let s = s.clone();
                    async move {
                        let path = req.uri().path().to_owned();
                        let headers = req
                            .headers()
                            .iter()
                            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or_default().to_owned()))
                            .collect();
                        let bytes = axum::body::to_bytes(req.into_body(), usize::MAX).await.unwrap();
                        s.lock().unwrap().push(Received {
                            path,
                            headers,
                            body: String::from_utf8_lossy(&bytes).into_owned(),
                        });
                        "ok"
                    }
                });
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                tx.send(listener.local_addr().unwrap()).unwrap();
                axum::serve(listener, app).await.unwrap();
            });
        });
        let addr = rx.recv().unwrap();
        (format!("http://{addr}"), store)
    })
}

fn received(path: &str) -> Vec<Received> {
    collector()
        .1
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.path == path)
        .cloned()
        .collect()
}

fn agentcat() -> agentcat::AgentCat {
    let url = &collector().0;
    let host = url.trim_start_matches("http://");
    builder("exporters")
        .exporter(
            "otlp",
            OtlpExporterConfig::new(url.clone()).header("x-api-key", "k1"),
        )
        .exporter(
            "posthog",
            PostHogExporterConfig::new("phc_test")
                .host(url.clone())
                .enable_ai_tracing(true),
        )
        .exporter(
            "sentry",
            SentryExporterConfig::new(format!("http://abc123@{host}/42"))
                .environment("test")
                .enable_tracing(true),
        )
        .build()
}

#[tokio::test]
async fn every_exporter_receives_each_event_once() {
    let agentcat = agentcat();
    let client = connect(agentcat.track(Todo::new())).await;
    call(&client, "fail", json!({"context": "Exporter test."}))
        .await
        .unwrap();
    assert!(agentcat::flush(Duration::from_secs(10)).await);

    // AgentCat still gets the event.
    let e = &api().events_for("exporters", 1).await[0];
    let session = e["session_id"].as_str().unwrap().to_owned();

    // OTLP: one span, session-derived trace ID, custom header.
    let otlp = received("/v1/traces");
    assert_eq!(otlp.len(), 1);
    assert!(otlp[0].headers.iter().any(|(k, v)| k == "x-api-key" && v == "k1"));
    let body: Value = serde_json::from_str(&otlp[0].body).unwrap();
    let span = &body["resourceSpans"][0]["scopeSpans"][0]["spans"][0];
    assert_eq!(span["name"], "mcp:tools/call");
    assert_eq!(span["status"]["code"], 2);
    assert_eq!(span["traceId"].as_str().unwrap().len(), 32);
    assert_eq!(
        body["resourceSpans"][0]["resource"]["attributes"][0]["value"]["stringValue"],
        "exporters"
    );

    // PostHog: capture + $exception + $ai_span in one batch.
    let posthog = received("/batch");
    assert_eq!(posthog.len(), 1);
    let batch: Value = serde_json::from_str(&posthog[0].body).unwrap();
    assert_eq!(batch["api_key"], "phc_test");
    let events: Vec<&str> = batch["batch"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["event"].as_str().unwrap())
        .collect();
    assert_eq!(events, ["mcp_tool_call", "$exception", "$ai_span"]);
    assert_eq!(batch["batch"][0]["distinct_id"], session.as_str());
    assert_eq!(batch["batch"][0]["properties"]["tool_name"], "fail");
    assert_eq!(batch["batch"][1]["properties"]["$exception_message"], "disk full");

    // Sentry: log, transaction and error envelopes.
    let sentry = received("/api/42/envelope/");
    assert_eq!(sentry.len(), 3, "{sentry:#?}");
    assert!(sentry.iter().all(|r| {
        r.headers
            .iter()
            .any(|(k, v)| k == "x-sentry-auth" && v.contains("sentry_key=abc123"))
    }));
    let item_types: Vec<String> = sentry
        .iter()
        .map(|r| {
            let line: Value = serde_json::from_str(r.body.lines().nth(1).unwrap()).unwrap();
            line["type"].as_str().unwrap().to_owned()
        })
        .collect();
    assert_eq!(item_types, ["log", "transaction", "event"]);
    let error_event: Value = serde_json::from_str(sentry[2].body.lines().nth(2).unwrap()).unwrap();
    assert_eq!(error_event["exception"]["values"][0]["value"], "disk full");
    assert_eq!(error_event["tags"]["environment"], "test");
}

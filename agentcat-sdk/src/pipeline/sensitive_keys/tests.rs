//! The sensitive key mask: every case of `tests/sensitive_keys/vectors.json`, and the headers of a
//! real streamable HTTP call through `from_call` and `process`.
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use http::{HeaderName, HeaderValue};
use rmcp::model::CallToolRequestParams;
use rmcp::transport::streamable_http_server::StreamableHttpServerConfig;
use serde_json::{Value, json};

use super::{MASK, is_sensitive_key, mask_sensitive_keys};
use crate::client::AgentCat;
use crate::config::Options;
use crate::event::assemble::from_call;
use crate::pipeline::process::{Redactors, process};
use crate::test_support::rmcp::{ContextEcho, RecordingSink, connect_http, serve_http};

/// The shared vectors file, as it is.
fn vectors() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/sensitive_keys/vectors.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// Every key case and every tree case. `Value` equality of maps ignores the key order, and the
/// input is the same after the call.
#[test]
fn vectors_hold() {
    let vectors = vectors();
    assert_eq!(vectors["mask"], MASK);
    let keys = vectors["keys"].as_array().unwrap();
    assert_eq!(keys.len(), 233);
    for case in keys {
        let key = case["key"].as_str().unwrap();
        assert_eq!(
            Value::Bool(is_sensitive_key(key)),
            case["sensitive"],
            "{key:?}"
        );
    }
    let trees = vectors["trees"].as_array().unwrap();
    assert_eq!(trees.len(), 17);
    for case in trees {
        let input = case["input"].clone();
        assert_eq!(
            mask_sensitive_keys(&input, false),
            case["expected"],
            "{}",
            case["name"]
        );
        assert_eq!(input, case["input"], "{}: the input changed", case["name"]);
    }
}

/// A tool call over streamable HTTP with credential headers: the event that `process` gives the
/// sender holds `[redacted]` for `authorization`, `cookie` and `x-api-key`, and the other headers
/// as sent. The handler's own view of the headers, which `ContextEcho` answers, keeps every value.
#[tokio::test]
async fn http_call_headers_are_masked_in_the_event() {
    let sink = Arc::new(RecordingSink::default());
    let agentcat = AgentCat::with_sink("", Options::default(), sink.clone());
    let config = StreamableHttpServerConfig::default().with_json_response(true);
    let url = serve_http(agentcat.track(ContextEcho), config).await;
    let sent = [
        ("authorization", "Bearer abc.def"),
        ("cookie", "sid=1"),
        ("x-api-key", "k-123"),
        ("user-agent", "claude-code/2.1.0"),
        ("x-anthropic-client", "claude-code"),
    ];
    let headers: HashMap<HeaderName, HeaderValue> = sent
        .iter()
        .map(|(name, value)| {
            (
                HeaderName::from_static(name),
                HeaderValue::from_static(value),
            )
        })
        .collect();
    let client = connect_http(&url, "http-client", headers).await;
    let answer = client
        .call_tool(CallToolRequestParams::new("echo"))
        .await
        .unwrap();
    client.cancel().await.unwrap();

    let held = &answer.structured_content.unwrap()["extra"]["requestInfo"]["headers"];
    for (name, value) in sent {
        assert_eq!(held[name], value, "the handler's {name}");
    }
    let mut records = sink.take_records();
    assert_eq!(records.len(), 1);
    let job = from_call(records.remove(0), "proj_test");
    let mut log = Vec::new();
    let event = process(job, &Redactors::default(), Duration::from_secs(1), &mut log)
        .await
        .unwrap();
    assert_eq!(log, Vec::<String>::new());
    let headers = &event["parameters"]["extra"]["requestInfo"]["headers"];
    assert_eq!(headers["authorization"], json!(MASK));
    assert_eq!(headers["cookie"], json!(MASK));
    assert_eq!(headers["x-api-key"], json!(MASK));
    assert_eq!(headers["user-agent"], "claude-code/2.1.0");
    assert_eq!(headers["x-anthropic-client"], "claude-code");
    assert_eq!(headers["mcp-session-id"], held["mcp-session-id"]);
    assert!(
        headers["mcp-session-id"]
            .as_str()
            .is_some_and(|id| !id.is_empty())
    );
}

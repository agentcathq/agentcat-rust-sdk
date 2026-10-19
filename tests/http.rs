//! Streamable HTTP: per-request handlers, request headers, 2026-07-28
//! stateless requests, and legacy sessions.

mod common;

use agentcat::AgentCat;
use common::*;
use rmcp::model::{ClientCapabilities, ClientInfo, Implementation};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use rmcp::ServiceExt;
use serde_json::{Value, json};

async fn spawn(agentcat: AgentCat, stateful: bool) -> String {
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(stateful)
        .with_json_response(!stateful)
        .with_sse_keep_alive(None);
    // The factory runs per session, or per request when stateless.
    let service: StreamableHttpService<agentcat::Tracked<Todo>, LocalSessionManager> =
        StreamableHttpService::new(move || Ok(agentcat.track(Todo::new())), Default::default(), config);
    let router = axum::Router::new().nest_service("/mcp", service);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    format!("http://{addr}/mcp")
}

async fn post_call(url: &str, version: &str, name: &str, args: Value, meta: Option<Value>) -> Value {
    let mut params = json!({"name": name, "arguments": args});
    if let Some(meta) = meta {
        params["_meta"] = meta;
    }
    let body = json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": params});
    let response = reqwest::Client::new()
        .post(url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", version)
        .header("Mcp-Method", "tools/call")
        .header("Mcp-Name", name)
        .header("User-Agent", "claude-code/2.0.0")
        .header("x-agentcat-health-run", "run-1")
        .body(body.to_string())
        .send()
        .await
        .unwrap();
    response.json().await.unwrap()
}

#[tokio::test]
async fn stateless_2026_requests_carry_client_info_protocol_and_headers() {
    let url = spawn(builder("http-2026").build(), false).await;
    let meta = json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientInfo": {"name": "claude-code", "version": "2.0.0"},
        "io.modelcontextprotocol/clientCapabilities": {},
    });
    let body = post_call(&url, "2026-07-28", "add_todo", json!({"text": "a", "context": "Testing HTTP."}), Some(meta)).await;
    let content = &body["result"]["content"];
    assert!(content[0]["text"].as_str().unwrap().starts_with("[session_id issued"), "{body}");
    assert_eq!(content[1]["text"], "Added: a");

    let e = &api().events_for("http-2026", 1).await[0];
    assert_eq!(e["client_name"], "claude-code");
    assert_eq!(e["client_version"], "2.0.0");
    assert_eq!(e["tags"]["agentcat_protocol_version"], "2026-07-28");
    assert_eq!(e["user_intent"], "Testing HTTP.");
    let req = &e["parameters"]["extra"]["http"]["req"];
    assert_eq!(req["method"], "POST");
    // The URI as the handler sees it (axum's nest_service strips "/mcp").
    assert!(req["url"].is_string());
    assert_eq!(req["headers"]["x-agentcat-health-run"], "run-1");
    assert_eq!(req["headers"]["user-agent"], "claude-code/2.0.0");
    assert_eq!(e["parameters"]["extra"]["mcpReq"]["id"], 7);
}

#[tokio::test]
async fn stateless_sessions_continue_across_requests() {
    let url = spawn(builder("http-continue").build(), false).await;
    let meta = json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientInfo": {"name": "c", "version": "1"},
        "io.modelcontextprotocol/clientCapabilities": {},
    });
    let first = post_call(&url, "2026-07-28", "add_todo", json!({"text": "a"}), Some(meta.clone())).await;
    let text = first["result"]["content"][0]["text"].as_str().unwrap();
    let session = text.lines().find_map(|l| l.strip_prefix("session_id: ")).unwrap().to_owned();
    let second = post_call(&url, "2026-07-28", "add_todo", json!({"text": "b", "session_id": session}), Some(meta)).await;
    // A fresh handler per request still strips the echoed handle.
    assert_eq!(second["result"]["content"], json!([{"type": "text", "text": "Added: b"}]));
    let events = api().events_for("http-continue", 2).await;
    assert!(events.iter().all(|e| e["session_id"] == session.as_str()));
}

#[tokio::test]
async fn legacy_stateless_requests_have_no_protocol_tag_or_placeholder_client() {
    let url = spawn(builder("http-legacy").build(), false).await;
    let body = post_call(&url, "2025-06-18", "add_todo", json!({"text": "a"}), None).await;
    assert_eq!(body["result"]["content"][1]["text"], "Added: a", "{body}");
    let e = &api().events_for("http-legacy", 1).await[0];
    assert!(e["tags"].get("agentcat_protocol_version").is_none(), "{e}");
    // rmcp synthesizes a placeholder client for these requests; it is not a client.
    assert!(e.get("client_name").is_none(), "{e}");
}

#[tokio::test]
async fn legacy_sessions_report_the_handshake_client() {
    let url = spawn(builder("http-session").build(), true).await;
    let transport = StreamableHttpClientTransport::from_uri(url);
    let client = ClientInfo::new(ClientCapabilities::default(), Implementation::new("session-client", "3.1.4"))
        .serve(transport)
        .await
        .unwrap();
    let first = call(&client, "add_todo", json!({"text": "a"})).await.unwrap();
    let session = minted_session(&first);
    call(&client, "add_todo", json!({"text": "b", "session_id": session})).await.unwrap();

    let events = api().events_for("http-session", 2).await;
    for e in &events {
        assert_eq!(e["client_name"], "session-client");
        assert_eq!(e["client_version"], "3.1.4");
        assert!(e["parameters"]["extra"]["sessionId"].is_string(), "{e}");
    }
}

use std::{collections::HashMap, sync::atomic::{AtomicU32, Ordering}, sync::Arc};

use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::{
        tool::Extension,
        wrapper::{Json, Parameters},
    },
    model::*,
    schemars, tool, tool_handler, tool_router,
    transport::{
        StreamableHttpClientTransport,
        streamable_http_client::StreamableHttpClientTransportConfig,
        streamable_http_server::{
            StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
        },
    },
};
use rmcp_poc::{AgentCatCall, Recorder, Tracked};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

// ---------------------------------------------------------------- server

#[derive(Debug, Clone, Default)]
pub struct Calc;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AddArgs {
    pub a: i64,
    pub b: i64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StrictArgs {
    pub text: String,
}

#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct AddOut {
    pub sum: i64,
}

#[tool_router]
impl Calc {
    #[tool(description = "Add two integers")]
    async fn add(&self, Parameters(AddArgs { a, b }): Parameters<AddArgs>) -> Json<AddOut> {
        Json(AddOut { sum: a + b })
    }

    #[tool(description = "Echo text; rejects unknown fields")]
    async fn strict_echo(&self, Parameters(StrictArgs { text }): Parameters<StrictArgs>) -> String {
        text
    }

    #[tool(description = "Returns the AgentCat session id the wrapper put in extensions")]
    async fn whoami(&self, Extension(call): Extension<AgentCatCall>) -> String {
        format!("{}|{}", call.session_id, call.intent.unwrap_or_default())
    }

    #[tool(description = "Panics")]
    async fn explode(&self) -> String {
        panic!("kaboom")
    }

    #[tool(description = "Always fails as a tool error")]
    async fn fail(&self) -> Result<String, String> {
        Err("boom".to_owned())
    }
}

#[tool_handler]
impl ServerHandler for Calc {}

fn args(v: Value) -> JsonObject {
    rmcp::model::object(v)
}

fn text_of(c: &ContentBlock) -> String {
    match c {
        ContentBlock::Text(t) => t.text.clone(),
        other => format!("{other:?}"),
    }
}

// ---------------------------------------------------------------- tests

/// Control: without the wrapper, an injected `context` breaks a strict tool.
#[tokio::test]
async fn control_unwrapped_strict_tool_rejects_context() -> anyhow::Result<()> {
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    tokio::spawn(async move {
        let s = Calc.serve(server_io).await.unwrap();
        let _ = s.waiting().await;
    });
    let client = ().serve(client_io).await?;
    let r = client
        .call_tool(
            CallToolRequestParams::new("strict_echo")
                .with_arguments(args(json!({"text": "hi", "context": "why"}))),
        )
        .await?;
    println!("unwrapped strict_echo -> {r:?}");
    assert_eq!(r.is_error, Some(true));
    assert!(text_of(&r.content[0]).contains("unknown field `context`"));
    client.cancel().await?;
    Ok(())
}

#[tokio::test]
async fn duplex_wrapper_end_to_end() -> anyhow::Result<()> {
    let rec = Recorder::default();
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    let server = Tracked::new(Calc, rec.clone(), "ses_duplex");
    tokio::spawn(async move {
        let s = server.serve(server_io).await.unwrap();
        let _ = s.waiting().await;
    });

    let mut client_cfg = ClientConfig::default();
    client_cfg.client_info = Implementation::new("poc-client", "9.9.9");
    let client = client_cfg.serve(client_io).await?;

    // --- tools/list: schema injection + extra tool
    let tools = client.list_all_tools().await?;
    let names: Vec<_> = tools.iter().map(|t| t.name.to_string()).collect();
    println!("tools: {names:?}");
    assert!(names.contains(&"get_more_tools".to_owned()));
    let add = tools.iter().find(|t| t.name == "add").unwrap();
    println!(
        "add.input_schema = {}",
        serde_json::to_string(&add.input_schema)?
    );
    println!(
        "add.output_schema = {}",
        serde_json::to_string(&add.output_schema)?
    );
    assert_eq!(add.input_schema["properties"]["context"]["type"], "string");
    assert!(
        add.input_schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("context"))
    );
    assert!(
        add.output_schema.as_ref().unwrap()["properties"]
            .get("mcp_session")
            .is_some()
    );
    // tools without params get an object schema with only `context`
    let fail = tools.iter().find(|t| t.name == "fail").unwrap();
    assert_eq!(fail.input_schema["required"], json!(["context"]));

    // --- tools/call: strict tool now succeeds because `context` is stripped
    let r = client
        .call_tool(
            CallToolRequestParams::new("strict_echo")
                .with_arguments(args(json!({"text": "hi", "context": "testing strict"}))),
        )
        .await?;
    println!("wrapped strict_echo -> {r:?}");
    assert_ne!(r.is_error, Some(true));
    assert_eq!(text_of(&r.content[1]), "hi");
    assert!(text_of(&r.content[0]).contains("ses_duplex"));

    // --- structured_content gets mcp_session
    let r = client
        .call_tool(
            CallToolRequestParams::new("add")
                .with_arguments(args(json!({"a": 2, "b": 3, "context": "adding"}))),
        )
        .await?;
    let sc = r.structured_content.clone().unwrap();
    println!("add structured_content (wire order) = {}", serde_json::to_string(&sc)?);
    assert_eq!(sc["sum"], 5);
    assert_eq!(sc["mcp_session"]["session_id"], "ses_duplex");

    // --- Extension<AgentCatCall> reaches the #[tool] fn
    let r = client
        .call_tool(
            CallToolRequestParams::new("whoami")
                .with_arguments(args(json!({"context": "who"}))),
        )
        .await?;
    assert_eq!(text_of(&r.content[1]), "ses_duplex|who");

    // --- tool error is observed
    let r = client
        .call_tool(
            CallToolRequestParams::new("fail").with_arguments(args(json!({"context": "x"}))),
        )
        .await?;
    assert_eq!(r.is_error, Some(true));

    // --- SDK-owned tool answered by the wrapper
    let r = client
        .call_tool(
            CallToolRequestParams::new("get_more_tools")
                .with_arguments(args(json!({"context": "need a calendar tool"}))),
        )
        .await?;
    assert!(text_of(&r.content[1]).contains("full tool list"));

    // --- unknown tool: protocol error passes through, still recorded
    let err = client
        .call_tool(CallToolRequestParams::new("nope").with_arguments(args(json!({}))))
        .await;
    println!("unknown tool -> {err:?}");
    assert!(err.is_err());

    client.cancel().await?;

    let events = rec.events();
    for e in &events {
        println!("{e:?}");
    }
    let init = &events[0];
    assert_eq!(init.kind, "initialize");
    assert_eq!(init.client.as_deref(), Some("poc-client@9.9.9"));
    let strict = events.iter().find(|e| e.name.as_deref() == Some("strict_echo")).unwrap();
    assert_eq!(strict.intent.as_deref(), Some("testing strict"));
    assert_eq!(strict.client.as_deref(), Some("poc-client@9.9.9"));
    assert!(strict.protocol_version.is_some());
    assert!(strict.raw_arguments.as_ref().unwrap().contains_key("context"));
    let fail = events.iter().find(|e| e.name.as_deref() == Some("fail")).unwrap();
    assert!(fail.is_error);
    let gmt = events.iter().find(|e| e.name.as_deref() == Some("get_more_tools")).unwrap();
    assert_eq!(gmt.intent.as_deref(), Some("need a calendar tool"));
    let nope = events.iter().find(|e| e.name.as_deref() == Some("nope")).unwrap();
    assert!(nope.is_error);
    Ok(())
}

async fn spawn_http(
    rec: Recorder,
    config: StreamableHttpServerConfig,
) -> anyhow::Result<(String, Arc<AtomicU32>)> {
    let instances = Arc::new(AtomicU32::new(0));
    let counter = instances.clone();
    let service: StreamableHttpService<Tracked<Calc>, LocalSessionManager> =
        StreamableHttpService::new(
            move || {
                let n = counter.fetch_add(1, Ordering::SeqCst);
                Ok(Tracked::new(Calc, rec.clone(), format!("ses_http_{n}")))
            },
            Default::default(),
            config,
        );
    let router = axum::Router::new().nest_service("/mcp", service);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    Ok((format!("http://{addr}/mcp"), instances))
}

async fn http_client(url: &str) -> anyhow::Result<rmcp::service::RunningService<rmcp::RoleClient, ClientConfig>> {
    let mut headers = HashMap::new();
    headers.insert(
        http::HeaderName::from_static("x-actor"),
        http::HeaderValue::from_static("alice"),
    );
    let transport = StreamableHttpClientTransport::from_config(
        StreamableHttpClientTransportConfig::with_uri(url.to_owned()).custom_headers(headers),
    );
    let mut cfg = ClientConfig::default();
    cfg.client_info = Implementation::new("http-client", "1.2.3");
    Ok(cfg.serve(transport).await?)
}

#[tokio::test]
async fn http_legacy_stateful_session() -> anyhow::Result<()> {
    let rec = Recorder::default();
    let (url, instances) = spawn_http(
        rec.clone(),
        StreamableHttpServerConfig::default().with_legacy_session_mode(true),
    )
    .await?;
    let client = http_client(&url).await?;
    let tools = client.list_all_tools().await?;
    assert!(tools.iter().any(|t| t.name == "get_more_tools"));
    for i in 0..2 {
        let r = client
            .call_tool(
                CallToolRequestParams::new("add")
                    .with_arguments(args(json!({"a": i, "b": 1, "context": "http add"}))),
            )
            .await?;
        assert_eq!(r.structured_content.unwrap()["mcp_session"]["session_id"], "ses_http_0");
    }
    client.cancel().await?;
    let events = rec.events();
    for e in &events {
        println!("stateful: {e:?}");
    }
    let calls: Vec<_> = events.iter().filter(|e| e.kind == "tools/call").collect();
    assert_eq!(calls.len(), 2);
    assert!(calls.iter().all(|e| e.http_header.as_deref() == Some("alice")));
    assert!(calls.iter().all(|e| e.mcp_session_id_header.is_some()));
    assert!(calls.iter().all(|e| e.client.as_deref() == Some("http-client@1.2.3")));
    println!("handler instances created: {}", instances.load(Ordering::SeqCst));
    Ok(())
}

#[tokio::test]
async fn http_stateless() -> anyhow::Result<()> {
    let rec = Recorder::default();
    let (url, instances) = spawn_http(
        rec.clone(),
        StreamableHttpServerConfig::default()
            .with_legacy_session_mode(false)
            .with_json_response(true),
    )
    .await?;
    let client = http_client(&url).await?;
    let _ = client.list_all_tools().await?;
    for i in 0..2 {
        let r = client
            .call_tool(
                CallToolRequestParams::new("add")
                    .with_arguments(args(json!({"a": i, "b": 1, "context": "stateless add"}))),
            )
            .await?;
        println!("stateless result sc = {:?}", r.structured_content);
    }
    client.cancel().await?;
    let events = rec.events();
    for e in &events {
        println!("stateless: {e:?}");
    }
    let calls: Vec<_> = events.iter().filter(|e| e.kind == "tools/call").collect();
    assert_eq!(calls.len(), 2);
    assert!(calls.iter().all(|e| e.http_header.as_deref() == Some("alice")));
    assert!(calls.iter().all(|e| e.mcp_session_id_header.is_none()));
    println!(
        "stateless handler instances created: {} (one per HTTP request)",
        instances.load(Ordering::SeqCst)
    );
    println!(
        "client info seen in stateless calls: {:?}",
        calls.iter().map(|e| e.client.clone()).collect::<Vec<_>>()
    );
    Ok(())
}

/// What does a panicking tool do to the wire? (decides whether the SDK needs catch_unwind)
#[tokio::test]
async fn panic_in_tool() -> anyhow::Result<()> {
    let rec = Recorder::default();
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    let server = Tracked::new(Calc, rec.clone(), "ses_panic");
    tokio::spawn(async move {
        let s = server.serve(server_io).await.unwrap();
        let _ = s.waiting().await;
    });
    let client = ().serve(client_io).await?;
    let r = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        client.call_tool(CallToolRequestParams::new("explode").with_arguments(args(json!({"context": "boom"})))),
    )
    .await;
    println!("panic call outcome: {r:?}");
    // server still alive?
    let r2 = client
        .call_tool(CallToolRequestParams::new("add").with_arguments(args(json!({"a": 1, "b": 1}))))
        .await;
    println!("call after panic: ok={}", r2.is_ok());
    println!("events after panic: {:?}", rec.events().iter().map(|e| (e.kind, e.name.clone())).collect::<Vec<_>>());
    client.cancel().await?;
    Ok(())
}

/// 2026-07-28 clients skip `initialize`: identity arrives per request in `_meta`.
#[tokio::test]
async fn discover_lifecycle_client() -> anyhow::Result<()> {
    use rmcp::ClientServiceExt;
    let rec = Recorder::default();
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    let server = Tracked::new(Calc, rec.clone(), "ses_discover");
    tokio::spawn(async move {
        let s = server.serve(server_io).await.unwrap();
        let _ = s.waiting().await;
    });
    let mut cfg = ClientConfig::default();
    cfg.client_info = Implementation::new("modern-client", "2.0.0");
    let client = cfg
        .serve_with_lifecycle(
            client_io,
            rmcp::ClientLifecycleMode::Discover {
                preferred_versions: vec![ProtocolVersion::V_2026_07_28],
            },
        )
        .await?;
    let tools = client.list_all_tools().await?;
    assert!(tools.iter().any(|t| t.name == "get_more_tools"));
    let r = client
        .call_tool(CallToolRequestParams::new("add").with_arguments(args(json!({"a": 1, "b": 2, "context": "modern"}))))
        .await?;
    println!("discover-mode result: {:?}", r.structured_content);
    client.cancel().await?;
    for e in rec.events() {
        println!("discover: kind={} name={:?} client={:?} proto={:?}", e.kind, e.name, e.client, e.protocol_version);
    }
    assert!(rec.events().iter().all(|e| e.kind != "initialize"));
    Ok(())
}

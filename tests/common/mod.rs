//! Shared test harness: a mock AgentCat API, a test MCP server, and an
//! in-process rmcp client.
//!
//! Each integration test binary is its own process, so it gets its own
//! publisher and mock API. Tests in one binary run in parallel; each test
//! uses its own server name and filters events by it.
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use agentcat::{AgentCat, AgentCatBuilder};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::*;
use rmcp::service::RunningService;
use rmcp::{ErrorData as McpError, RoleClient, ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROJECT_ID: &str = "proj_2cOHEO0LYGADMzRvWTXXVbbgxgm";

/// The mock AgentCat API.
pub struct MockApi {
    pub url: String,
    requests: Arc<Mutex<Vec<Value>>>,
    /// resource_name → (requests left to fail, status).
    failures: Arc<Mutex<HashMap<String, (usize, u16)>>>,
}

static API: OnceLock<MockApi> = OnceLock::new();

/// Starts (once per test binary) a mock API on a dedicated thread, so it
/// outlives every test's runtime.
pub fn api() -> &'static MockApi {
    API.get_or_init(|| {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let failures: Arc<Mutex<HashMap<String, (usize, u16)>>> = Arc::default();
        let (tx, rx) = std::sync::mpsc::channel();
        let (r, f) = (requests.clone(), failures.clone());
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async move {
                let app = axum::Router::new().route(
                    "/v1/public/events/",
                    axum::routing::post(move |axum::Json(body): axum::Json<Value>| {
                        let (r, f) = (r.clone(), f.clone());
                        async move {
                            let name = body["resource_name"].as_str().unwrap_or_default().to_owned();
                            let failing = {
                                let mut failures = f.lock().unwrap();
                                match failures.get_mut(&name) {
                                    Some((left, status)) if *left > 0 => {
                                        *left -= 1;
                                        Some(*status)
                                    }
                                    _ => None,
                                }
                            };
                            r.lock().unwrap().push(body);
                            if let Some(status) = failing {
                                let status = axum::http::StatusCode::from_u16(status).unwrap();
                                (status, axum::Json(serde_json::json!({"error": "mock failure"})))
                            } else {
                                (
                                    axum::http::StatusCode::ACCEPTED,
                                    axum::Json(serde_json::json!({"status": "accepted"})),
                                )
                            }
                        }
                    }),
                );
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                tx.send(listener.local_addr().unwrap()).unwrap();
                axum::serve(listener, app).await.unwrap();
            });
        });
        let addr = rx.recv().unwrap();
        MockApi {
            url: format!("http://{addr}"),
            requests,
            failures,
        }
    })
}

impl MockApi {
    /// Every request body received so far that matches `filter`.
    pub fn requests_where(&self, filter: impl Fn(&Value) -> bool) -> Vec<Value> {
        self.requests.lock().unwrap().iter().filter(|v| filter(v)).cloned().collect()
    }

    /// Answers the next `n` requests for events named `resource_name` with
    /// `status`.
    pub fn fail_next(&self, resource_name: &str, n: usize, status: u16) {
        self.failures.lock().unwrap().insert(resource_name.to_owned(), (n, status));
    }

    /// Waits until at least `n` requests match, or panics after 10 s.
    pub async fn wait_for(&self, n: usize, filter: impl Fn(&Value) -> bool) -> Vec<Value> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let found = self.requests_where(&filter);
            if found.len() >= n {
                return found;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {n} events; got {}: {found:#?}",
                found.len()
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// The events of one test server, waiting for `n`.
    pub async fn events_for(&self, server_name: &str, n: usize) -> Vec<Value> {
        let name = server_name.to_owned();
        self.wait_for(n, move |v| v["server_name"] == name.as_str()).await
    }

    /// Asserts that no further event for `server_name` arrives within a
    /// short grace period after `n` events.
    pub async fn assert_count_settles(&self, server_name: &str, n: usize) {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let name = server_name.to_owned();
        let found = self.requests_where(move |v| v["server_name"] == name.as_str());
        assert_eq!(found.len(), n, "{found:#?}");
    }
}

/// A builder wired to the mock API, with diagnostics and the log file off.
pub fn builder(server_name: &str) -> AgentCatBuilder {
    AgentCat::builder(PROJECT_ID)
        .api_base_url(api().url.clone())
        .disable_diagnostics(true)
        .log_file(false)
        .server_info(server_name, "1.0.0")
}

// ── The test server ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddArgs {
    /// The todo text.
    pub text: String,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Count {
    pub count: usize,
    pub note: String,
}

#[derive(Clone)]
pub struct Todo {
    items: Arc<Mutex<Vec<String>>>,
    tool_router: ToolRouter<Todo>,
}

impl Default for Todo {
    fn default() -> Self {
        Self::new()
    }
}

#[tool_router]
impl Todo {
    pub fn new() -> Self {
        Self {
            items: Arc::default(),
            tool_router: Self::tool_router(),
        }
    }

    /// Strict arguments: an unstripped AgentCat parameter would fail here.
    #[tool(description = "Add a todo item")]
    async fn add_todo(&self, Parameters(AddArgs { text }): Parameters<AddArgs>) -> Result<CallToolResult, McpError> {
        self.items.lock().unwrap().push(text.clone());
        Ok(CallToolResult::success(vec![ContentBlock::text(format!("Added: {text}"))]))
    }

    #[tool(description = "Count todo items")]
    async fn count_todos(&self) -> Result<Json<Count>, McpError> {
        let count = self.items.lock().unwrap().len();
        Ok(Json(Count {
            count,
            note: "x".repeat(10),
        }))
    }

    #[tool(description = "Echo the arguments it receives")]
    async fn echo(&self, Parameters(args): Parameters<JsonObject>) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::success(vec![ContentBlock::text(Value::Object(args).to_string())]))
    }

    #[tool(description = "Always fails with a tool error")]
    async fn fail(&self) -> Result<CallToolResult, McpError> {
        Ok(CallToolResult::error(vec![ContentBlock::text("disk full")]))
    }

    #[tool(description = "Always fails with a protocol error")]
    async fn reject(&self) -> Result<CallToolResult, McpError> {
        Err(McpError::invalid_params("bad input", None))
    }

    #[tool(description = "Panics")]
    async fn explode(&self) -> Result<CallToolResult, McpError> {
        panic!("kaboom")
    }
}

#[tool_handler]
impl ServerHandler for Todo {
    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("todo-server", "0.1.0"))
    }
}

pub type Client = RunningService<RoleClient, ClientInfo>;

/// Serves `server` over an in-memory duplex and connects a client named
/// `test-client` 9.9.9.
pub async fn connect<S: ServerHandler>(server: S) -> Client {
    let (server_io, client_io) = tokio::io::duplex(1 << 20);
    tokio::spawn(async move {
        if let Ok(running) = server.serve(server_io).await {
            let _ = running.waiting().await;
        }
    });
    ClientInfo::new(ClientCapabilities::default(), Implementation::new("test-client", "9.9.9"))
        .serve(client_io)
        .await
        .expect("client connects")
}

/// Calls a tool with JSON arguments.
pub async fn call(client: &Client, name: &str, args: Value) -> Result<CallToolResult, rmcp::ServiceError> {
    let mut params = CallToolRequestParams::new(name.to_owned());
    if let Value::Object(map) = args {
        params = params.with_arguments(map);
    }
    client.call_tool(params).await
}

/// The text of a result's content blocks.
pub fn texts(result: &CallToolResult) -> Vec<String> {
    let v = serde_json::to_value(result).unwrap();
    v["content"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["text"].as_str().unwrap_or_default().to_owned())
        .collect()
}

/// The `ses_` ID in a mint-back block.
pub fn minted_session(result: &CallToolResult) -> String {
    let first = texts(result).into_iter().next().unwrap();
    first
        .lines()
        .find_map(|l| l.strip_prefix("session_id: "))
        .unwrap_or_else(|| panic!("no session line in {first}"))
        .to_owned()
}

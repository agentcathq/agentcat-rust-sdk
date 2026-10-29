//! The tracked server of the suite: the tools of `tools.json` behind rmcp's `ToolRouter` (see
//! `tools`), wrapped with [`AgentCat::track`], its options built from the scenario's options
//! profile (see `profile`), and served over one transport (this module).
mod profile;
mod tools;

use std::net::Ipv4Addr;
use std::sync::Arc;

use agentcat_sdk::{AgentCat, Tracked};
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::post_service;
use axum::serve::ListenerExt;
use rmcp::ServiceExt;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use tokio::io::DuplexStream;
use tokio::task::JoinHandle;

use super::Transport;
use super::scenario::Scenario;
pub(crate) use profile::options;
use tools::{ToolServer, Tools};

/// The capacity of the in-memory pipe of a stdio connection, in bytes per direction.
const PIPE_BYTES: usize = 64 * 1024;

/// The path of the streamable HTTP endpoint.
const MCP_PATH: &str = "/mcp";

/// The JSON-RPC error that the stateless endpoint answers to every method but POST.
const METHOD_NOT_ALLOWED: &str =
    r#"{"jsonrpc":"2.0","error":{"code":-32000,"message":"Method not allowed."},"id":null}"#;

/// Builds the tracked server of each connection, session or request.
type Factory = Arc<dyn Fn() -> Tracked<ToolServer> + Send + Sync>;

/// The tracked server, served over one transport.
pub(crate) enum Served {
    /// Each connection is an in-memory pipe to a fresh tracked server.
    Stdio(Factory),
    /// rmcp's streamable HTTP service at `url`, on a spawned task.
    Http { url: String, task: JoinHandle<()> },
}

/// Serves the tools of `scenario` (its page size), tracked with `agentcat`, over `transport`:
/// stdio over in-memory pipes; HTTP with rmcp's `StreamableHttpService` on `127.0.0.1` with a
/// free port, with MCP sessions (`legacy_session_mode`) except `HttpStateless`, and JSON
/// responses (`json_response`) except `HttpSse`. The stateless endpoint takes POST only: every
/// other method gets 405 with [`METHOD_NOT_ALLOWED`].
pub(crate) async fn serve(
    transport: Transport,
    agentcat: &AgentCat,
    scenario: &Scenario,
) -> Served {
    let server = ToolServer::new(
        Tools::new(scenario.page_size),
        agentcat.clone(),
        scenario.project_id.clone(),
    );
    let handle = agentcat.clone();
    let factory: Factory = Arc::new(move || handle.track(server.clone()));
    let (stateless, sse) = match transport {
        Transport::Stdio => return Served::Stdio(factory),
        Transport::Http => (false, false),
        Transport::HttpSse => (false, true),
        Transport::HttpStateless => (true, false),
    };
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(!stateless)
        .with_json_response(!sse);
    let service = StreamableHttpService::new(
        move || Ok(factory()),
        Arc::new(LocalSessionManager::default()),
        config,
    );
    let router = if stateless {
        let post = post_service(service).fallback(|| async { method_not_allowed() });
        axum::Router::new().route(MCP_PATH, post)
    } else {
        axum::Router::new().route_service(MCP_PATH, service)
    };
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .expect("a loopback port is free");
    let address = listener.local_addr().expect("the listener has an address");
    // In an MCP session, rmcp answers each request with an SSE stream that it writes in parts.
    // With Nagle's algorithm, the last small part waits for the client's delayed ACK (about 40 ms
    // on Linux) on every request. TCP_NODELAY sends each part at once.
    let listener = listener.tap_io(|stream| {
        stream.set_nodelay(true).expect("TCP_NODELAY can be set");
    });
    let task = tokio::spawn(async move {
        axum::serve(listener, router)
            .await
            .expect("the HTTP server runs");
    });
    Served::Http {
        url: format!("http://{address}{MCP_PATH}"),
        task,
    }
}

impl Served {
    /// A new stdio connection: the client's end of an in-memory pipe whose other end a fresh
    /// tracked server serves, and the task of that server, which ends when the client closes its
    /// end.
    ///
    /// # Panics
    ///
    /// When the server is served over HTTP. The task panics when the server stops with an error.
    pub(crate) fn connect_stdio(&self) -> (DuplexStream, JoinHandle<()>) {
        let Self::Stdio(factory) = self else {
            panic!("connect_stdio on an HTTP server")
        };
        let server = factory();
        let (server_end, client_end) = tokio::io::duplex(PIPE_BYTES);
        let task = tokio::spawn(async move {
            let running = server
                .serve(server_end)
                .await
                .expect("the stdio server starts");
            running
                .waiting()
                .await
                .expect("the stdio server stops cleanly");
        });
        (client_end, task)
    }

    /// Stops the HTTP server.
    pub(crate) fn stop(&self) {
        if let Self::Http { task, .. } = self {
            task.abort();
        }
    }
}

/// The stateless answer to GET, DELETE and every other method but POST.
fn method_not_allowed() -> impl IntoResponse {
    (
        StatusCode::METHOD_NOT_ALLOWED,
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::ALLOW, "POST"),
        ],
        METHOD_NOT_ALLOWED,
    )
}

#[tokio::test]
async fn the_stateless_endpoint_takes_post_only() {
    let mock = super::mock_api::MockApi::start().await;
    let options = agentcat_sdk::Options::default()
        .api_base_url(mock.url())
        .log_destination(agentcat_sdk::LogDestination::Tracing);
    let agentcat = AgentCat::new("proj_E2E", options);
    let scenario = Scenario::load("basic");
    let served = serve(Transport::HttpStateless, &agentcat, &scenario).await;
    let Served::Http { url, .. } = &served else {
        panic!("an HTTP transport serves over HTTP")
    };
    let http = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("the HTTP client builds");
    for method in [reqwest::Method::GET, reqwest::Method::DELETE] {
        let response = http
            .request(method.clone(), url)
            .send()
            .await
            .expect("the request is sent");
        assert_eq!(
            response.status(),
            StatusCode::METHOD_NOT_ALLOWED,
            "{method}"
        );
        let allow = response.headers().get(header::ALLOW).cloned();
        assert_eq!(allow.as_ref().and_then(|v| v.to_str().ok()), Some("POST"));
        let body = response.text().await.expect("the body reads");
        assert_eq!(body, METHOD_NOT_ALLOWED, "{method}");
    }
    served.stop();
}

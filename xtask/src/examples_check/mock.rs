//! The mock of the `AgentCat` API that every example run publishes to.
use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{Method, StatusCode, Uri, header};
use serde_json::Value;

/// The path of a publish request.
pub(super) const PUBLISH_PATH: &str = "/v1/public/events/";

/// A request the mock received.
#[derive(Debug, Clone)]
pub(super) struct Recorded {
    pub(super) method: Method,
    pub(super) path: String,
    /// When the mock had the whole body.
    pub(super) received: Instant,
    /// The body parsed as JSON; the text itself when it is not JSON; `null` when it is empty.
    pub(super) body: Value,
    /// The body as text.
    pub(super) raw: String,
}

impl Recorded {
    pub(super) fn is_publish(&self) -> bool {
        self.method == Method::POST && self.path == PUBLISH_PATH
    }
}

/// The requests the mock received, in order.
type Requests = Arc<Mutex<Vec<Recorded>>>;

/// The mock of the `AgentCat` API: on 127.0.0.1, it answers a publish (`POST /v1/public/events/`)
/// with `202 {"status":"accepted"}` and any other request with 404, and records every request.
#[derive(Debug)]
pub(super) struct MockApi {
    /// The base URL, `http://127.0.0.1:<port>`.
    pub(super) url: String,
    requests: Requests,
    server: tokio::task::JoinHandle<()>,
}

impl MockApi {
    pub(super) async fn start() -> Result<Self, String> {
        let requests = Requests::default();
        let router = axum::Router::new()
            .fallback(record)
            .with_state(Arc::clone(&requests));
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|e| format!("the mock API cannot listen: {e}"))?;
        let address = listener
            .local_addr()
            .map_err(|e| format!("the mock API has no address: {e}"))?;
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        Ok(Self {
            url: format!("http://{address}"),
            requests,
            server,
        })
    }

    pub(super) fn requests(&self) -> Vec<Recorded> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Drop for MockApi {
    fn drop(&mut self) {
        self.server.abort();
    }
}

/// Records a request and answers it.
async fn record(
    State(requests): State<Requests>,
    method: Method,
    uri: Uri,
    body: Bytes,
) -> (
    StatusCode,
    [(header::HeaderName, &'static str); 1],
    &'static str,
) {
    let received = Instant::now();
    let raw = String::from_utf8_lossy(&body).into_owned();
    let body = if raw.is_empty() {
        Value::Null
    } else {
        serde_json::from_str(&raw).unwrap_or_else(|_| Value::String(raw.clone()))
    };
    let request = Recorded {
        method,
        path: uri.path().to_owned(),
        received,
        body,
        raw,
    };
    let publish = request.is_publish();
    requests
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(request);
    let json = [(header::CONTENT_TYPE, "application/json")];
    if publish {
        (StatusCode::ACCEPTED, json, r#"{"status":"accepted"}"#)
    } else {
        (StatusCode::NOT_FOUND, json, r#"{"error":"mock"}"#)
    }
}

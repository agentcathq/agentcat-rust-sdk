//! The client of a streamable HTTP connection: one POST per JSON-RPC message with reqwest, a JSON
//! or SSE answer, and `DELETE` at the close of an MCP session.
use reqwest::StatusCode;
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use serde_json::Value;

use super::super::Lane;

/// What every POST accepts.
const ACCEPTED_TYPES: &str = "application/json, text/event-stream";

/// The client of a streamable HTTP connection.
pub(super) struct Web {
    http: reqwest::Client,
    url: String,
    /// The scenario's headers.
    headers: Vec<(String, String)>,
    /// Whether requests carry `mcp-method` and `mcp-name` (2026-07-28).
    named: bool,
    /// The `mcp-protocol-version` of later requests, once known.
    pub(super) version: Option<&'static str>,
    /// The `mcp-session-id` the server issued.
    pub(super) session: Option<String>,
}

impl Web {
    /// A client of the endpoint `url` that sends `headers` with every request, on `lane`.
    pub(super) fn new(url: &str, headers: &[(String, String)], lane: Lane) -> Self {
        let http = reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("the HTTP client builds");
        Self {
            http,
            url: url.to_owned(),
            headers: headers.to_vec(),
            named: lane == Lane::V2026_07_28,
            version: None,
            session: None,
        }
    }

    /// A request with `method` and every header but the content headers.
    fn request(&self, method: reqwest::Method) -> reqwest::RequestBuilder {
        let mut request = self.http.request(method, &self.url);
        for (name, value) in &self.headers {
            request = request.header(name, value);
        }
        if let Some(version) = self.version {
            request = request.header("mcp-protocol-version", version);
        }
        if let Some(session) = &self.session {
            request = request.header("mcp-session-id", session);
        }
        request
    }

    /// POSTs `message` (the JSON-RPC `method`, for the tool `tool`); returns the response (none
    /// for 202 Accepted) and the `mcp-session-id` of the answer.
    ///
    /// # Panics
    ///
    /// When the POST fails, the status is not a success, or the body is not a JSON-RPC message.
    pub(super) async fn post(
        &self,
        message: &Value,
        method: &str,
        tool: Option<&str>,
    ) -> (Option<Value>, Option<String>) {
        let mut request = self
            .request(reqwest::Method::POST)
            .header(ACCEPT, ACCEPTED_TYPES)
            .header(CONTENT_TYPE, "application/json");
        if self.named {
            request = request.header("mcp-method", method);
            if let Some(tool) = tool {
                request = request.header("mcp-name", tool);
            }
        }
        let body = serde_json::to_vec(message).expect("a message serializes");
        let response = request
            .body(body)
            .send()
            .await
            .unwrap_or_else(|error| panic!("{method}: the POST failed: {error}"));
        let session = response
            .headers()
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let status = response.status();
        if status == StatusCode::ACCEPTED {
            return (None, session);
        }
        let event_stream = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("text/event-stream"));
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            panic!("{method}: HTTP {status}: {text}");
        }
        let reply = if event_stream {
            read_event_stream(response, &message["id"]).await
        } else {
            let body = response.bytes().await.expect("the response body reads");
            serde_json::from_slice(&body).expect("the response body is JSON")
        };
        (Some(reply), session)
    }

    /// Ends the MCP session with `DELETE`, when the server issued one.
    ///
    /// # Panics
    ///
    /// When the `DELETE` fails or its status is not a success.
    pub(super) async fn close(self) {
        if self.session.is_none() {
            return;
        }
        let response = self
            .request(reqwest::Method::DELETE)
            .send()
            .await
            .expect("the DELETE is sent");
        assert!(
            response.status().is_success(),
            "DELETE: HTTP {}",
            response.status()
        );
    }
}

/// Reads the SSE stream of `response` until the JSON-RPC response with `id`; other messages and
/// events without data are skipped.
///
/// # Panics
///
/// When the stream ends first, or an event's data is not JSON.
async fn read_event_stream(mut response: reqwest::Response, id: &Value) -> Value {
    let mut buffer: Vec<u8> = Vec::new();
    loop {
        while let Some(end) = buffer.windows(2).position(|pair| pair == b"\n\n") {
            let event: Vec<u8> = buffer.drain(..end + 2).collect();
            let event = String::from_utf8(event).expect("an SSE event is UTF-8");
            let data: Vec<&str> = event
                .lines()
                .filter_map(|line| line.strip_prefix("data:"))
                .map(|data| data.strip_prefix(' ').unwrap_or(data))
                .collect();
            let data = data.join("\n");
            if data.trim().is_empty() {
                continue;
            }
            let message: Value = serde_json::from_str(&data)
                .unwrap_or_else(|error| panic!("an SSE event is not JSON ({error}): {data}"));
            if message.get("id") == Some(id) {
                return message;
            }
        }
        let chunk = response.chunk().await.expect("the SSE stream reads");
        let Some(chunk) = chunk else {
            panic!("the SSE stream ended without the answer to {id}")
        };
        buffer.extend(chunk.iter().filter(|byte| **byte != b'\r'));
    }
}

//! The JSON-RPC client of the examples check: one message per line on a stdio example's stdin
//! and stdout, or one POST per message to an HTTP example with a JSON or server-sent-event answer.
use axum::http::header;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

/// The MCP protocol version the client asks for: the newest with `initialize` on every rmcp 3.x
/// minor.
const PROTOCOL_VERSION: &str = "2025-11-25";

/// The `Accept` header of every POST to an HTTP example.
const ACCEPTED_TYPES: &str = "application/json, text/event-stream";

/// A JSON-RPC client of the example. Dropping it closes the connection: the stdin of a stdio
/// example closes; an HTTP example sees no further request.
pub(super) struct Client {
    connection: Connection,
    /// The ID of the next request.
    next_id: u64,
}

/// How the client reaches the example.
pub(super) enum Connection {
    /// One JSON-RPC message per line on the example's stdin; the answers come from its stdout.
    Stdio {
        stdin: ChildStdin,
        messages: UnboundedReceiver<Value>,
    },
    /// Streamable HTTP: one POST per message.
    Http {
        http: reqwest::Client,
        url: String,
        /// The `mcp-session-id` the example issued, if any.
        session: Option<String>,
        /// The `mcp-protocol-version` of the requests after `initialize`.
        version: Option<String>,
    },
}

impl Client {
    /// Sends `initialize` and `notifications/initialized`.
    pub(super) async fn connect(connection: Connection) -> Result<Self, String> {
        let mut client = Self {
            connection,
            next_id: 1,
        };
        let params = json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": {"name": "examples-check", "version": "1.0.0"},
        });
        let result = client.request("initialize", params).await?;
        if let Connection::Http { version, .. } = &mut client.connection {
            *version = Some(
                result["protocolVersion"]
                    .as_str()
                    .unwrap_or(PROTOCOL_VERSION)
                    .to_owned(),
            );
        }
        let initialized = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        client
            .send(&initialized, "notifications/initialized")
            .await?;
        Ok(client)
    }

    /// Sends the request `method` with `params`; returns its `result`.
    ///
    /// # Errors
    ///
    /// The request cannot be sent, the answer does not come, or it is a JSON-RPC error.
    pub(super) async fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let message = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let reply = self
            .send(&message, method)
            .await?
            .ok_or_else(|| format!("{method}: the example sent no answer"))?;
        if let Some(error) = reply.get("error") {
            return Err(format!("{method} failed: {error}"));
        }
        reply
            .get("result")
            .cloned()
            .ok_or_else(|| format!("{method}: the answer has no result: {reply}"))
    }

    /// Sends `message`; returns the answer with its ID, or `None` for a notification.
    async fn send(&mut self, message: &Value, method: &str) -> Result<Option<Value>, String> {
        let id = message.get("id").cloned();
        match &mut self.connection {
            Connection::Stdio { stdin, messages } => {
                let line = format!("{message}\n");
                stdin
                    .write_all(line.as_bytes())
                    .await
                    .map_err(|e| format!("{method}: cannot write to the example's stdin: {e}"))?;
                let Some(id) = id else {
                    return Ok(None);
                };
                while let Some(reply) = messages.recv().await {
                    if reply.get("id") == Some(&id) && reply.get("method").is_none() {
                        return Ok(Some(reply));
                    }
                }
                Err(format!("{method}: stdout closed before the answer"))
            }
            Connection::Http {
                http,
                url,
                session,
                version,
            } => {
                let mut request = http
                    .post(url.as_str())
                    .header(header::ACCEPT, ACCEPTED_TYPES)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(message.to_string());
                if let Some(session) = session.as_deref() {
                    request = request.header("mcp-session-id", session);
                }
                if let Some(version) = version.as_deref() {
                    request = request.header("mcp-protocol-version", version);
                }
                let response = request
                    .send()
                    .await
                    .map_err(|e| format!("{method}: the POST failed: {e}"))?;
                if let Some(issued) = response
                    .headers()
                    .get("mcp-session-id")
                    .and_then(|value| value.to_str().ok())
                {
                    *session = Some(issued.to_owned());
                }
                http_answer(response, method, id.as_ref()).await
            }
        }
    }
}

/// The answer in the HTTP `response` to the message `method` with the ID `id` (`None` for a
/// notification, which gets 202 and no answer): a JSON body, or the event with that ID in a
/// server-sent-event stream.
async fn http_answer(
    mut response: reqwest::Response,
    method: &str,
    id: Option<&Value>,
) -> Result<Option<Value>, String> {
    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(format!("{method}: HTTP {status}: {text}"));
    }
    let Some(id) = id else {
        return Ok(None);
    };
    let event_stream = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/event-stream"));
    if !event_stream {
        let body = response
            .bytes()
            .await
            .map_err(|e| format!("{method}: cannot read the answer: {e}"))?;
        return serde_json::from_slice(&body)
            .map(Some)
            .map_err(|e| format!("{method}: the answer is not JSON: {e}"));
    }
    let mut events = EventStream::default();
    loop {
        for data in events.take_data() {
            let reply: Value = serde_json::from_str(&data)
                .map_err(|e| format!("{method}: an event is not JSON ({e}): {data}"))?;
            if reply.get("id") == Some(id) {
                return Ok(Some(reply));
            }
        }
        let chunk = response
            .chunk()
            .await
            .map_err(|e| format!("{method}: cannot read the event stream: {e}"))?
            .ok_or_else(|| format!("{method}: the event stream ended before the answer"))?;
        events.push(&chunk);
    }
}

/// The bytes of a server-sent-event stream that are not yet whole events.
#[derive(Debug, Default)]
pub(super) struct EventStream {
    buffer: Vec<u8>,
}

impl EventStream {
    /// Adds the bytes of `chunk`, without carriage returns.
    pub(super) fn push(&mut self, chunk: &[u8]) {
        self.buffer
            .extend(chunk.iter().filter(|byte| **byte != b'\r'));
    }

    /// Removes the whole events from the buffer; returns the data of each event that has any (the
    /// `data:` lines, joined with newlines).
    pub(super) fn take_data(&mut self) -> Vec<String> {
        let mut found = Vec::new();
        while let Some(end) = self.buffer.windows(2).position(|pair| pair == b"\n\n") {
            let event: Vec<u8> = self.buffer.drain(..end + 2).collect();
            let event = String::from_utf8_lossy(&event);
            let data: Vec<&str> = event
                .lines()
                .filter_map(|line| line.strip_prefix("data:"))
                .map(|data| data.strip_prefix(' ').unwrap_or(data))
                .collect();
            let data = data.join("\n");
            if !data.trim().is_empty() {
                found.push(data);
            }
        }
        found
    }
}

/// Reads the example's stdout line by line and returns the lines that are not JSON-RPC
/// messages. For a stdio example (`messages` set) each JSON-RPC message goes on to the client
/// through `messages`; an HTTP example must print no line at all. Blank lines do not count.
pub(super) async fn read_stdout(
    stdout: ChildStdout,
    messages: Option<UnboundedSender<Value>>,
) -> Vec<String> {
    let mut reader = BufReader::new(stdout);
    let mut bad = Vec::new();
    let mut bytes = Vec::new();
    loop {
        bytes.clear();
        match reader.read_until(b'\n', &mut bytes).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let line = String::from_utf8_lossy(&bytes);
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match (&messages, json_rpc_message(line)) {
            // The client may have closed; the remaining lines are still judged.
            (Some(sender), Some(message)) => {
                let _ = sender.send(message);
            }
            _ => bad.push(line.to_owned()),
        }
    }
    bad
}

/// The JSON-RPC 2.0 message on `line`: a JSON object with `"jsonrpc": "2.0"`.
pub(super) fn json_rpc_message(line: &str) -> Option<Value> {
    serde_json::from_str::<Value>(line)
        .ok()
        .filter(|value| value["jsonrpc"] == "2.0")
}

//! The scripted client: hand-written JSON-RPC, one message per line over the in-memory pipe of a
//! stdio connection, or one HTTP POST per message with reqwest.
//!
//! - 2025-11-25: a session starts with `initialize` (id 0) and `notifications/initialized`. Over
//!   HTTP, later requests carry `mcp-protocol-version` and, when the server issued one,
//!   `mcp-session-id`; the client close sends `DELETE` for such a session.
//! - 2026-07-28: a session starts with `server/discover` (id `"discover"`), and every request
//!   carries the `_meta` envelope (protocol version, client info, client capabilities). Over
//!   HTTP, every request carries `mcp-protocol-version`, `mcp-method` and, for `tools/call`,
//!   `mcp-name`.
//!
//! Every HTTP request also carries the scenario's `httpHeaders`. The request IDs of a client count
//! from 0. Before each connect and each step, the driver waits until the wall clock is past the
//! millisecond in which the previous request was answered: the server stamps an event between a
//! request's arrival and its answer, so the events of two steps never share a millisecond and
//! their time order is the step order. A `parallel` step waits once, as a whole.
//!
//! The transcript holds one entry per session start (`initialize` or `discover`, with the
//! negotiated `protocolVersion`, `serverInfo`, `capabilities` and `instructions`) and one per
//! `listTools` or `callTool` (the `request`, then the `result` or the JSON-RPC `error`).
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::StatusCode;
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use serde_json::{Map, Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use super::Lane;
use super::scenario::{ClientInfo, Scenario, Step};
use super::server::Served;

/// The `_meta` key of the protocol version of a 2026-07-28 request.
const META_PROTOCOL_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
/// The `_meta` key of the client info of a 2026-07-28 request.
const META_CLIENT_INFO: &str = "io.modelcontextprotocol/clientInfo";
/// The `_meta` key of the client capabilities of a 2026-07-28 request.
const META_CLIENT_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
/// The `_meta` key of the server info of a `server/discover` result.
const META_SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";

/// The request ID of `server/discover`, outside the count of the other requests.
const DISCOVER_ID: &str = "discover";

/// What every POST accepts.
const ACCEPTED_TYPES: &str = "application/json, text/event-stream";

/// How long a stdio server may take to stop once the client closed its end of the pipe.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(10);

/// The clients of a run, to close once the events have arrived.
pub(crate) struct Clients(Vec<Client>);

impl Clients {
    /// Closes every client, in session order.
    ///
    /// # Panics
    ///
    /// When a stdio server does not stop cleanly within [`CLOSE_TIMEOUT`], or a `DELETE` fails.
    pub(crate) async fn close(self) {
        for client in self.0 {
            client.close().await;
        }
    }
}

/// Runs the sessions of `scenario` against `served` on `lane`; returns the transcript and the
/// clients, still connected.
///
/// # Panics
///
/// When a request cannot be sent, the server answers what the protocol does not allow, the
/// session negotiates another version than `lane`, or a step names a handle or cursor that no
/// earlier step saved.
pub(crate) async fn drive(
    scenario: &Scenario,
    served: &Served,
    lane: Lane,
) -> (Vec<Value>, Clients) {
    let headers = scenario.headers();
    let mut run = Run::default();
    let mut answered = 0;
    for (index, session) in scenario.sessions.iter().enumerate() {
        wait_past(answered).await;
        let (client, entry) = Client::connect(index, served, lane, &session.client, &headers).await;
        answered = now_ms();
        run.transcript.push(entry);
        run.clients.push(client);
        for (position, step) in session.steps.iter().enumerate() {
            wait_past(answered).await;
            run.step(index, step, &format!("step {position} of session {index}"))
                .await;
            answered = now_ms();
        }
    }
    (run.transcript, Clients(run.clients))
}

/// The state of a run: the transcript, the clients, and the handles and cursors saved so far.
#[derive(Default)]
struct Run {
    transcript: Vec<Value>,
    clients: Vec<Client>,
    handles: HashMap<String, String>,
    cursors: HashMap<String, String>,
}

impl Run {
    /// Runs `step` of session `session`; `at` names it in messages.
    async fn step(&mut self, session: usize, step: &Step, at: &str) {
        match step {
            Step::ListTools {
                cursor,
                save_cursor_as,
            } => {
                let params = cursor
                    .as_deref()
                    .map(|text| json!({"cursor": fill(text, &self.cursors, at, "a cursor")}));
                let mut entry = json!({"session": session, "op": "listTools"});
                if let Some(params) = &params {
                    entry["request"] = params.clone();
                }
                let answer = self.clients[session].request("tools/list", params).await;
                if let Some(name) = save_cursor_as {
                    let next = answer
                        .as_ref()
                        .ok()
                        .and_then(|result| result.get("nextCursor"))
                        .and_then(Value::as_str)
                        .unwrap_or_else(|| panic!("{at}: no nextCursor for {name}"));
                    self.cursors.insert(name.clone(), next.to_owned());
                }
                record(&mut entry, answer);
                self.transcript.push(entry);
            }
            Step::CallTool {
                session: Some(_), ..
            } => panic!("{at}: only a step inside parallel names a session"),
            Step::CallTool {
                name,
                args,
                save_handle_as,
                session: None,
            } => {
                let arguments = self.substitute(args, at);
                let entry = call(&self.clients[session], session, name, arguments).await;
                self.save_handle(save_handle_as.as_deref(), &entry, at);
                self.transcript.push(entry);
            }
            Step::Parallel { steps } => self.parallel(session, steps, at).await,
        }
    }

    /// Runs the `callTool` steps of a `parallel` step of session `session` at once, records their
    /// entries in step order, then saves their handles.
    async fn parallel(&mut self, session: usize, steps: &[Step], at: &str) {
        let mut calls = Vec::with_capacity(steps.len());
        for (position, step) in steps.iter().enumerate() {
            let at = format!("{at}, call {position}");
            let Step::CallTool {
                name,
                args,
                save_handle_as,
                session: on,
            } = step
            else {
                panic!("{at}: a parallel step holds callTool steps only")
            };
            let on = on.unwrap_or(session);
            assert!(
                on <= session,
                "{at}: session must be this or an earlier one"
            );
            calls.push((on, name, self.substitute(args, &at), save_handle_as, at));
        }
        let entries = futures::future::join_all(calls.iter().map(|(on, name, arguments, ..)| {
            call(&self.clients[*on], *on, name, arguments.clone())
        }))
        .await;
        for ((.., save_handle_as, at), entry) in calls.iter().zip(entries) {
            self.save_handle(save_handle_as.as_deref(), &entry, at);
            self.transcript.push(entry);
        }
    }

    /// `args` with each `${NAME}` in its strings replaced by the handle saved under `NAME`.
    fn substitute(&self, args: &Map<String, Value>, at: &str) -> Value {
        fn walk(value: &Value, handles: &HashMap<String, String>, at: &str) -> Value {
            match value {
                Value::String(text) => Value::String(fill(text, handles, at, "a session handle")),
                Value::Array(items) => items.iter().map(|item| walk(item, handles, at)).collect(),
                Value::Object(fields) => Value::Object(
                    fields
                        .iter()
                        .map(|(key, value)| (key.clone(), walk(value, handles, at)))
                        .collect(),
                ),
                other => other.clone(),
            }
        }
        walk(&Value::Object(args.clone()), &self.handles, at)
    }

    /// Saves the session handle that the result of `entry` issued under `name`, when given.
    fn save_handle(&mut self, name: Option<&str>, entry: &Value, at: &str) {
        let Some(name) = name else { return };
        let handle = entry
            .get("result")
            .and_then(issued_handle)
            .unwrap_or_else(|| panic!("{at}: no session handle for {name}"));
        self.handles.insert(name.to_owned(), handle);
    }
}

/// `text` with each `${NAME}` replaced by the value saved under `NAME` (`NAME` is letters, digits
/// and `_`); `what` names the kind of value in the panic for a name that nothing saved.
fn fill(text: &str, saved: &HashMap<String, String>, at: &str, what: &str) -> String {
    let mut filled = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("${") {
        let after = &rest[start + 2..];
        let name = after.find('}').map(|end| &after[..end]).filter(|name| {
            !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        });
        filled.push_str(&rest[..start]);
        if let Some(name) = name {
            let value = saved
                .get(name)
                .unwrap_or_else(|| panic!("{at}: no step saved {what} as {name}"));
            filled.push_str(value);
            rest = &after[name.len() + 1..];
        } else {
            filled.push_str("${");
            rest = after;
        }
    }
    filled.push_str(rest);
    filled
}

/// The session handle a tool result issued: `structuredContent.mcp_session.session_id`, else
/// the `ses_` ID after `session_id: ` in a text block.
fn issued_handle(result: &Value) -> Option<String> {
    let structured = result
        .pointer("/structuredContent/mcp_session/session_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty());
    if let Some(id) = structured {
        return Some(id.to_owned());
    }
    let texts = result
        .get("content")?
        .as_array()?
        .iter()
        .filter_map(|block| block.get("text")?.as_str());
    for text in texts {
        for (start, _) in text.match_indices("session_id: ses_") {
            let id = &text[start + "session_id: ".len()..];
            let body = id.get(4..31).filter(|body| {
                body.bytes().all(|b| b.is_ascii_alphanumeric())
                    && !id[31..].starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
            });
            if body.is_some() {
                return Some(id[..31].to_owned());
            }
        }
    }
    None
}

/// Calls the tool `name` with `arguments` on `client`, the client of session `session`; returns
/// the transcript entry.
async fn call(client: &Client, session: usize, name: &str, arguments: Value) -> Value {
    let request = json!({"name": name, "arguments": arguments});
    let mut entry = json!({"session": session, "op": "callTool", "request": request.clone()});
    let answer = client.request("tools/call", Some(request)).await;
    record(&mut entry, answer);
    entry
}

/// Adds the answer to `entry`: `result`, or `error` for a JSON-RPC error.
fn record(entry: &mut Value, answer: Result<Value, Value>) {
    match answer {
        Ok(result) => entry["result"] = result,
        Err(error) => entry["error"] = error,
    }
}

/// The result of a JSON-RPC response, or its error.
///
/// # Panics
///
/// When `message` is neither.
fn answer(message: Value) -> Result<Value, Value> {
    let Value::Object(mut fields) = message else {
        panic!("the answer is not a JSON-RPC response: {message}")
    };
    if let Some(result) = fields.remove("result") {
        return Ok(result);
    }
    fields.remove("error").map_or_else(
        || panic!("the answer is not a JSON-RPC response: {fields:?}"),
        Err,
    )
}

/// One client connection.
struct Client {
    link: Link,
    lane: Lane,
    /// The `_meta` envelope of a 2026-07-28 request.
    envelope: Value,
    /// The ID of the next request.
    next_id: AtomicU64,
}

impl Client {
    /// Connects a client for session `index` as `info` to `served` on `lane`, and starts the
    /// session (`initialize` or `server/discover`); returns the client and the transcript entry
    /// of the start.
    async fn connect(
        index: usize,
        served: &Served,
        lane: Lane,
        info: &ClientInfo,
        headers: &[(String, String)],
    ) -> (Self, Value) {
        let info = json!({"name": info.name, "version": info.version});
        let link = match served {
            Served::Stdio(_) => {
                let (stream, task) = served.connect_stdio();
                Link::Pipe(Pipe::new(stream, task))
            }
            Served::Http { url, .. } => Link::Web(Web::new(url, headers, lane)),
        };
        let envelope = Value::Object(Map::from_iter([
            (META_PROTOCOL_VERSION.to_owned(), json!(lane.version())),
            (META_CLIENT_INFO.to_owned(), info.clone()),
            (META_CLIENT_CAPABILITIES.to_owned(), json!({})),
        ]));
        let mut client = Self {
            link,
            lane,
            envelope,
            next_id: AtomicU64::new(0),
        };
        let (op, result) = match lane {
            Lane::V2025_11_25 => ("initialize", client.initialize(info).await),
            Lane::V2026_07_28 => ("discover", client.discover().await),
        };
        let entry = json!({"session": index, "op": op, "result": result});
        (client, entry)
    }

    /// `initialize` and `notifications/initialized`; returns what the server agreed to.
    async fn initialize(&mut self, info: Value) -> Value {
        let version = self.lane.version();
        let params = json!({"protocolVersion": version, "capabilities": {}, "clientInfo": info});
        let (message, session) = self.exchange("initialize", Some(params), None).await;
        let result = answer(message).unwrap_or_else(|error| panic!("initialize failed: {error}"));
        assert_eq!(
            result.get("protocolVersion").and_then(Value::as_str),
            Some(version),
            "the negotiated protocol version"
        );
        if let Link::Web(web) = &mut self.link {
            web.version = Some(version);
            web.session = session;
        }
        self.link
            .notify(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
            .await;
        json!({
            "protocolVersion": version,
            "serverInfo": result.get("serverInfo").cloned().unwrap_or(Value::Null),
            "capabilities": result.get("capabilities").cloned().unwrap_or(Value::Null),
            "instructions": result.get("instructions").cloned().unwrap_or(Value::Null),
        })
    }

    /// `server/discover`; returns what the server offers, on this lane's version.
    async fn discover(&mut self) -> Value {
        let version = self.lane.version();
        if let Link::Web(web) = &mut self.link {
            web.version = Some(version);
        }
        let id = json!(DISCOVER_ID);
        let params = self.with_envelope(None);
        let (message, _) = self.exchange_with(id, "server/discover", params).await;
        let result = answer(message).unwrap_or_else(|error| panic!("discover failed: {error}"));
        let offered = result
            .get("supportedVersions")
            .and_then(Value::as_array)
            .is_some_and(|versions| versions.iter().any(|v| v.as_str() == Some(version)));
        assert!(offered, "the server does not offer {version}: {result}");
        let server_info = result
            .get("_meta")
            .and_then(|meta| meta.get(META_SERVER_INFO))
            .cloned()
            .unwrap_or(Value::Null);
        json!({
            "protocolVersion": version,
            "serverInfo": server_info,
            "capabilities": result.get("capabilities").cloned().unwrap_or(Value::Null),
            "instructions": result.get("instructions").cloned().unwrap_or(Value::Null),
        })
    }

    /// Sends the request `method` with `params` (the envelope added on 2026-07-28); returns its
    /// result, or its JSON-RPC error.
    async fn request(&self, method: &str, params: Option<Value>) -> Result<Value, Value> {
        let params = self.with_envelope(params);
        let (message, _) = self.exchange(method, params, None).await;
        answer(message)
    }

    /// `params` with the `_meta` envelope on 2026-07-28, as they are on 2025-11-25.
    fn with_envelope(&self, params: Option<Value>) -> Option<Value> {
        match self.lane {
            Lane::V2025_11_25 => params,
            Lane::V2026_07_28 => {
                let mut params = params.unwrap_or_else(|| json!({}));
                params["_meta"] = self.envelope.clone();
                Some(params)
            }
        }
    }

    /// Sends the request `method` with the next ID (`id` when given); returns the response and
    /// the `mcp-session-id` the HTTP server answered with.
    async fn exchange(
        &self,
        method: &str,
        params: Option<Value>,
        id: Option<Value>,
    ) -> (Value, Option<String>) {
        let id = id.unwrap_or_else(|| json!(self.next_id.fetch_add(1, Ordering::Relaxed)));
        self.exchange_with(id, method, params).await
    }

    /// Sends the request `method` with `id` and `params`; returns the response and the
    /// `mcp-session-id` the HTTP server answered with.
    async fn exchange_with(
        &self,
        id: Value,
        method: &str,
        params: Option<Value>,
    ) -> (Value, Option<String>) {
        let mut message = Map::from_iter([
            ("jsonrpc".to_owned(), json!("2.0")),
            ("id".to_owned(), id),
            ("method".to_owned(), json!(method)),
        ]);
        let tool = params
            .as_ref()
            .filter(|_| method == "tools/call")
            .and_then(|params| params.get("name"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        if let Some(params) = params {
            message.insert("params".to_owned(), params);
        }
        let message = Value::Object(message);
        match &self.link {
            Link::Pipe(pipe) => (pipe.request(&message).await, None),
            Link::Web(web) => {
                let (reply, session) = web.post(&message, method, tool.as_deref()).await;
                let reply = reply.unwrap_or_else(|| panic!("{method}: the server answered 202"));
                (reply, session)
            }
        }
    }

    /// Closes the connection: the stdio pipe (and waits for its server to stop), or the HTTP
    /// session.
    async fn close(self) {
        match self.link {
            Link::Pipe(pipe) => pipe.close().await,
            Link::Web(web) => web.close().await,
        }
    }
}

/// How a client reaches its server.
enum Link {
    Pipe(Pipe),
    Web(Web),
}

impl Link {
    /// Sends the notification `message`.
    async fn notify(&self, message: &Value) {
        match self {
            Self::Pipe(pipe) => pipe.write(message).await,
            Self::Web(web) => {
                let method = message["method"].as_str().unwrap_or_default();
                let (reply, _) = web.post(message, method, None).await;
                assert!(reply.is_none(), "{method}: the server answered {reply:?}");
            }
        }
    }
}

/// The requests waiting for their answer, by the JSON text of their ID.
type Waiting = Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>;

/// The client's end of a stdio connection.
struct Pipe {
    writer: tokio::sync::Mutex<Option<WriteHalf<DuplexStream>>>,
    waiting: Waiting,
    /// Reads the server's lines and hands each answer to its request.
    reader: JoinHandle<()>,
    /// The task of the server, which ends when the client closes the pipe.
    server: JoinHandle<()>,
}

impl Pipe {
    /// The client's end `stream` of the pipe that the task `server` serves.
    fn new(stream: DuplexStream, server: JoinHandle<()>) -> Self {
        let (read, write) = tokio::io::split(stream);
        let waiting = Waiting::default();
        let reader = tokio::spawn(read_answers(read, Arc::clone(&waiting)));
        Self {
            writer: tokio::sync::Mutex::new(Some(write)),
            waiting,
            reader,
            server,
        }
    }

    /// Writes `message` and its newline.
    async fn write(&self, message: &Value) {
        let mut line = serde_json::to_vec(message).expect("a message serializes");
        line.push(b'\n');
        let mut writer = self.writer.lock().await;
        let writer = writer.as_mut().expect("the pipe is open");
        writer
            .write_all(&line)
            .await
            .expect("the pipe takes the line");
        writer.flush().await.expect("the pipe flushes");
    }

    /// Writes the request `message` and waits for its answer.
    async fn request(&self, message: &Value) -> Value {
        let key = message["id"].to_string();
        let (sender, receiver) = oneshot::channel();
        self.waiting
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.clone(), sender);
        self.write(message).await;
        receiver
            .await
            .unwrap_or_else(|_| panic!("the stdio server closed the pipe before answering {key}"))
    }

    /// Closes the client's end and waits for the server, then the reader, to stop.
    async fn close(self) {
        if let Some(mut writer) = self.writer.lock().await.take() {
            writer.shutdown().await.expect("the pipe closes");
        }
        for (task, what) in [
            (self.server, "the stdio server"),
            (self.reader, "the reader"),
        ] {
            match tokio::time::timeout(CLOSE_TIMEOUT, task).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => panic!("{what} failed: {error}"),
                Err(elapsed) => panic!("{what} did not stop within {CLOSE_TIMEOUT:?} ({elapsed})"),
            }
        }
    }
}

/// Reads the server's lines from `read` until the pipe closes, and hands each response to the
/// request in `waiting` with its ID. Notifications and server requests are ignored.
async fn read_answers(read: ReadHalf<DuplexStream>, waiting: Waiting) {
    let mut lines = BufReader::new(read).lines();
    while let Some(line) = lines.next_line().await.expect("the pipe reads") {
        let message: Value = serde_json::from_str(&line).unwrap_or_else(|error| {
            panic!("the server wrote a line that is not JSON ({error}): {line}")
        });
        let is_response = message.get("result").is_some() || message.get("error").is_some();
        let Some(id) = message.get("id").filter(|_| is_response) else {
            continue;
        };
        let sender = waiting
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&id.to_string());
        if let Some(sender) = sender {
            let _ignored = sender.send(message);
        }
    }
}

/// The client of a streamable HTTP connection.
struct Web {
    http: reqwest::Client,
    url: String,
    /// The scenario's headers.
    headers: Vec<(String, String)>,
    /// Whether requests carry `mcp-method` and `mcp-name` (2026-07-28).
    named: bool,
    /// The `mcp-protocol-version` of later requests, once known.
    version: Option<&'static str>,
    /// The `mcp-session-id` the server issued.
    session: Option<String>,
}

impl Web {
    /// A client of the endpoint `url` that sends `headers` with every request, on `lane`.
    fn new(url: &str, headers: &[(String, String)], lane: Lane) -> Self {
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
    async fn post(
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
    async fn close(self) {
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

/// The wall clock in milliseconds since the Unix epoch.
fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is after 1970")
        .as_millis()
}

/// Waits until the wall clock is past the millisecond `ms`.
async fn wait_past(ms: u128) {
    while now_ms() <= ms {
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
}

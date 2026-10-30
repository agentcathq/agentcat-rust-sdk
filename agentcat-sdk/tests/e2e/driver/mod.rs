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
//!
//! Each exchange (the send and the wait for the answer) must end within [`REQUEST_TIMEOUT`]; the
//! panic names the step. This module holds the steps and the protocol; `pipe` is the stdio
//! connection and `web` the HTTP one.
mod pipe;
mod web;

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

use super::Lane;
use super::scenario::{ClientInfo, Scenario, Step};
use super::server::Served;
use pipe::Pipe;
use web::Web;

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

/// How long one exchange may take: the send of a message and the wait for its answer.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// The clients of a run, to close once the events have arrived.
pub(crate) struct Clients(Vec<Client>);

impl Clients {
    /// Closes every client, in session order.
    ///
    /// # Panics
    ///
    /// When a stdio server does not stop cleanly, or a `DELETE` fails or takes longer than
    /// [`REQUEST_TIMEOUT`].
    pub(crate) async fn close(self) {
        for (index, client) in self.0.into_iter().enumerate() {
            client.close(&format!("the close of session {index}")).await;
        }
    }
}

/// Runs the sessions of `scenario` against `served` on `lane`; returns the transcript and the
/// clients, still connected.
///
/// # Panics
///
/// When a request cannot be sent or gets no answer within [`REQUEST_TIMEOUT`], the server
/// answers what the protocol does not allow, the session negotiates another version than `lane`,
/// or a step names a handle or cursor that no earlier step saved.
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
        let at = format!("the start of session {index}");
        let (client, entry) =
            Client::connect(index, served, lane, &session.client, &headers, &at).await;
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
                let answer = self.clients[session]
                    .request("tools/list", params, at)
                    .await;
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
                let entry = call(&self.clients[session], session, name, arguments, at).await;
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
        let entries =
            futures::future::join_all(calls.iter().map(|(on, name, arguments, _, at)| {
                call(&self.clients[*on], *on, name, arguments.clone(), at)
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

/// Calls the tool `name` with `arguments` on `client`, the client of session `session`, for the
/// step `at`; returns the transcript entry.
async fn call(client: &Client, session: usize, name: &str, arguments: Value, at: &str) -> Value {
    let request = json!({"name": name, "arguments": arguments});
    let mut entry = json!({"session": session, "op": "callTool", "request": request.clone()});
    let answer = client.request("tools/call", Some(request), at).await;
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
    /// session (`initialize` or `server/discover`; `at` names the start in messages); returns the
    /// client and the transcript entry of the start.
    async fn connect(
        index: usize,
        served: &Served,
        lane: Lane,
        info: &ClientInfo,
        headers: &[(String, String)],
        at: &str,
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
            Lane::V2025_11_25 => ("initialize", client.initialize(info, at).await),
            Lane::V2026_07_28 => ("discover", client.discover(at).await),
        };
        let entry = json!({"session": index, "op": op, "result": result});
        (client, entry)
    }

    /// `initialize` and `notifications/initialized`; returns what the server agreed to.
    async fn initialize(&mut self, info: Value, at: &str) -> Value {
        let version = self.lane.version();
        let params = json!({"protocolVersion": version, "capabilities": {}, "clientInfo": info});
        let id = self.next_id();
        let (message, session) = self.exchange(id, "initialize", Some(params), at).await;
        let result =
            answer(message).unwrap_or_else(|error| panic!("{at}: initialize failed: {error}"));
        assert_eq!(
            result.get("protocolVersion").and_then(Value::as_str),
            Some(version),
            "{at}: the negotiated protocol version"
        );
        if let Link::Web(web) = &mut self.link {
            web.version = Some(version);
            web.session = session;
        }
        let initialized = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        within(
            at,
            "notifications/initialized",
            self.link.notify(&initialized),
        )
        .await;
        json!({
            "protocolVersion": version,
            "serverInfo": result.get("serverInfo").cloned().unwrap_or(Value::Null),
            "capabilities": result.get("capabilities").cloned().unwrap_or(Value::Null),
            "instructions": result.get("instructions").cloned().unwrap_or(Value::Null),
        })
    }

    /// `server/discover`; returns what the server offers, on this lane's version.
    async fn discover(&mut self, at: &str) -> Value {
        let version = self.lane.version();
        if let Link::Web(web) = &mut self.link {
            web.version = Some(version);
        }
        let params = self.with_envelope(None);
        let (message, _) = self
            .exchange(json!(DISCOVER_ID), "server/discover", params, at)
            .await;
        let result =
            answer(message).unwrap_or_else(|error| panic!("{at}: discover failed: {error}"));
        let offered = result
            .get("supportedVersions")
            .and_then(Value::as_array)
            .is_some_and(|versions| versions.iter().any(|v| v.as_str() == Some(version)));
        assert!(
            offered,
            "{at}: the server does not offer {version}: {result}"
        );
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

    /// Sends the request `method` with `params` (the envelope added on 2026-07-28) for the step
    /// `at`; returns its result, or its JSON-RPC error.
    async fn request(&self, method: &str, params: Option<Value>, at: &str) -> Result<Value, Value> {
        let params = self.with_envelope(params);
        let (message, _) = self.exchange(self.next_id(), method, params, at).await;
        answer(message)
    }

    /// The ID of the next request.
    fn next_id(&self) -> Value {
        json!(self.next_id.fetch_add(1, Ordering::Relaxed))
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

    /// Sends the request `method` with `id` and `params` for the step `at`; returns the response
    /// and the `mcp-session-id` the HTTP server answered with.
    ///
    /// # Panics
    ///
    /// When the exchange takes longer than [`REQUEST_TIMEOUT`], or the HTTP server answers 202.
    async fn exchange(
        &self,
        id: Value,
        method: &str,
        params: Option<Value>,
        at: &str,
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
            Link::Pipe(pipe) => (within(at, method, pipe.request(&message)).await, None),
            Link::Web(web) => {
                let (reply, session) =
                    within(at, method, web.post(&message, method, tool.as_deref())).await;
                let reply =
                    reply.unwrap_or_else(|| panic!("{at}: {method}: the server answered 202"));
                (reply, session)
            }
        }
    }

    /// Closes the connection (`at` names the close in messages): the stdio pipe (and waits for
    /// its server to stop), or the HTTP session.
    async fn close(self, at: &str) {
        match self.link {
            Link::Pipe(pipe) => pipe.close().await,
            Link::Web(web) => within(at, "DELETE", web.close()).await,
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

/// The output of `exchange`, the exchange of `what` for the step `at`.
///
/// # Panics
///
/// When `exchange` takes longer than [`REQUEST_TIMEOUT`].
async fn within<T>(at: &str, what: &str, exchange: impl Future<Output = T>) -> T {
    tokio::time::timeout(REQUEST_TIMEOUT, exchange)
        .await
        .unwrap_or_else(|elapsed| {
            panic!("{at}: {what} got no answer within {REQUEST_TIMEOUT:?} ({elapsed})")
        })
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

#[test]
fn fill_replaces_each_saved_name() {
    let saved = HashMap::from([("S1".to_owned(), "ses_A".to_owned())]);
    for (text, filled) in [
        ("${S1}", "ses_A"),
        ("a ${S1} b ${S1}", "a ses_A b ses_A"),
        ("$${S1}}", "$ses_A}"),
        ("${S1", "${S1"),
        ("${}", "${}"),
        ("${not a name}", "${not a name}"),
        ("start", "start"),
    ] {
        assert_eq!(fill(text, &saved, "test", "a handle"), filled, "{text}");
    }
}

#[test]
#[should_panic(expected = "test: no step saved a handle as S2")]
fn fill_refuses_a_name_that_no_step_saved() {
    fill("${S2}", &HashMap::new(), "test", "a handle");
}

#[test]
fn issued_handle_reads_the_structured_content_then_the_text() {
    let id = "ses_3LN9sNQXZmvwvVJXsJUQym6w950";
    let other = "ses_3LN9sTmh7fwKQfXHiW4c9Mc0P0P";
    let text = |text: String| json!({"content": [{"type": "text", "text": text}]});
    for (result, handle) in [
        (
            json!({
                "content": [{"type": "text", "text": format!("session_id: {other}")}],
                "structuredContent": {"mcp_session": {"session_id": id}},
            }),
            Some(id),
        ),
        (
            json!({
                "content": [{"type": "text", "text": format!("session_id: {id}")}],
                "structuredContent": {"mcp_session": {"session_id": ""}},
            }),
            Some(id),
        ),
        (
            text(format!(
                "[issued]
session_id: {id}
more"
            )),
            Some(id),
        ),
        (text(format!("session_id: {id}x")), None),
        (text(format!("session_id: {id}_")), None),
        (
            text(format!("session_id: {id}x, session_id: {other}")),
            Some(other),
        ),
        (text("session_id: ses_short".to_owned()), None),
        (json!({"content": [{"type": "image", "data": ""}]}), None),
        (json!({}), None),
    ] {
        assert_eq!(issued_handle(&result).as_deref(), handle, "{result}");
    }
}

#[tokio::test(start_paused = true)]
#[should_panic(expected = "step 3 of session 0: tools/call got no answer within 30s")]
async fn an_exchange_without_an_answer_names_its_step() {
    within(
        "step 3 of session 0",
        "tools/call",
        std::future::pending::<()>(),
    )
    .await;
}

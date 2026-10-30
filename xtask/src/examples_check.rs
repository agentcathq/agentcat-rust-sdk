//! The examples check: runs one customer example of `agentcat-sdk` against a local mock of the
//! `AgentCat` API and checks what a person who copies the example expects.
//!
//! `cargo run -q -p xtask -- examples-check <stdio | streamable_http | hooks | stateless_http>`
//! builds the example (`cargo build -p agentcat-sdk --example <name> --all-features`), starts the
//! mock on 127.0.0.1 and runs the example in a clean environment: every `AGENTCAT_*` and
//! `MCPCAT_*` variable is removed, then `AGENTCAT_API_URL` is the mock, `AGENTCAT_PROJECT_ID` is
//! `proj_examples` and `HOME` is a temporary directory (the SDK writes `~/agentcat.log` there),
//! which the check removes after the run. The check refuses to start the example when the API URL
//! is not a loopback address, so no run reaches a production host.
//!
//! A JSON-RPC client (lines on stdin and stdout, or one POST per message with a JSON or
//! server-sent-event answer for streamable HTTP) sends `initialize`, `tools/list` and three tool
//! calls with protocol version 2025-11-25, and closes the connection right after the last call:
//! for a stdio example it closes stdin; for an HTTP example it stops sending and then sends
//! SIGTERM (Unix) or ends the process (Windows, which has no SIGTERM).
//! The check asserts:
//! - the first call issues a session ID in a text block before the tool's result, and keeps that
//!   result;
//! - each call returns the expected text and `isError`;
//! - one `mcp:tools/call` publish per call, every publish for `proj_examples`, no request other
//!   than a publish, and for `hooks` the values of the hooks and one custom event;
//! - every stdout line is a JSON-RPC message (an HTTP example prints no stdout line at all);
//! - the example exits 0 within 5 s of the stdin close or the SIGTERM;
//! - the publish of the last call arrives before the exit.
//!
//! On Windows an HTTP example cannot shut down on its own (the process is ended), so it fails the
//! exit status there; CI runs the check on Linux.
//!
//! It prints `ok <name>` and the time from the close to the exit, or a `FAIL <name>: <text>` line
//! for each failed assertion and exits 1.
use std::ffi::OsString;
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{Method, StatusCode, Uri, header};
use serde_json::{Map, Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

/// The project ID of every example run.
pub const PROJECT_ID: &str = "proj_examples";

/// How long an example may take to exit after the stdin close or the SIGTERM.
pub const EXIT_BUDGET: Duration = Duration::from_secs(5);

/// How long an HTTP example may take to listen on its port.
const LISTEN_TIMEOUT: Duration = Duration::from_secs(20);

/// How long the client may take from the start of the example to the close of the connection.
const SESSION_TIMEOUT: Duration = Duration::from_secs(60);

/// How long the readers of the example's stdout and stderr may take to finish after its exit.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

/// The path of a publish request.
const PUBLISH_PATH: &str = "/v1/public/events/";

/// The `context` argument of every call.
const CONTEXT: &str = "Checking the example";

/// The MCP protocol version the client asks for: the newest with `initialize` on every rmcp 3.x
/// minor.
const PROTOCOL_VERSION: &str = "2025-11-25";

/// The `Accept` header of every POST to an HTTP example.
const ACCEPTED_TYPES: &str = "application/json, text/event-stream";

/// How an example talks MCP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Transport {
    /// JSON-RPC lines on stdin and stdout.
    Stdio,
    /// Streamable HTTP at `http://127.0.0.1:$PORT/mcp`.
    Http,
}

/// One example and its calls.
#[derive(Debug, Clone, Copy)]
struct Example {
    /// The name of the example (`agentcat-sdk/examples/<name>.rs`).
    name: &'static str,
    transport: Transport,
    /// Whether the example is `hooks`: its own calls, a `fail` tool and the hook values.
    hooks: bool,
}

/// The examples the check knows.
const EXAMPLES: [Example; 4] = [
    Example {
        name: "stdio",
        transport: Transport::Stdio,
        hooks: false,
    },
    Example {
        name: "streamable_http",
        transport: Transport::Http,
        hooks: false,
    },
    Example {
        name: "hooks",
        transport: Transport::Stdio,
        hooks: true,
    },
    Example {
        name: "stateless_http",
        transport: Transport::Http,
        hooks: false,
    },
];

impl Example {
    /// The tools `tools/list` must list.
    fn tools(self) -> &'static [&'static str] {
        if self.hooks {
            &["add", "echo", "fail"]
        } else {
            &["add", "echo"]
        }
    }

    /// The three calls of the run, in order.
    fn calls(self) -> Vec<Call> {
        if self.hooks {
            vec![
                Call::new("add", json!({"a": 2, "b": 3}), "5", false),
                Call::new(
                    "echo",
                    json!({"text": "my secret-token is here"}),
                    "my secret-token is here",
                    false,
                ),
                Call::new("fail", json!({}), "the record does not exist", true),
            ]
        } else {
            vec![
                Call::new("add", json!({"a": 2, "b": 3}), "5", false),
                Call::new("echo", json!({"text": "hello"}), "hello", false),
                Call::new("add", json!({"a": 4, "b": 5}), "9", false),
            ]
        }
    }

    /// What ends the connection: the name the failure texts use.
    fn close_name(self) -> &'static str {
        match self.transport {
            Transport::Stdio => "stdin close",
            Transport::Http if cfg!(unix) => "SIGTERM",
            Transport::Http => "process end",
        }
    }
}

/// One tool call and the answer it must get.
#[derive(Debug, Clone)]
struct Call {
    tool: &'static str,
    /// The tool's own arguments, a JSON object.
    arguments: Value,
    /// The text of the tool's result.
    text: &'static str,
    /// The `isError` of the result.
    is_error: bool,
}

impl Call {
    fn new(tool: &'static str, arguments: Value, text: &'static str, is_error: bool) -> Self {
        Self {
            tool,
            arguments,
            text,
            is_error,
        }
    }
}

/// Parses the arguments, builds the example and runs the check; returns the exit code and the
/// lines to print.
#[must_use]
pub fn run(args: &[String], root: &Path) -> (u8, Vec<String>) {
    let [name] = args else {
        return usage();
    };
    let Some(example) = EXAMPLES.iter().find(|e| e.name == name.as_str()).copied() else {
        return usage();
    };
    let binary = match build(root, example.name) {
        Ok(binary) => binary,
        Err(message) => return (1, vec![format!("examples-check: {message}")]),
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            return (
                1,
                vec![format!("examples-check: cannot start the runtime: {e}")],
            );
        }
    };
    let outcome = runtime.block_on(check(example, &binary, root, EXIT_BUDGET));
    report(example, &outcome)
}

fn usage() -> (u8, Vec<String>) {
    let names: Vec<&str> = EXAMPLES.iter().map(|e| e.name).collect();
    (
        2,
        vec![format!(
            "usage: cargo run -q -p xtask -- examples-check <{}>",
            names.join(" | ")
        )],
    )
}

/// The exit code and the lines of `outcome`: the example's stderr comes first when the run
/// failed.
fn report(example: Example, outcome: &Outcome) -> (u8, Vec<String>) {
    let name = example.name;
    if outcome.fatal.is_none() && outcome.failures.is_empty() {
        let after = outcome.exit_after.map_or_else(String::new, |after| {
            format!(
                " (exit {} ms after the {})",
                after.as_millis(),
                example.close_name()
            )
        });
        return (0, vec![format!("ok {name}{after}")]);
    }
    let mut lines = Vec::new();
    if !outcome.stderr.is_empty() {
        lines.push(format!(
            "--- stderr of the example ---\n{}",
            outcome.stderr.trim_end()
        ));
    }
    lines.extend(
        outcome
            .failures
            .iter()
            .map(|text| format!("FAIL {name}: {text}")),
    );
    if let Some(message) = &outcome.fatal {
        lines.push(format!("examples-check: {message}"));
    }
    (1, lines)
}

/// Builds the example and returns the path of its executable, which cargo reports.
fn build(root: &Path, name: &str) -> Result<PathBuf, String> {
    let output = std::process::Command::new("cargo")
        .args([
            "build",
            "-q",
            "-p",
            "agentcat-sdk",
            "--example",
            name,
            "--all-features",
            "--message-format=json-render-diagnostics",
        ])
        .current_dir(root)
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| format!("cannot run `cargo build`: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo build of the example {name} failed ({})",
            output.status
        ));
    }
    example_executable(&String::from_utf8_lossy(&output.stdout), name)
        .ok_or_else(|| format!("cargo build reported no executable for the example {name}"))
}

/// The executable of the example `name` in the JSON messages of `cargo build`.
fn example_executable(messages: &str, name: &str) -> Option<PathBuf> {
    messages
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|message| {
            message["reason"] == "compiler-artifact"
                && message["target"]["name"] == name
                && message["target"]["kind"]
                    .as_array()
                    .is_some_and(|kinds| kinds.iter().any(|kind| kind == "example"))
        })
        .find_map(|message| message["executable"].as_str().map(PathBuf::from))
}

/// The result of one run.
#[derive(Debug, Default)]
struct Outcome {
    /// The failed assertions.
    failures: Vec<String>,
    /// The error that stopped the run before every assertion ran.
    fatal: Option<String>,
    /// The example's stderr.
    stderr: String,
    /// The time from the close of the connection to the exit, when the example exited in time.
    exit_after: Option<Duration>,
}

/// Runs the example at `binary` and checks it.
async fn check(example: Example, binary: &Path, root: &Path, exit_budget: Duration) -> Outcome {
    let mut outcome = Outcome::default();
    let (mock, home, port) = match prepare(example).await {
        Ok(prepared) => prepared,
        Err(message) => {
            outcome.fatal = Some(message);
            return outcome;
        }
    };
    let inherited = std::env::vars_os().map(|(name, _)| name);
    let started = example_env(inherited, &mock.url, &home.0, port)
        .and_then(|env| start(example, binary, root, &env));
    let Started {
        mut child,
        stdin,
        messages,
        stdout_reader,
        stderr_reader,
    } = match started {
        Ok(started) => started,
        Err(message) => {
            outcome.fatal = Some(message);
            return outcome;
        }
    };

    let driven = tokio::time::timeout(
        SESSION_TIMEOUT,
        drive(
            example,
            &mut child,
            port,
            stdin,
            messages,
            &mut outcome.failures,
        ),
    )
    .await
    .unwrap_or_else(|_| {
        Err(format!(
            "the client did not finish within {} s",
            SESSION_TIMEOUT.as_secs()
        ))
    });
    let exited_at = match driven {
        Ok(closed_at) => wait_for_exit(example, &mut child, closed_at, exit_budget, &mut outcome)
            .await
            .unwrap_or_else(|message| {
                outcome.fatal = Some(message);
                None
            }),
        Err(message) => {
            outcome.fatal = Some(message);
            let _ = child.start_kill();
            let _ = child.wait().await;
            None
        }
    };

    if let Ok(Ok(bad_lines)) = tokio::time::timeout(DRAIN_TIMEOUT, stdout_reader).await {
        outcome.failures.extend(bad_lines.iter().map(|line| {
            format!(
                "a stdout line is not JSON-RPC: {}",
                Value::String(line.clone())
            )
        }));
    }
    if let Ok(Ok(text)) = tokio::time::timeout(DRAIN_TIMEOUT, stderr_reader).await {
        outcome.stderr = text;
    }
    if outcome.fatal.is_none() {
        let requests = mock.requests();
        outcome
            .failures
            .extend(judge_publishes(&requests, &example.calls(), exited_at));
        if example.hooks {
            outcome.failures.extend(judge_hooks(&requests));
        }
    }
    drop(home);
    outcome
}

/// The mock API, the temporary home and, for an HTTP example, a free port.
async fn prepare(example: Example) -> Result<(MockApi, TempHome, Option<u16>), String> {
    let mock = MockApi::start().await?;
    let home = TempHome::new()?;
    let port = match example.transport {
        Transport::Stdio => None,
        Transport::Http => Some(free_port()?),
    };
    Ok((mock, home, port))
}

/// A started example and the readers of its output.
struct Started {
    child: Child,
    stdin: ChildStdin,
    /// The JSON-RPC messages of a stdio example's stdout.
    messages: UnboundedReceiver<Value>,
    /// Returns the stdout lines that are not JSON-RPC messages, after the example's exit.
    stdout_reader: tokio::task::JoinHandle<Vec<String>>,
    /// Returns the example's stderr, after its exit.
    stderr_reader: tokio::task::JoinHandle<String>,
}

/// Starts the example at `binary` with `env`, and the readers of its stdout and stderr.
fn start(
    example: Example,
    binary: &Path,
    root: &Path,
    env: &ExampleEnv,
) -> Result<Started, String> {
    let mut command = Command::new(binary);
    env.apply(&mut command);
    command
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", binary.display()))?;
    let (Some(stdin), Some(stdout), Some(mut stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        return Err("the example has no stdin, stdout or stderr pipe".to_owned());
    };
    let stderr_reader = tokio::spawn(async move {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes).await;
        String::from_utf8_lossy(&bytes).into_owned()
    });
    let (sender, messages) = unbounded_channel();
    let forward = example.transport == Transport::Stdio;
    let stdout_reader = tokio::spawn(read_stdout(stdout, forward.then_some(sender)));
    Ok(Started {
        child,
        stdin,
        messages,
        stdout_reader,
        stderr_reader,
    })
}

/// Waits for the example's exit after the close at `closed_at`; returns the time of the exit when
/// it came within `exit_budget`. A late exit or a status other than 0 is a failure.
async fn wait_for_exit(
    example: Example,
    child: &mut Child,
    closed_at: Instant,
    exit_budget: Duration,
    outcome: &mut Outcome,
) -> Result<Option<Instant>, String> {
    match tokio::time::timeout(exit_budget, child.wait()).await {
        Ok(Ok(status)) => {
            let exited_at = Instant::now();
            if !status.success() {
                let status = status
                    .code()
                    .map_or_else(|| status.to_string(), |code| code.to_string());
                outcome
                    .failures
                    .push(format!("the example exited with status {status}"));
            }
            outcome.exit_after = Some(exited_at.duration_since(closed_at));
            Ok(Some(exited_at))
        }
        Ok(Err(e)) => Err(format!("cannot wait for the example: {e}")),
        Err(_) => {
            outcome.failures.push(format!(
                "the example did not exit within {} ms of the {}",
                exit_budget.as_millis(),
                example.close_name()
            ));
            let _ = child.start_kill();
            let _ = child.wait().await;
            Ok(None)
        }
    }
}

/// Connects to the example, lists its tools, makes the calls and closes the connection right
/// after the last call, with no pause; returns the time of the close.
async fn drive(
    example: Example,
    child: &mut Child,
    port: Option<u16>,
    stdin: ChildStdin,
    messages: UnboundedReceiver<Value>,
    failures: &mut Vec<String>,
) -> Result<Instant, String> {
    // An HTTP example keeps its stdin open until the end of the run.
    let (connection, _held_stdin) = match port {
        None => (Connection::Stdio { stdin, messages }, None),
        Some(port) => {
            wait_for_port(child, port).await?;
            let http = reqwest::Client::builder()
                .no_proxy()
                .build()
                .map_err(|e| format!("cannot build the HTTP client: {e}"))?;
            let connection = Connection::Http {
                http,
                url: format!("http://127.0.0.1:{port}/mcp"),
                session: None,
                version: None,
            };
            (connection, Some(stdin))
        }
    };
    let mut client = Client::connect(connection).await?;
    make_calls(&mut client, example, failures).await?;
    let closed_at = Instant::now();
    drop(client);
    if example.transport == Transport::Http {
        stop(child).await?;
    }
    Ok(closed_at)
}

/// Lists the tools and makes the calls of `example`; a failed request stops the run.
async fn make_calls(
    client: &mut Client,
    example: Example,
    failures: &mut Vec<String>,
) -> Result<(), String> {
    let listed = client.request("tools/list", json!({})).await?;
    failures.extend(judge_tools(&listed["tools"], example.tools()));
    let mut session = None;
    for call in example.calls() {
        let params = json!({
            "name": call.tool,
            "arguments": call_arguments(&call, session.as_deref()),
        });
        let result = client.request("tools/call", params).await?;
        failures.extend(judge_call(&call, &result, &mut session));
    }
    Ok(())
}

/// A JSON-RPC client of the example. Dropping it closes the connection: the stdin of a stdio
/// example closes; an HTTP example sees no further request.
struct Client {
    connection: Connection,
    /// The ID of the next request.
    next_id: u64,
}

/// How the client reaches the example.
enum Connection {
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
    async fn connect(connection: Connection) -> Result<Self, String> {
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
    async fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
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
struct EventStream {
    buffer: Vec<u8>,
}

impl EventStream {
    /// Adds the bytes of `chunk`, without carriage returns.
    fn push(&mut self, chunk: &[u8]) {
        self.buffer
            .extend(chunk.iter().filter(|byte| **byte != b'\r'));
    }

    /// Removes the whole events from the buffer; returns the data of each event that has any (the
    /// `data:` lines, joined with newlines).
    fn take_data(&mut self) -> Vec<String> {
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

/// Ends an HTTP example: SIGTERM, which the example answers with its shutdown.
#[cfg(unix)]
async fn stop(child: &mut Child) -> Result<(), String> {
    let Some(pid) = child.id() else {
        return Ok(());
    };
    let status = Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .status()
        .await
        .map_err(|e| format!("cannot run `kill`: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`kill -TERM {pid}` failed ({status})"))
    }
}

/// Ends an HTTP example. Windows has no SIGTERM: the process ends at once.
#[cfg(not(unix))]
async fn stop(child: &mut Child) -> Result<(), String> {
    child
        .start_kill()
        .map_err(|e| format!("cannot end the example: {e}"))
}

/// Reads the example's stdout line by line and returns the lines that are not JSON-RPC
/// messages. For a stdio example (`messages` set) each JSON-RPC message goes on to the client
/// through `messages`; an HTTP example must print no line at all. Blank lines do not count.
async fn read_stdout(stdout: ChildStdout, messages: Option<UnboundedSender<Value>>) -> Vec<String> {
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
fn json_rpc_message(line: &str) -> Option<Value> {
    serde_json::from_str::<Value>(line)
        .ok()
        .filter(|value| value["jsonrpc"] == "2.0")
}

/// A free TCP port on 127.0.0.1.
fn free_port() -> Result<u16, String> {
    std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .and_then(|listener| listener.local_addr())
        .map(|address| address.port())
        .map_err(|e| format!("no free loopback port: {e}"))
}

/// Waits until the example accepts a connection on `port`; fails when it exits first or after
/// [`LISTEN_TIMEOUT`].
async fn wait_for_port(child: &mut Child, port: u16) -> Result<(), String> {
    let end = Instant::now() + LISTEN_TIMEOUT;
    while Instant::now() < end {
        if tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port))
            .await
            .is_ok()
        {
            return Ok(());
        }
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!(
                "the example exited ({status}) before it listened on port {port}"
            ));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Err(format!("the example did not listen on port {port}"))
}

/// The arguments of `call`: its own, the `context`, and the `session_id` once the first call
/// issued one.
fn call_arguments(call: &Call, session: Option<&str>) -> Map<String, Value> {
    let mut arguments = call.arguments.as_object().cloned().unwrap_or_default();
    arguments.insert("context".to_owned(), Value::from(CONTEXT));
    if let Some(session) = session.filter(|session| !session.is_empty()) {
        arguments.insert("session_id".to_owned(), Value::from(session));
    }
    arguments
}

/// The failures of a `tools/list` answer (`tools`, a JSON list of tools) that misses a tool of
/// `expected`.
fn judge_tools(tools: &Value, expected: &[&str]) -> Vec<String> {
    let names: Vec<&str> = tools
        .as_array()
        .map(|tools| {
            tools
                .iter()
                .filter_map(|tool| tool["name"].as_str())
                .collect()
        })
        .unwrap_or_default();
    expected
        .iter()
        .filter(|tool| !names.contains(tool))
        .map(|tool| format!("tools/list does not list {tool}: {}", json!(names)))
        .collect()
}

/// The ID in a text block that issues a session: the `ses_` word after `session_id: `.
fn issued_session_id(text: &str) -> Option<String> {
    const MARK: &str = "session_id: ses_";
    text.match_indices(MARK).find_map(|(at, _)| {
        let rest = &text[at + MARK.len()..];
        let end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .unwrap_or(rest.len());
        (end > 0).then(|| format!("ses_{}", &rest[..end]))
    })
}

/// The failures of the result of `call` (`result`, a JSON `CallToolResult`).
///
/// The first call of a server has no session ID (`session` is `None`). The SDK puts a text block
/// that holds the ID it issues before the tool's own result, which it keeps; the later calls pass
/// that ID back. So the tool's text is the first block of a later call, and the block after the
/// issued one on the first call. After the first call, `session` holds the issued ID, or an empty
/// text when the call issued none.
fn judge_call(call: &Call, result: &Value, session: &mut Option<String>) -> Vec<String> {
    let mut failures = Vec::new();
    let content: &[Value] = result["content"].as_array().map_or(&[], Vec::as_slice);
    let mut rest = content;
    if session.is_none() {
        let first = content.first().and_then(|block| block["text"].as_str());
        if let Some(id) = first.and_then(issued_session_id) {
            *session = Some(id);
            rest = &content[1..];
        } else {
            failures.push(format!(
                "the first call ({}) did not issue a session ID: {}",
                call.tool,
                json!(first)
            ));
            *session = Some(String::new());
        }
    }
    let texts: Vec<Value> = rest.iter().map(|block| block["text"].clone()).collect();
    let got = match texts.as_slice() {
        [one] => one.clone(),
        _ => Value::Array(texts),
    };
    let is_error = is_truthy(&result["isError"]);
    if got != call.text || is_error != call.is_error {
        failures.push(format!(
            "{} returned {got} (isError {is_error}), expected {} (isError {})",
            call.tool,
            json!(call.text),
            call.is_error
        ));
    }
    failures
}

/// Whether a JSON value counts as true: not `null`, `false`, `0` or an empty text.
fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|n| n != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// The field `camel` of an event, else the field `snake`; `None` when both are absent or null.
fn field<'a>(event: &'a Value, camel: &str, snake: &str) -> Option<&'a Value> {
    [camel, snake]
        .into_iter()
        .filter_map(|name| event.get(name))
        .find(|value| !value.is_null())
}

/// The event type of a publish body.
fn event_type(event: &Value) -> Option<&str> {
    field(event, "eventType", "event_type").and_then(Value::as_str)
}

/// The resource name of a publish body.
fn resource_name(event: &Value) -> Option<&str> {
    field(event, "resourceName", "resource_name").and_then(Value::as_str)
}

/// The failures of the requests the mock received: one tool-call publish per call (`calls`),
/// every publish for [`PROJECT_ID`], no request other than a publish, and the publish of the
/// last call before the exit at `exited_at` (`None` when the example did not exit in time).
fn judge_publishes(
    requests: &[Recorded],
    calls: &[Call],
    exited_at: Option<Instant>,
) -> Vec<String> {
    let mut failures = Vec::new();
    let (publishes, stray): (Vec<&Recorded>, Vec<&Recorded>) =
        requests.iter().partition(|request| request.is_publish());
    if !stray.is_empty() {
        failures.push(format!(
            "the mock received {} request(s) that are not publishes",
            stray.len()
        ));
    }
    let tool_calls: Vec<&Recorded> = publishes
        .iter()
        .copied()
        .filter(|request| event_type(&request.body) == Some("mcp:tools/call"))
        .collect();
    let got: Vec<Option<&str>> = tool_calls
        .iter()
        .map(|request| resource_name(&request.body))
        .collect();
    let want: Vec<&str> = calls.iter().map(|call| call.tool).collect();
    let mut got_sorted = got.clone();
    got_sorted.sort_unstable();
    let mut want_sorted: Vec<Option<&str>> = want.iter().copied().map(Some).collect();
    want_sorted.sort_unstable();
    if got_sorted != want_sorted {
        failures.push(format!(
            "publishes for the tool calls: got {}, expected {}",
            json!(got),
            json!(want)
        ));
    }
    let last = want.last().copied();
    let last_call = tool_calls
        .iter()
        .rev()
        .find(|request| resource_name(&request.body) == last);
    if last_call
        .zip(exited_at)
        .is_some_and(|(request, exited_at)| request.received > exited_at)
    {
        failures.push("the publish of the last call arrived after the example exited".into());
    }
    for request in &publishes {
        let project = field(&request.body, "projectId", "project_id");
        if project.and_then(Value::as_str) != Some(PROJECT_ID) {
            failures.push(format!(
                "a publish names the project {}, expected {PROJECT_ID}",
                project.unwrap_or(&Value::Null)
            ));
        }
    }
    failures
}

/// The failures of the hook values of the `hooks` example in the publishes of `requests`.
fn judge_hooks(requests: &[Recorded]) -> Vec<String> {
    let mut failures = Vec::new();
    let publishes: Vec<&Recorded> = requests.iter().filter(|r| r.is_publish()).collect();
    let tool_events: Vec<&Value> = publishes
        .iter()
        .map(|request| &request.body)
        .filter(|event| event_type(event) == Some("mcp:tools/call"))
        .collect();
    for event in &tool_events {
        let tool = resource_name(event).unwrap_or("undefined");
        let text = |camel: &str, snake: &str| field(event, camel, snake).and_then(Value::as_str);
        if text("identifyActorGivenId", "identify_actor_given_id") != Some("demo-user") {
            failures.push(format!("{tool}: identify user_id is not \"demo-user\""));
        }
        if text("identifyActorName", "identify_actor_name") != Some("Demo User") {
            failures.push(format!("{tool}: identify user_name is not \"Demo User\""));
        }
        if event["tags"]["example"] != "hooks" {
            failures.push(format!("{tool}: the tag example is not \"hooks\""));
        }
        if event["properties"]["source"] != "hooks-example" {
            failures.push(format!(
                "{tool}: the property source is not \"hooks-example\""
            ));
        }
        if event["properties"]["redacted_by"] != "example" {
            failures.push(format!("{tool}: the redact_event hook did not run"));
        }
    }
    if publishes.iter().any(|r| r.raw.contains("secret-token")) {
        failures.push("a publish still holds secret-token".to_owned());
    }
    if !publishes.iter().any(|r| r.raw.contains("[REDACTED]")) {
        failures.push("no publish holds [REDACTED]".to_owned());
    }
    let fail_event = tool_events
        .iter()
        .find(|event| resource_name(event) == Some("fail"));
    let recorded = fail_event.is_some_and(|event| {
        let error = &event["error"];
        let error_type = field(error, "type", "name").unwrap_or(&Value::Null);
        field(event, "isError", "is_error") == Some(&Value::Bool(true)) && is_truthy(error_type)
    });
    if !recorded {
        failures.push("the fail call has no error recorded by capture_error".to_owned());
    }
    let customs: Vec<&Value> = publishes
        .iter()
        .map(|request| &request.body)
        .filter(|event| event_type(event) == Some("agentcat:custom"))
        .collect();
    match customs.as_slice() {
        [custom] => {
            let name = field(custom, "resourceName", "resource_name");
            if name.and_then(Value::as_str) != Some("echo-called") {
                failures.push(format!(
                    "the custom event has resource {}, expected \"echo-called\"",
                    name.unwrap_or(&Value::Null)
                ));
            }
        }
        _ => failures.push(format!("expected 1 custom event, got {}", customs.len())),
    }
    failures
}

/// What the example's environment changes in the inherited one.
#[derive(Debug, PartialEq)]
struct ExampleEnv {
    /// The variables to remove.
    remove: Vec<OsString>,
    /// The variables to set, after the removal.
    set: Vec<(&'static str, OsString)>,
}

impl ExampleEnv {
    fn apply(&self, command: &mut Command) {
        for name in &self.remove {
            command.env_remove(name);
        }
        for (name, value) in &self.set {
            command.env(name, value);
        }
    }
}

/// The environment of an example run, from the names of the `inherited` variables: every
/// `AGENTCAT_*` and `MCPCAT_*` variable removed (any case: a developer's value must not turn the
/// SDK off or point it elsewhere), then `AGENTCAT_API_URL` (`api_url`), `AGENTCAT_PROJECT_ID`,
/// `HOME` and `USERPROFILE` (`home`, so the SDK does not write the real `~/agentcat.log`) set, and
/// `PORT` set for an HTTP example (`port`) or removed.
///
/// # Errors
///
/// `api_url` is not a loopback URL: the example must not start.
fn example_env(
    inherited: impl IntoIterator<Item = OsString>,
    api_url: &str,
    home: &Path,
    port: Option<u16>,
) -> Result<ExampleEnv, String> {
    if !is_loopback_url(api_url) {
        return Err(format!(
            "refusing to start the example: AGENTCAT_API_URL {api_url:?} is not a loopback URL"
        ));
    }
    let mut remove: Vec<OsString> = inherited
        .into_iter()
        .filter(|name| {
            let name = name.to_string_lossy().to_ascii_uppercase();
            name.starts_with("AGENTCAT_") || name.starts_with("MCPCAT_")
        })
        .collect();
    let mut set = vec![
        ("HOME", home.as_os_str().to_owned()),
        ("USERPROFILE", home.as_os_str().to_owned()),
        ("AGENTCAT_API_URL", OsString::from(api_url)),
        ("AGENTCAT_PROJECT_ID", OsString::from(PROJECT_ID)),
    ];
    match port {
        Some(port) => set.push(("PORT", OsString::from(port.to_string()))),
        None => remove.push(OsString::from("PORT")),
    }
    Ok(ExampleEnv { remove, set })
}

/// Whether `url` is an `http://` URL whose host is a loopback address: `localhost`, an address in
/// 127.0.0.0/8, or `[::1]`. A URL with user information (`user@host`) does not count.
fn is_loopback_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.contains('@') {
        return false;
    }
    let host = if let Some(bracketed) = authority.strip_prefix('[') {
        match bracketed.split_once(']') {
            Some((host, port)) if port.is_empty() || port.starts_with(':') => host,
            _ => return false,
        }
    } else {
        authority.split(':').next().unwrap_or_default()
    };
    host.eq_ignore_ascii_case("localhost")
        || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// A temporary home directory, removed when dropped.
#[derive(Debug)]
struct TempHome(PathBuf);

impl TempHome {
    fn new() -> Result<Self, String> {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos());
        let path =
            std::env::temp_dir().join(format!("examples-check-{}-{nanos}", std::process::id()));
        std::fs::create_dir(&path).map_err(|e| format!("cannot create {}: {e}", path.display()))?;
        Ok(Self(path))
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A request the mock received.
#[derive(Debug, Clone)]
struct Recorded {
    method: Method,
    path: String,
    /// When the mock had the whole body.
    received: Instant,
    /// The body parsed as JSON; the text itself when it is not JSON; `null` when it is empty.
    body: Value,
    /// The body as text.
    raw: String,
}

impl Recorded {
    fn is_publish(&self) -> bool {
        self.method == Method::POST && self.path == PUBLISH_PATH
    }
}

/// The requests the mock received, in order.
type Requests = Arc<Mutex<Vec<Recorded>>>;

/// The mock of the `AgentCat` API: on 127.0.0.1, it answers a publish (`POST /v1/public/events/`)
/// with `202 {"status":"accepted"}` and any other request with 404, and records every request.
#[derive(Debug)]
struct MockApi {
    /// The base URL, `http://127.0.0.1:<port>`.
    url: String,
    requests: Requests,
    server: tokio::task::JoinHandle<()>,
}

impl MockApi {
    async fn start() -> Result<Self, String> {
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

    fn requests(&self) -> Vec<Recorded> {
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

#[cfg(test)]
mod tests;

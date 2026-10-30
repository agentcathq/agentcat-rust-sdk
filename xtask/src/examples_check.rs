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
mod client;
mod judge;
mod mock;
mod sandbox;

use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use serde_json::{Map, Value, json};
use tokio::io::AsyncReadExt;
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use client::{Client, Connection, read_stdout};
use judge::{judge_call, judge_hooks, judge_publishes, judge_tools};
use mock::MockApi;
use sandbox::{ExampleEnv, TempHome, example_env};

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

/// The `context` argument of every call.
const CONTEXT: &str = "Checking the example";

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

#[cfg(test)]
mod tests;

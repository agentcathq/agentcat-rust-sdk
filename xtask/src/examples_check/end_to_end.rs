//! End-to-end tests of [`check`]: the real process plumbing (the pipes, the exit budget and the
//! kill, the stdout reader, the HTTP and server-sent-event exchange, the SIGTERM) against stubs
//! that stand in for an example.
//!
//! A stdio stub is a bash script that answers the requests in their order and publishes each tool
//! call to the mock API before it answers. An HTTP stub is a bash script that hands its `PORT`,
//! its API URL and its project to the test in a file and waits for the SIGTERM; the test itself
//! serves MCP on that port. Every request goes to 127.0.0.1: the stubs and the test refuse any
//! other API host.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, Mutex, PoisonError};

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};

use super::mock::PUBLISH_PATH;
use super::sandbox::is_loopback_url;
use super::*;

const STDIO: Example = EXAMPLES[0];
const HTTP: Example = EXAMPLES[1];

/// A budget short enough for a test, long enough for a stub that answers at once.
const SHORT: Duration = Duration::from_millis(300);

/// The stdio stub; `@MODE@` is `pass`, `noise` (a line that is not JSON-RPC first), `late` (it
/// does not exit after the stdin close) or `held` (a child process keeps its stdout open after its
/// exit).
const STDIO_STUB: &str = r#"#!/usr/bin/env bash
set -u
export LC_ALL=C
mode=@MODE@
hostport="${AGENTCAT_API_URL#http://}"
host="${hostport%%:*}"
port="${hostport##*:}"
if [ "$host" != 127.0.0.1 ]; then
  echo "the stub publishes to 127.0.0.1 only, not $host" >&2
  exit 9
fi
publish() {
  local body="{\"project_id\":\"$AGENTCAT_PROJECT_ID\",\"event_type\":\"mcp:tools/call\",\"resource_name\":\"$1\"}"
  exec 3<>"/dev/tcp/$host/$port"
  printf 'POST %s HTTP/1.1\r\nHost: %s\r\nContent-Type: application/json\r\nContent-Length: %s\r\nConnection: close\r\n\r\n%s' \
    @PUBLISH_PATH@ "$hostport" "${#body}" "$body" >&3
  cat <&3 >/dev/null
  exec 3<&-
}
answer() {
  printf '{"jsonrpc":"2.0","id":%s,"result":%s}\n' "$1" "$2"
}
texts() {
  printf '{"content":[%s],"isError":false}' "$1"
}
if [ "$mode" = noise ]; then
  echo "hello from the stub"
fi
id_pattern='"id":([0-9]+)'
calls=0
while IFS= read -r line; do
  if ! [[ $line =~ $id_pattern ]]; then
    continue
  fi
  id="${BASH_REMATCH[1]}"
  case "$line" in
    *'"method":"initialize"'*)
      answer "$id" '{"protocolVersion":"2025-11-25","capabilities":{"tools":{}},"serverInfo":{"name":"stub","version":"1.0.0"}}'
      ;;
    *'"method":"tools/list"'*)
      answer "$id" '{"tools":[{"name":"add","inputSchema":{"type":"object"}},{"name":"echo","inputSchema":{"type":"object"}}]}'
      ;;
    *'"method":"tools/call"'*)
      calls=$((calls + 1))
      case "$calls" in
        1)
          publish add
          answer "$id" "$(texts '{"type":"text","text":"session_id: ses_stub1"},{"type":"text","text":"5"}')"
          ;;
        2)
          publish echo
          answer "$id" "$(texts '{"type":"text","text":"hello"}')"
          ;;
        *)
          publish add
          answer "$id" "$(texts '{"type":"text","text":"9"}')"
          ;;
      esac
      ;;
  esac
done
case "$mode" in
  late) exec sleep 30 ;;
  held) sleep 5 2>/dev/null & ;;
esac
exit 0
"#;

/// The HTTP stub; `@DIR@` is the directory of the handshake file, `@MODE@` is `term` (it exits 0
/// on SIGTERM) or `eof` (it ignores SIGTERM and exits on the stdin close).
const HTTP_STUB: &str = r#"#!/usr/bin/env bash
set -u
dir='@DIR@'
printf '%s %s %s\n' "$PORT" "$AGENTCAT_API_URL" "$AGENTCAT_PROJECT_ID" > "$dir/handshake.tmp"
mv "$dir/handshake.tmp" "$dir/handshake"
if [ @MODE@ = eof ]; then
  trap '' TERM
  cat >/dev/null 2>&1
  exit 0
fi
sleep 60 </dev/null >/dev/null 2>&1 &
sleeper=$!
trap 'kill "$sleeper"; exit 0' TERM
wait "$sleeper"
exit 3
"#;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

/// Writes `script` as the executable `stub` in `dir`; returns its path.
fn write_stub(dir: &Path, script: &str) -> PathBuf {
    let stub = dir.join("stub");
    fs::write(&stub, script).unwrap();
    fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
    stub
}

/// Checks the stdio stub in `mode` with the budgets `exit_budget` and `drain_budget`.
fn check_stdio(mode: &str, exit_budget: Duration, drain_budget: Duration) -> Outcome {
    let dir = TempHome::new().unwrap();
    let script = STDIO_STUB
        .replace("@MODE@", mode)
        .replace("@PUBLISH_PATH@", PUBLISH_PATH);
    let stub = write_stub(&dir.0, &script);
    runtime().block_on(check(STDIO, &stub, &dir.0, exit_budget, drain_budget))
}

#[test]
fn a_stdio_example_that_answers_every_call_passes() {
    let outcome = check_stdio("pass", EXIT_BUDGET, DRAIN_BUDGET);
    assert_eq!(outcome.fatal, None, "{}", outcome.stderr);
    assert_eq!(outcome.failures, Vec::<String>::new(), "{}", outcome.stderr);
    assert!(outcome.exit_after.is_some());
}

#[test]
fn a_stdout_line_that_is_not_json_rpc_fails() {
    let outcome = check_stdio("noise", EXIT_BUDGET, DRAIN_BUDGET);
    assert_eq!(outcome.fatal, None, "{}", outcome.stderr);
    assert_eq!(
        outcome.failures,
        [r#"a stdout line is not JSON-RPC: "hello from the stub""#]
    );
}

/// The stub sleeps 30 s after the stdin close: the check ends it at the budget.
#[test]
fn a_late_exit_fails_and_the_example_is_ended() {
    let started = Instant::now();
    let outcome = check_stdio("late", SHORT, DRAIN_BUDGET);
    assert_eq!(outcome.fatal, None, "{}", outcome.stderr);
    assert_eq!(
        outcome.failures,
        ["the example did not exit within 300 ms of the stdin close"]
    );
    assert_eq!(outcome.exit_after, None);
    assert!(started.elapsed() < Duration::from_secs(20));
}

/// The stub exits 0 at once, but a child process (`sleep 5`) keeps its stdout open.
#[test]
fn a_stdout_held_open_after_the_exit_fails() {
    let outcome = check_stdio("held", EXIT_BUDGET, SHORT);
    assert_eq!(outcome.fatal, None, "{}", outcome.stderr);
    assert_eq!(
        outcome.failures,
        ["the example's stdout did not close within 300 ms of its exit"]
    );
    assert!(outcome.exit_after.is_some());
}

/// A POST the HTTP stub server received: its JSON-RPC method and its `mcp-session-id` and
/// `mcp-protocol-version` headers.
type Seen = (String, Option<String>, Option<String>);

/// The state of the HTTP stub server.
#[derive(Debug, Clone)]
struct StubServer {
    http: reqwest::Client,
    /// The publish URL of the mock API.
    publish_url: String,
    project: String,
    seen: Arc<Mutex<Vec<Seen>>>,
}

/// The session ID the HTTP stub server issues.
const STUB_SESSION: &str = "stub-session";

/// The protocol version the HTTP stub server answers `initialize` with.
const PROTOCOL_VERSION_OF_STUB: &str = "2025-11-25";

/// Checks the HTTP stub in `mode` with the exit budget `exit_budget`, while the test serves MCP
/// for it; returns the outcome and the POSTs the server received.
fn check_http(mode: &str, exit_budget: Duration) -> (Outcome, Vec<Seen>) {
    let dir = TempHome::new().unwrap();
    let script = HTTP_STUB
        .replace("@DIR@", dir.0.to_str().unwrap())
        .replace("@MODE@", mode);
    let stub = write_stub(&dir.0, &script);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let outcome = runtime().block_on(async {
        let server = tokio::spawn(serve_after_handshake(dir.0.clone(), Arc::clone(&seen)));
        let outcome = check(HTTP, &stub, &dir.0, exit_budget, DRAIN_BUDGET).await;
        server.abort();
        outcome
    });
    let seen = seen.lock().unwrap_or_else(PoisonError::into_inner).clone();
    (outcome, seen)
}

/// Waits for the handshake file of the HTTP stub in `dir`, then serves MCP on the port it names
/// and publishes each tool call to the API URL it names, which must be a loopback URL.
async fn serve_after_handshake(dir: PathBuf, seen: Arc<Mutex<Vec<Seen>>>) {
    let handshake = dir.join("handshake");
    let text = loop {
        if let Ok(text) = fs::read_to_string(&handshake) {
            break text;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let words: Vec<&str> = text.split_whitespace().collect();
    assert_eq!(words.len(), 3, "a bad handshake: {text:?}");
    let (port, api_url, project) = (words[0], words[1], words[2]);
    assert!(is_loopback_url(api_url), "{api_url}");
    let port: u16 = port.parse().unwrap();
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap();
    let state = StubServer {
        http: reqwest::Client::builder().no_proxy().build().unwrap(),
        publish_url: format!("{api_url}{PUBLISH_PATH}"),
        project: project.to_owned(),
        seen,
    };
    let router = axum::Router::new()
        .route("/mcp", axum::routing::post(answer_mcp))
        .with_state(state);
    axum::serve(listener, router).await.unwrap();
}

/// Answers one JSON-RPC message of the client. `initialize` gets a server-sent-event stream whose
/// first event is a request of the server with the same ID (the client must skip it) and the
/// session ID; `tools/list` gets a JSON body; a tool call is published, then answered with a
/// server-sent-event stream.
async fn answer_mcp(State(stub): State<StubServer>, headers: HeaderMap, body: Bytes) -> Response {
    let message: Value = serde_json::from_slice(&body).unwrap_or_default();
    let method = message["method"].as_str().unwrap_or_default().to_owned();
    let header_text = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };
    stub.seen
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push((
            method.clone(),
            header_text("mcp-session-id"),
            header_text("mcp-protocol-version"),
        ));
    let id = &message["id"];
    match method.as_str() {
        "initialize" => {
            let ping = json!({"jsonrpc": "2.0", "id": id, "method": "ping"});
            let result = json!({
                "protocolVersion": PROTOCOL_VERSION_OF_STUB,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "stub", "version": "1.0.0"},
            });
            let answer = json!({"jsonrpc": "2.0", "id": id, "result": result});
            let mut response = event_stream(&[ping, answer]);
            response
                .headers_mut()
                .insert("mcp-session-id", HeaderValue::from_static(STUB_SESSION));
            response
        }
        "tools/list" => {
            let tools = json!({"tools": [{"name": "add"}, {"name": "echo"}]});
            let answer = json!({"jsonrpc": "2.0", "id": id, "result": tools});
            (
                [(header::CONTENT_TYPE, "application/json")],
                answer.to_string(),
            )
                .into_response()
        }
        "tools/call" => {
            let tool = message["params"]["name"].as_str().unwrap_or_default();
            let arguments = &message["params"]["arguments"];
            let text = if tool == "add" {
                let sum = arguments["a"].as_i64().unwrap_or_default()
                    + arguments["b"].as_i64().unwrap_or_default();
                sum.to_string()
            } else {
                arguments["text"].as_str().unwrap_or_default().to_owned()
            };
            let mut content = Vec::new();
            if arguments.get("session_id").is_none() {
                content.push(json!({"type": "text", "text": "session_id: ses_stub1"}));
            }
            content.push(json!({"type": "text", "text": text}));
            let event = json!({
                "project_id": stub.project,
                "event_type": "mcp:tools/call",
                "resource_name": tool,
            });
            let published = stub
                .http
                .post(&stub.publish_url)
                .body(event.to_string())
                .send()
                .await;
            if published.is_err() {
                return StatusCode::BAD_GATEWAY.into_response();
            }
            let result = json!({"content": content, "isError": false});
            event_stream(&[json!({"jsonrpc": "2.0", "id": id, "result": result})])
        }
        _ => StatusCode::ACCEPTED.into_response(),
    }
}

/// A server-sent-event stream with one event per message, with carriage returns as some servers
/// send them.
fn event_stream(messages: &[Value]) -> Response {
    let mut body = String::new();
    for message in messages {
        body.push_str("event: message\r\ndata: ");
        body.push_str(&message.to_string());
        body.push_str("\r\n\r\n");
    }
    ([(header::CONTENT_TYPE, "text/event-stream")], body).into_response()
}

#[test]
fn an_http_example_with_event_stream_answers_passes() {
    let (outcome, seen) = check_http("term", EXIT_BUDGET);
    assert_eq!(outcome.fatal, None, "{}", outcome.stderr);
    assert_eq!(outcome.failures, Vec::<String>::new(), "{}", outcome.stderr);
    assert!(outcome.exit_after.is_some());
    let session = Some(STUB_SESSION.to_owned());
    let version = Some(PROTOCOL_VERSION_OF_STUB.to_owned());
    let after_initialize = |method: &str| (method.to_owned(), session.clone(), version.clone());
    assert_eq!(
        seen,
        [
            ("initialize".to_owned(), None, None),
            after_initialize("notifications/initialized"),
            after_initialize("tools/list"),
            after_initialize("tools/call"),
            after_initialize("tools/call"),
            after_initialize("tools/call"),
        ]
    );
}

/// The stub ignores SIGTERM and exits when its stdin closes: the check keeps the stdin open until
/// the exit, so only the SIGTERM could end it in time.
#[test]
fn an_http_example_that_waits_for_the_stdin_close_fails() {
    let (outcome, _) = check_http("eof", SHORT);
    assert_eq!(outcome.fatal, None, "{}", outcome.stderr);
    assert_eq!(
        outcome.failures,
        ["the example did not exit within 300 ms of the SIGTERM"]
    );
}

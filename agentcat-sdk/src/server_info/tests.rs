//! The heartbeat from a request to the POST, over in-memory connections: a tracked [`Demo`]
//! server, a client, and a handle whose delivery posts to an [`Api`]. Each test sets the memory
//! of the process first; nextest runs each test in a process of its own.
use std::future::{pending, ready};
use std::pin::pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use futures::future::poll_fn;
use futures::task::noop_waker_ref;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, InitializeRequestParams,
    InitializeResult, ListToolsResult, PaginatedRequestParams, RequestId, ServerCapabilities, Tool,
};
use rmcp::service::{MaybeSendFuture, RequestContext, RoleClient, RunningService};
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde_json::{Value, json};
use tokio::time::Instant;

use super::memory::Trigger;
use super::memory::{self, INTERVAL, LASTING_AGE};
use super::{
    Claim, Report, beside, can_send, contained, for_call, for_listing, not_sent_line, sent_line,
};
use crate::client::AgentCat;
use crate::config::{HookError, Options};
use crate::constants::GET_MORE_TOOLS_NAME;
use crate::event::CallRecord;
use crate::event::assemble::EventJob;
use crate::sink::Sink;
use crate::test_support::json::object;
use crate::test_support::rmcp::{bare_context, connect};
use crate::test_support::server_info::{
    Answer, Api, BLOCK, Demo, Harness, LONG, Mode, PROJECT, snapshot, tool,
};

/// A connected test client.
type Client = RunningService<RoleClient, InitializeRequestParams>;

/// How long a test waits for something that comes at once when the code is right.
const WAIT: Duration = Duration::from_secs(10);

/// The line of a read that its request outlived.
const DROPPED_LINE: &str = "Server info not sent: the request ended before the lists were read";

/// Serves `demo`, tracked by the handle of `harness`, to a new client.
async fn serve(harness: &Harness, demo: Demo) -> Client {
    connect(harness.agentcat.track(demo), "heartbeat-client").await
}

/// Calls the tool `name` with no arguments; the call must give a result.
async fn call(client: &Client, name: &str) -> CallToolResult {
    client
        .call_tool(CallToolRequestParams::new(name.to_owned()))
        .await
        .unwrap_or_else(|error| panic!("{name}: {error}"))
}

/// Calls `alpha` and checks that the call gave its normal result.
async fn call_alpha(client: &Client) {
    let result = call(client, "alpha").await;
    assert_ne!(result.is_error, Some(true));
    assert_eq!(result.structured_content, Some(json!({"tool": "alpha"})));
}

/// Sends a feedback note: a `get_more_tools` call, which the SDK answers.
async fn send_feedback_note(client: &Client) {
    let arguments = object(json!({"context": "I looked for a tool that deletes a file."}));
    let request = CallToolRequestParams::new(GET_MORE_TOOLS_NAME).with_arguments(arguments);
    let result = client.call_tool(request).await.expect("the note is taken");
    assert_ne!(result.is_error, Some(true));
}

/// Sends one `tools/list` request and gives the names of the first page.
async fn list(client: &Client) -> Vec<String> {
    let result = client.list_tools(None).await.expect("the listing works");
    result
        .tools
        .iter()
        .map(|tool| tool.name.to_string())
        .collect()
}

/// The names of the tools of the heartbeat `body`.
fn tool_names(body: &Value) -> Vec<&str> {
    body["features"]["tools"]
        .as_array()
        .expect("the body has tools")
        .iter()
        .map(|tool| tool["name"].as_str().expect("a tool has a name"))
        .collect()
}

/// The keys of the JSON object `value`, in order.
fn keys(value: &Value) -> Vec<&str> {
    value
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect()
}

#[test]
fn the_lines_hold_counts_and_reasons_only() {
    assert_eq!(
        sent_line(3, 120, &[]),
        "Server info sent: 3 tools, 120 bytes"
    );
    assert_eq!(
        sent_line(0, 2, &["resources", "prompts"]),
        "Server info sent: 0 tools, 2 bytes, without the lists that did not work: resources, prompts"
    );
    assert_eq!(not_sent_line("why"), "Server info not sent: why");
}

#[tokio::test]
async fn v1_one_tool_call_sends_the_whole_server() {
    memory::start_aged(LASTING_AGE);
    let harness = Harness::new();
    let demo = Demo::new();
    let listed = Arc::clone(&demo.listed);
    let client = serve(&harness, demo).await;
    call_alpha(&client).await;
    assert_eq!(harness.reports(), 1);

    let heartbeats = harness.heartbeats().await;
    assert_eq!(heartbeats.len(), 1);
    let body = &heartbeats[0];
    assert_eq!(
        keys(body),
        [
            "project_id",
            "server_name",
            "server_version",
            "server_title",
            "instructions",
            "features"
        ]
    );
    assert_eq!(body["project_id"], PROJECT);
    assert_eq!(body["server_name"], "demo-server");
    assert_eq!(body["server_version"], "1.2.3");
    assert_eq!(body["server_title"], "Demo Title");
    assert_eq!(body["instructions"], "Use the demo tools.");
    assert_eq!(
        keys(&body["features"]),
        ["tools", "resources", "resourceTemplates", "prompts"]
    );

    // The tools are what a client gets from tools/list.
    let seen = client.list_all_tools().await.expect("the listing works");
    assert_eq!(
        body["features"]["tools"],
        serde_json::to_value(&seen).unwrap()
    );
    assert_eq!(
        tool_names(body),
        ["alpha", "beta", "gamma", GET_MORE_TOOLS_NAME]
    );
    let beta = &body["features"]["tools"][1];
    assert_eq!(beta["description"], "b".repeat(LONG));
    assert!(beta["inputSchema"]["properties"]["session_id"].is_object());
    assert!(beta["inputSchema"]["properties"]["context"].is_object());
    assert!(beta["outputSchema"]["properties"]["mcp_session"].is_object());

    assert_eq!(body["features"]["resources"].as_array().unwrap().len(), 2);
    assert_eq!(
        body["features"]["resourceTemplates"],
        json!([{
            "uriTemplate": "file:///{name}.txt",
            "name": "files",
            "description": "Any file by name",
        }])
    );
    let prompts = body["features"]["prompts"].as_array().unwrap();
    assert_eq!(prompts.len(), 2);
    assert_eq!(prompts[1]["arguments"][0]["name"], "topic");

    // One read: one call of each list handler; the listing of the client came after it.
    assert_eq!(listed.tools.load(Ordering::SeqCst), 2);
    assert_eq!(listed.resources.load(Ordering::SeqCst), 1);
    assert_eq!(listed.templates.load(Ordering::SeqCst), 1);
    assert_eq!(listed.prompts.load(Ordering::SeqCst), 1);
    let lines = harness.lines(1).await;
    let bytes = serde_json::to_vec(body).unwrap().len();
    assert_eq!(lines, [format!("Server info sent: 4 tools, {bytes} bytes")]);
}

#[tokio::test]
async fn v2_a_paged_tool_list_is_sent_whole() {
    memory::start_aged(LASTING_AGE);
    let harness = Harness::new();
    let client = serve(&harness, Demo::paged()).await;
    call_alpha(&client).await;
    let heartbeats = harness.heartbeats().await;
    assert_eq!(heartbeats.len(), 1);
    assert_eq!(
        tool_names(&heartbeats[0]),
        [
            "alpha",
            "beta",
            GET_MORE_TOOLS_NAME,
            "gamma",
            "delta",
            "epsilon"
        ]
    );
}

#[tokio::test]
async fn v3_a_server_with_tools_only_sends_tools_only() {
    memory::start_aged(LASTING_AGE);
    let harness = Harness::new();
    let client = serve(&harness, Demo::tools_only()).await;
    call_alpha(&client).await;
    let heartbeats = harness.heartbeats().await;
    assert_eq!(heartbeats.len(), 1);
    assert_eq!(keys(&heartbeats[0]["features"]), ["tools"]);
}

#[tokio::test]
async fn v4_a_prompt_list_that_fails_is_left_out_of_the_heartbeat() {
    memory::start_aged(LASTING_AGE);
    let harness = Harness::new();
    let demo = Demo {
        prompts: Some(Mode::Fails),
        ..Demo::new()
    };
    let client = serve(&harness, demo).await;
    call_alpha(&client).await;
    let heartbeats = harness.heartbeats().await;
    assert_eq!(heartbeats.len(), 1);
    assert_eq!(
        keys(&heartbeats[0]["features"]),
        ["tools", "resources", "resourceTemplates"]
    );
    assert_eq!(
        tool_names(&heartbeats[0]),
        ["alpha", "beta", "gamma", GET_MORE_TOOLS_NAME]
    );
    let lines = harness.lines(1).await;
    assert!(
        lines[0].ends_with(", without the lists that did not work: prompts"),
        "{lines:?}"
    );
}

#[tokio::test]
async fn v5_a_tool_list_that_fails_sends_nothing() {
    memory::start_aged(LASTING_AGE);
    let harness = Harness::new();
    let demo = Demo {
        tools: Mode::Fails,
        ..Demo::new()
    };
    let client = serve(&harness, demo).await;
    call_alpha(&client).await;
    assert_eq!(harness.reports(), 1);
    assert_eq!(harness.heartbeats().await.len(), 0);
    assert_eq!(
        harness.lines(1).await,
        ["Server info not sent: the tools list gave a JSON-RPC error"]
    );
    // The failed attempt waits like any other: the next call starts none.
    call_alpha(&client).await;
    assert_eq!(harness.reports(), 1);
}

#[tokio::test]
async fn v6_a_list_whose_cursor_repeats_fails_at_once() {
    let started = std::time::Instant::now();
    memory::start_aged(LASTING_AGE);
    let harness = Harness::new();
    let demo = Demo {
        prompts: Some(Mode::SameCursor),
        ..Demo::new()
    };
    let listed = Arc::clone(&demo.listed);
    let client = serve(&harness, demo).await;
    call_alpha(&client).await;
    let heartbeats = harness.heartbeats().await;
    assert_eq!(heartbeats.len(), 1);
    assert_eq!(
        keys(&heartbeats[0]["features"]),
        ["tools", "resources", "resourceTemplates"]
    );
    assert_eq!(listed.prompts.load(Ordering::SeqCst), 2);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[tokio::test(start_paused = true)]
async fn v7_one_heartbeat_an_hour() {
    memory::start_aged(LASTING_AGE);
    let harness = Harness::new();
    let client = serve(&harness, Demo::new()).await;
    for _ in 0..3 {
        call_alpha(&client).await;
    }
    assert_eq!(harness.reports(), 1);
    assert_eq!(harness.heartbeats().await.len(), 1);

    tokio::time::advance(Duration::from_secs(3599)).await;
    call_alpha(&client).await;
    assert_eq!(list(&client).await.len(), 4);
    send_feedback_note(&client).await;
    assert_eq!(harness.reports(), 1);

    tokio::time::advance(Duration::from_secs(1)).await;
    call_alpha(&client).await;
    assert_eq!(harness.reports(), 2);
    assert_eq!(harness.heartbeats().await.len(), 2);
}

#[tokio::test]
async fn v8_two_server_objects_share_the_memory_of_the_process() {
    memory::start_aged(LASTING_AGE);
    let harness = Harness::new();
    let first = serve(&harness, Demo::new()).await;
    let second = serve(&harness, Demo::new()).await;
    call_alpha(&first).await;
    call_alpha(&second).await;
    assert_eq!(list(&second).await.len(), 4);
    assert_eq!(harness.reports(), 1);

    // A second handle for the same project is the same key.
    let other_handle = Harness::new();
    let third = serve(&other_handle, Demo::new()).await;
    call_alpha(&third).await;
    assert_eq!(other_handle.reports(), 0);

    // Another server name is another key.
    let renamed = Demo {
        name: "other-server",
        ..Demo::new()
    };
    let fourth = serve(&harness, renamed).await;
    call_alpha(&fourth).await;
    assert_eq!(harness.reports(), 2);
    let heartbeats = harness.heartbeats().await;
    let names: Vec<&Value> = heartbeats.iter().map(|body| &body["server_name"]).collect();
    assert_eq!(names, ["demo-server", "other-server"]);

    // Another project is another key.
    let other_project = Harness::with("proj_another", Options::default(), Api::default());
    let fifth = serve(&other_project, Demo::new()).await;
    call_alpha(&fifth).await;
    assert_eq!(other_project.reports(), 1);
}

/// A sink that keeps the heartbeat reports and drops everything else.
#[derive(Default)]
struct Reports(Mutex<Vec<String>>);

impl Reports {
    /// The reports so far: `built`, or the reason of a skipped one.
    fn taken(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

impl Sink for Reports {
    fn log(&self, _message: String) {}

    fn record(&self, _record: CallRecord) {}

    fn event(&self, _job: EventJob) {}

    fn server_info(&self, report: Report) {
        let text = match report {
            Report::Built(_) => "built".to_owned(),
            Report::Skipped(why) => why,
        };
        self.0.lock().unwrap().push(text);
    }
}

/// Builds a handle whose reports go to the sink.
type Handle = fn(Arc<Reports>) -> AgentCat;

/// Sends a tool call, a listing and a feedback note to `demo` tracked by `agentcat`.
async fn every_trigger(agentcat: &AgentCat, demo: Demo) {
    let client = connect(agentcat.track(demo), "heartbeat-client").await;
    call_alpha(&client).await;
    assert_ne!(list(&client).await.len(), 0);
    let request = CallToolRequestParams::new(GET_MORE_TOOLS_NAME)
        .with_arguments(object(json!({"context": "A note."})));
    let _ = client.call_tool(request).await;
}

#[tokio::test]
async fn v9_no_heartbeat_without_a_project_with_tracing_off_or_with_the_sdk_off() {
    memory::start_aged(LASTING_AGE);
    let cases: [(&str, Handle); 3] = [
        ("no project", |sink| {
            AgentCat::with_sink("", Options::default(), sink)
        }),
        ("tracing off", |sink| {
            AgentCat::with_sink(PROJECT, Options::default().enable_tracing(false), sink)
        }),
        ("the SDK off", |sink| {
            AgentCat::disabled_with_sink(PROJECT, Options::default(), sink)
        }),
    ];
    for (name, handle) in cases {
        let sink = Arc::new(Reports::default());
        let agentcat = handle(Arc::clone(&sink));
        assert!(!can_send(agentcat.inner()), "{name}");
        let demo = Demo::new();
        let listed = Arc::clone(&demo.listed);
        every_trigger(&agentcat, demo).await;
        assert_eq!(sink.taken(), [""; 0], "{name}");
        // Only the listing of the client reached the handler.
        assert_eq!(listed.tools.load(Ordering::SeqCst), 1, "{name}");
        assert_eq!(listed.prompts.load(Ordering::SeqCst), 0, "{name}");
    }

    // The same requests on a handle that sends: the memory was ready all along.
    let sink = Arc::new(Reports::default());
    let agentcat = AgentCat::with_sink(PROJECT, Options::default(), sink.clone());
    assert!(can_send(agentcat.inner()));
    every_trigger(&agentcat, Demo::new()).await;
    assert_eq!(sink.taken(), ["built"]);
}

#[tokio::test(start_paused = true)]
async fn v10_the_call_returns_when_it_is_ready_while_a_list_never_returns() {
    memory::start_aged(LASTING_AGE);
    let harness = Harness::new();
    let took = Duration::from_millis(50);
    let demo = Demo {
        prompts: Some(Mode::Hangs),
        call_takes: took,
        ..Demo::new()
    };
    let listed = Arc::clone(&demo.listed);
    let client = serve(&harness, demo).await;

    // Each call ends first, so its read is dropped: the call takes its own time and no more.
    // The first four dropped reads give their attempt back and write no line.
    for _ in 1..5 {
        let started = Instant::now();
        call_alpha(&client).await;
        assert_eq!(started.elapsed(), took);
        assert_eq!(harness.reports(), 0);
    }
    // The fifth keeps its attempt and writes the one line, so the next call starts no read.
    call_alpha(&client).await;
    assert_eq!(harness.reports(), 1);
    let started = Instant::now();
    call_alpha(&client).await;
    assert_eq!(started.elapsed(), took);
    assert_eq!(harness.reports(), 1);
    assert_eq!(listed.prompts.load(Ordering::SeqCst), 5);
    assert_eq!(harness.heartbeats().await.len(), 0);
    assert_eq!(harness.lines(1).await, [DROPPED_LINE]);

    // A listing starts a read each time and returns at once. The twentieth dropped read keeps
    // its attempt and writes the one line.
    tokio::time::advance(INTERVAL).await;
    for listing in 1..=20 {
        let started = Instant::now();
        assert_eq!(list(&client).await.len(), 4);
        assert_eq!(started.elapsed(), Duration::ZERO);
        assert_eq!(harness.reports(), usize::from(listing == 20) + 1);
    }
    assert_eq!(listed.prompts.load(Ordering::SeqCst), 25);
    let started = Instant::now();
    send_feedback_note(&client).await;
    assert_eq!(started.elapsed(), Duration::ZERO);
    call_alpha(&client).await;
    assert_eq!(listed.prompts.load(Ordering::SeqCst), 25);
    assert_eq!(harness.reports(), 2);
    assert_eq!(harness.lines(2).await, [DROPPED_LINE; 2]);
    assert_eq!(harness.heartbeats().await.len(), 0);
}

#[tokio::test(start_paused = true)]
async fn v10_twenty_dropped_feedback_notes_in_a_row_keep_the_attempt() {
    memory::start_aged(LASTING_AGE);
    let harness = Harness::new();
    let demo = Demo {
        prompts: Some(Mode::Hangs),
        ..Demo::new()
    };
    let listed = Arc::clone(&demo.listed);
    let client = serve(&harness, demo).await;
    for _ in 0..21 {
        let started = Instant::now();
        send_feedback_note(&client).await;
        assert_eq!(started.elapsed(), Duration::ZERO);
    }
    assert_eq!(listed.prompts.load(Ordering::SeqCst), 20);
    assert_eq!(harness.reports(), 1);
    assert_eq!(harness.lines(1).await, [DROPPED_LINE]);
}

#[tokio::test(start_paused = true)]
async fn v10_a_call_that_is_ready_at_once_does_not_wait_for_a_tool_list_that_never_returns() {
    memory::start_aged(LASTING_AGE);
    let harness = Harness::new();
    let demo = Demo {
        tools: Mode::Hangs,
        ..Demo::new()
    };
    let client = serve(&harness, demo).await;
    for _ in 0..5 {
        let started = Instant::now();
        call_alpha(&client).await;
        assert_eq!(started.elapsed(), Duration::ZERO);
    }
    assert_eq!(harness.lines(1).await, [DROPPED_LINE]);
}

#[tokio::test(start_paused = true)]
async fn v10_a_long_call_outlives_a_list_that_never_returns() {
    memory::start_aged(LASTING_AGE);
    let harness = Harness::new();
    let took = Duration::from_secs(20);
    let demo = Demo {
        resources: Some(Mode::Hangs),
        call_takes: took,
        ..Demo::new()
    };
    let client = serve(&harness, demo).await;
    let started = Instant::now();
    call_alpha(&client).await;
    assert_eq!(started.elapsed(), took);
    // The read gave the list up after its 5 s and sent the heartbeat without it.
    let heartbeats = harness.heartbeats().await;
    assert_eq!(heartbeats.len(), 1);
    assert_eq!(
        keys(&heartbeats[0]["features"]),
        ["tools", "resourceTemplates", "prompts"]
    );
}

#[tokio::test]
async fn v10_a_list_handler_that_blocks_its_thread_holds_the_request_one_block_at_most() {
    memory::start_aged(LASTING_AGE);
    let harness = Harness::new();
    let demo = Demo {
        tools: Mode::Blocks,
        ..Demo::new()
    };
    let listed = Arc::clone(&demo.listed);
    let client = serve(&harness, demo).await;

    // The call is ready at once; the one poll of the read that runs before its answer returns
    // holds the thread for one block, and no more.
    let started = std::time::Instant::now();
    call_alpha(&client).await;
    let took = started.elapsed();
    assert!(took >= BLOCK, "{took:?}");
    assert!(took < BLOCK * 2, "{took:?}");
    assert_eq!(listed.tools.load(Ordering::SeqCst), 1);

    // The next call starts no read and is not held.
    let started = std::time::Instant::now();
    call_alpha(&client).await;
    let took = started.elapsed();
    assert!(took < BLOCK, "{took:?}");
    assert_eq!(listed.tools.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn v10_a_list_handler_that_panics_costs_only_the_heartbeat() {
    memory::start_aged(LASTING_AGE);
    let harness = Harness::new();
    let demo = Demo {
        tools: Mode::Panics,
        ..Demo::new()
    };
    let client = serve(&harness, demo).await;
    call_alpha(&client).await;
    assert_eq!(harness.heartbeats().await.len(), 0);
    assert_eq!(
        harness.lines(1).await,
        ["Server info not sent: the tools list panicked"]
    );
    call_alpha(&client).await;
}

#[tokio::test]
async fn v10_an_api_that_never_answers_holds_no_call_no_flush_and_no_shutdown() {
    memory::start_aged(LASTING_AGE);
    let harness = Harness::with(
        PROJECT,
        Options::default(),
        Api::answering(&[Answer::Never]),
    );
    let client = serve(&harness, Demo::new()).await;
    call_alpha(&client).await;
    call_alpha(&client).await;
    assert_eq!(harness.reports(), 1);

    let started = std::time::Instant::now();
    assert!(harness.agentcat.flush(WAIT).await);
    assert_eq!(harness.api.events().len(), 2);
    assert_eq!(harness.api.heartbeats().len(), 1);
    assert!(harness.agentcat.shutdown(WAIT).await);
    assert!(started.elapsed() < WAIT);
    assert_eq!(harness.lines(0).await.len(), 0);
}

#[tokio::test]
async fn v10_a_500_ends_the_attempt_with_one_post() {
    memory::start_aged(LASTING_AGE);
    let harness = Harness::with(
        PROJECT,
        Options::default(),
        Api::answering(&[Answer::ServerError]),
    );
    let client = serve(&harness, Demo::new()).await;
    call_alpha(&client).await;
    assert_eq!(harness.heartbeats().await.len(), 1);
    assert_eq!(
        harness.lines(1).await,
        ["Server info not sent: the API answered 500"]
    );
    call_alpha(&client).await;
    assert_eq!(list(&client).await.len(), 4);
    assert!(harness.agentcat.flush(WAIT).await);
    assert_eq!(harness.reports(), 1);
    assert_eq!(harness.api.heartbeats().len(), 1);
}

/// A server whose name, its instructions, a tool and a resource hold `secret`.
fn secret_demo() -> Demo {
    let schema = json!({
        "type": "object",
        "properties": {"text": {"type": "string", "description": "a secret"}},
    });
    let secret_tool = Tool::new("secret_tool", "secret tool", object(schema));
    Demo {
        name: "secret-server",
        instructions: Some("the secret plan"),
        tool_pages: vec![vec![tool("alpha"), secret_tool]],
        resource_items: vec![rmcp::model::Resource::new("file:///secret.txt", "secret")],
        ..Demo::new()
    }
}

#[tokio::test]
async fn v11_the_redaction_hook_runs_on_the_text_of_the_heartbeat() {
    memory::start_aged(LASTING_AGE);
    let options = Options::default().redact_sensitive_information(|text: String| async move {
        Ok(text.replace("secret", "[x]"))
    });
    let harness = Harness::with(PROJECT, options, Api::default());
    let client = serve(&harness, secret_demo()).await;
    call_alpha(&client).await;
    let heartbeats = harness.heartbeats().await;
    assert_eq!(heartbeats.len(), 1, "{:?}", harness.lines(1).await);
    let body = &heartbeats[0];
    assert_eq!(body["instructions"], "the [x] plan");
    assert_eq!(body["server_name"], "secret-server");
    let secret_tool = &body["features"]["tools"][1];
    assert_eq!(secret_tool["name"], "secret_tool");
    assert_eq!(secret_tool["description"], "[x] tool");
    assert_eq!(
        secret_tool["inputSchema"]["properties"]["text"]["description"],
        "a [x]"
    );
    let resource = &body["features"]["resources"][0];
    assert_eq!(resource["uri"], "file:///secret.txt");
    assert_eq!(resource["name"], "secret");
}

#[tokio::test]
async fn v12_a_redaction_hook_that_fails_sends_nothing() {
    memory::start_aged(LASTING_AGE);
    let options = Options::default().redact_sensitive_information(|_text: String| async move {
        Err(HookError::from("no redaction today"))
    });
    let harness = Harness::with(PROJECT, options, Api::default());
    let client = serve(&harness, secret_demo()).await;
    call_alpha(&client).await;
    assert_eq!(harness.heartbeats().await.len(), 0);
    assert_eq!(
        harness.lines(1).await,
        ["Server info not sent: the redaction did not work: the hook gave an error"]
    );
    // The call's own event is dropped by the same hook; nothing left the process.
    assert!(harness.agentcat.flush(WAIT).await);
    assert_eq!(harness.api.events().len(), 0);
}

#[tokio::test(start_paused = true)]
async fn v10_a_redaction_hook_that_hangs_holds_no_call() {
    memory::start_aged(LASTING_AGE);
    let options = Options::default()
        .redact_sensitive_information(|_text: String| pending::<Result<String, HookError>>());
    let harness = Harness::with(PROJECT, options, Api::default());
    let took = Duration::from_millis(50);
    let demo = Demo {
        call_takes: took,
        ..Demo::new()
    };
    let client = serve(&harness, demo).await;
    for _ in 0..3 {
        let started = Instant::now();
        call_alpha(&client).await;
        assert_eq!(started.elapsed(), took);
    }
    // The read ended next to the first call; the hook hangs on the worker's side.
    assert_eq!(harness.reports(), 1);
    assert_eq!(harness.api.heartbeats().len(), 0);
}

#[tokio::test]
async fn v13_a_413_is_followed_by_the_body_without_schemas() {
    memory::start_aged(LASTING_AGE);
    let harness = Harness::with(
        PROJECT,
        Options::default(),
        Api::answering(&[Answer::TooLarge]),
    );
    let client = serve(&harness, Demo::new()).await;
    call_alpha(&client).await;
    let heartbeats = harness.heartbeats().await;
    assert_eq!(heartbeats.len(), 2);
    let full = heartbeats[0]["features"]["tools"].as_array().unwrap();
    let slim = heartbeats[1]["features"]["tools"].as_array().unwrap();
    assert_eq!(full.len(), slim.len());
    for (full, slim) in full.iter().zip(slim) {
        assert!(full["inputSchema"].is_object());
        assert!(slim.get("inputSchema").is_none() && slim.get("outputSchema").is_none());
        for key in ["name", "title", "description"] {
            assert_eq!(full.get(key), slim.get(key), "{key}");
        }
    }

    // 413 two times: two POSTs and no third.
    memory::start_aged(LASTING_AGE);
    let harness = Harness::with(
        PROJECT,
        Options::default(),
        Api::answering(&[Answer::TooLarge, Answer::TooLarge]),
    );
    let client = serve(&harness, Demo::new()).await;
    call_alpha(&client).await;
    assert_eq!(harness.heartbeats().await.len(), 2);
    call_alpha(&client).await;
    assert!(harness.agentcat.flush(WAIT).await);
    assert_eq!(harness.api.heartbeats().len(), 2);
}

#[tokio::test]
async fn v15_the_heartbeat_names_the_server_as_the_event_of_the_call_does() {
    memory::start_aged(LASTING_AGE);
    let harness = Harness::new();
    let client = serve(&harness, Demo::new()).await;
    call_alpha(&client).await;
    let heartbeats = harness.heartbeats().await;
    assert!(harness.agentcat.flush(WAIT).await);
    let events = harness.api.events();
    assert_eq!((heartbeats.len(), events.len()), (1, 1));
    assert_eq!(events[0].server_name.as_deref(), Some("demo-server"));
    assert_eq!(events[0].server_version.as_deref(), Some("1.2.3"));
    assert_eq!(
        heartbeats[0]["server_name"].as_str(),
        events[0].server_name.as_deref()
    );
    assert_eq!(
        heartbeats[0]["server_version"].as_str(),
        events[0].server_version.as_deref()
    );
}

#[tokio::test]
async fn v17_a_new_memory_sends_for_a_listing_and_for_a_feedback_note() {
    memory::forget();
    let harness = Harness::new();
    let client = serve(&harness, Demo::new()).await;
    call_alpha(&client).await;
    assert_eq!(harness.reports(), 0);
    assert_eq!(
        list(&client).await,
        ["alpha", "beta", "gamma", GET_MORE_TOOLS_NAME]
    );
    assert_eq!(harness.reports(), 1);
    assert_eq!(harness.heartbeats().await.len(), 1);
    call_alpha(&client).await;
    send_feedback_note(&client).await;
    assert_eq!(harness.reports(), 1);

    memory::forget();
    send_feedback_note(&client).await;
    assert_eq!(harness.reports(), 2);
    let heartbeats = harness.heartbeats().await;
    assert_eq!(heartbeats.len(), 2);
    assert_eq!(heartbeats[0], heartbeats[1]);
}

#[tokio::test]
async fn v18_without_a_memory_each_listing_sends_and_no_tool_call_does() {
    let harness = Harness::new();
    let client = serve(&harness, Demo::new()).await;
    for round in 1..=3 {
        memory::forget();
        call_alpha(&client).await;
        assert_eq!(harness.reports(), round - 1);
        memory::forget();
        assert_eq!(
            list(&client).await,
            ["alpha", "beta", "gamma", GET_MORE_TOOLS_NAME]
        );
        assert_eq!(harness.reports(), round);
    }
    assert_eq!(harness.heartbeats().await.len(), 3);
    assert!(harness.agentcat.flush(WAIT).await);
    assert_eq!(harness.api.events().len(), 3);
}

#[tokio::test(start_paused = true)]
async fn v19_a_tool_call_sends_once_the_memory_is_60_seconds_old() {
    memory::forget();
    let harness = Harness::new();
    let client = serve(&harness, Demo::new()).await;
    call_alpha(&client).await;
    assert_eq!(harness.reports(), 0);
    tokio::time::advance(Duration::from_secs(59)).await;
    call_alpha(&client).await;
    assert_eq!(harness.reports(), 0);
    tokio::time::advance(Duration::from_secs(1)).await;
    call_alpha(&client).await;
    assert_eq!(harness.reports(), 1);
    assert_eq!(harness.heartbeats().await.len(), 1);
}

#[tokio::test]
async fn a_tool_list_of_300_long_tools_goes_out_whole() {
    memory::start_aged(LASTING_AGE);
    let harness = Harness::new();
    let mut tools = vec![tool("alpha")];
    for index in 0..299 {
        let mut long = tool("long");
        long.name = format!("long_{index}").into();
        long.description = Some("d".repeat(LONG).into());
        tools.push(long);
    }
    let demo = Demo {
        tool_pages: vec![tools],
        ..Demo::tools_only()
    };
    let client = serve(&harness, demo).await;
    call_alpha(&client).await;
    let heartbeats = harness.heartbeats().await;
    assert_eq!(heartbeats.len(), 1);
    let sent = heartbeats[0]["features"]["tools"].as_array().unwrap();
    assert_eq!(sent.len(), 301);
    for long in &sent[1..300] {
        assert_eq!(long["description"].as_str().map(str::len), Some(LONG));
    }
    let bytes = serde_json::to_vec(&heartbeats[0]).unwrap().len();
    assert!(bytes > 600_000, "{bytes}");
    assert_eq!(
        harness.lines(1).await,
        [format!("Server info sent: 301 tools, {bytes} bytes")]
    );
}

/// A server whose list handler and whose tool note the ID of the request they serve.
#[derive(Clone, Default)]
struct Ids {
    listed: Arc<Mutex<Vec<RequestId>>>,
    called: Arc<Mutex<Vec<RequestId>>>,
}

impl ServerHandler for Ids {
    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> + MaybeSendFuture + '_ {
        self.listed.lock().unwrap().push(context.id);
        ready(Ok(ListToolsResult::with_all_items(vec![tool("alpha")])))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        (name == "alpha").then(|| tool("alpha"))
    }

    fn call_tool(
        &self,
        _request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResponse, ErrorData>> + MaybeSendFuture + '_ {
        self.called.lock().unwrap().push(context.id);
        ready(Ok(CallToolResult::structured(json!({})).into()))
    }

    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
    }
}

#[tokio::test]
async fn the_list_handlers_get_the_context_of_the_request_that_started_the_check() {
    memory::start_aged(LASTING_AGE);
    let harness = Harness::new();
    let ids = Ids::default();
    let client = connect(harness.agentcat.track(ids.clone()), "heartbeat-client").await;
    call(&client, "alpha").await;
    assert_eq!(harness.reports(), 1);
    let called = ids.called.lock().unwrap().clone();
    assert_eq!(called.len(), 1);
    assert_eq!(*ids.listed.lock().unwrap(), called);
}

/// A server whose `get_info` panics.
struct NoInfo;

impl ServerHandler for NoInfo {
    fn get_info(&self) -> InitializeResult {
        panic!("no info")
    }
}

#[tokio::test]
async fn a_get_info_that_panics_starts_no_read() {
    memory::start_aged(LASTING_AGE);
    let sink = Arc::new(Reports::default());
    let agentcat = AgentCat::with_sink(PROJECT, Options::default(), sink.clone());
    let tracked = agentcat.track(NoInfo);
    let context = bare_context(None);

    // A listing learns the server name from get_info: without it nothing is due.
    assert!(for_listing(&tracked, &context).is_none());
    assert_eq!(sink.taken(), [""; 0]);
    // A call without a server name has no key.
    assert!(for_call(&tracked, false, None, &context).is_none());
    assert_eq!(sink.taken(), [""; 0]);

    // A call whose event has a server name claims the attempt, and then cannot read the rest.
    let server = rmcp::model::Implementation::new("demo-server", "1.2.3");
    assert!(for_call(&tracked, false, Some(&server), &context).is_none());
    assert_eq!(sink.taken(), ["get_info panicked"]);
    // The failed attempt stays recorded.
    assert!(for_call(&tracked, true, Some(&server), &context).is_none());
    assert_eq!(sink.taken().len(), 1);

    // A listing claims on the name that the call kept, and then cannot read the rest.
    memory::forget();
    assert!(for_listing(&tracked, &context).is_none());
    assert_eq!(sink.taken(), ["get_info panicked", "get_info panicked"]);
    assert!(for_listing(&tracked, &context).is_none());
    assert_eq!(sink.taken().len(), 2);
}

#[tokio::test]
async fn a_feedback_note_and_a_tool_call_are_told_apart() {
    memory::forget();
    let sink = Arc::new(Reports::default());
    let agentcat = AgentCat::with_sink(PROJECT, Options::default(), sink.clone());
    let tracked = agentcat.track(Demo::new());
    let context = bare_context(None);
    let server = tracked.inner().get_info().server_info;
    // In a new memory a tool call starts nothing, and a feedback note does.
    assert!(for_call(&tracked, false, Some(&server), &context).is_none());
    let read = for_call(&tracked, true, Some(&server), &context);
    beside(ready(()), read).await;
    assert_eq!(sink.taken(), ["built"]);
}

#[tokio::test]
async fn a_read_that_is_dropped_unpolled_gives_its_attempt_back() {
    memory::forget();
    let sink = Arc::new(Reports::default());
    let agentcat = AgentCat::with_sink(PROJECT, Options::default(), sink.clone());
    let tracked = agentcat.track(Demo::new());
    let context = bare_context(None);
    drop(for_listing(&tracked, &context).expect("a heartbeat is due"));
    assert_eq!(sink.taken(), [""; 0]);
    let read = for_listing(&tracked, &context);
    assert!(read.is_some());
    beside(ready(()), read).await;
    assert_eq!(sink.taken(), ["built"]);
    assert!(for_listing(&tracked, &context).is_none());
}

#[tokio::test]
async fn a_hundred_listings_on_one_handle_call_get_info_at_most_twice() {
    memory::forget();
    let harness = Harness::new();
    let demo = Demo::new();
    let listed = Arc::clone(&demo.listed);
    let client = serve(&harness, demo.clone()).await;
    let before = listed.infos.load(Ordering::SeqCst);
    for _ in 0..100 {
        assert_eq!(list(&client).await.len(), 4);
    }
    let infos = listed.infos.load(Ordering::SeqCst) - before;
    assert!(infos <= 2, "{infos} calls of get_info");
    assert_eq!(harness.reports(), 1);

    // A server tracked again for each request shares the name that the handle keeps.
    let context = bare_context(None);
    for _ in 0..100 {
        let tracked = harness.agentcat.track(demo.clone());
        assert!(for_listing(&tracked, &context).is_none());
    }
    assert_eq!(listed.infos.load(Ordering::SeqCst) - before, infos);
}

#[tokio::test]
async fn a_listing_claims_on_the_kept_name_and_keeps_the_name_that_get_info_gives() {
    memory::forget();
    let sink = Arc::new(Reports::default());
    let agentcat = AgentCat::with_sink(PROJECT, Options::default(), sink.clone());
    let context = bare_context(None);
    let first = agentcat.track(Demo::new());
    beside(ready(()), for_listing(&first, &context)).await;
    assert_eq!(sink.taken(), ["built"]);

    // The kept name is not due, so a renamed server is not asked for its name.
    let renamed = Demo {
        name: "renamed-server",
        ..Demo::new()
    };
    let listed = Arc::clone(&renamed.listed);
    let second = agentcat.track(renamed);
    assert!(for_listing(&second, &context).is_none());
    assert_eq!(listed.infos.load(Ordering::SeqCst), 0);

    // Once the kept name is due, get_info gives the name of the heartbeat and the name to keep.
    memory::forget();
    let read = for_listing(&second, &context);
    assert_eq!(listed.infos.load(Ordering::SeqCst), 1);
    beside(ready(()), read).await;
    assert_eq!(sink.taken(), ["built", "built"]);
    let kept = agentcat
        .inner()
        .with_registry::<Demo, _>(|registry| registry.server_name().map(str::to_owned));
    assert_eq!(kept.as_deref(), Some("renamed-server"));
    assert!(for_listing(&second, &context).is_some());
}

#[tokio::test]
async fn a_tool_call_keeps_the_name_of_its_event_for_the_listings() {
    memory::start_aged(LASTING_AGE);
    let sink = Arc::new(Reports::default());
    let agentcat = AgentCat::with_sink(PROJECT, Options::default(), sink.clone());
    let demo = Demo::new();
    let listed = Arc::clone(&demo.listed);
    let tracked = agentcat.track(demo);
    let context = bare_context(None);
    let server = rmcp::model::Implementation::new("demo-server", "1.2.3");
    beside(
        ready(()),
        for_call(&tracked, false, Some(&server), &context),
    )
    .await;
    let infos = listed.infos.load(Ordering::SeqCst);
    assert!(for_listing(&tracked, &context).is_none());
    assert_eq!(listed.infos.load(Ordering::SeqCst), infos);
}

/// The panic of a read that stands for a panic of the SDK's own code. It does not run the panic
/// hook, so the test writes nothing to standard error.
fn sdk_panic() -> Report {
    std::panic::resume_unwind(Box::new("a panic of the SDK"))
}

#[tokio::test]
async fn a_panic_of_the_read_keeps_the_attempt() {
    memory::forget();
    let sink = Arc::new(Reports::default());
    let agentcat = AgentCat::with_sink(PROJECT, Options::default(), sink.clone());
    let shared = agentcat.inner();
    let claim = Claim::take(shared, Trigger::Listing, "demo-server").expect("due");
    contained(claim, async { sdk_panic() }).await;
    assert_eq!(sink.taken(), ["the read panicked"]);
    assert!(Claim::take(shared, Trigger::Listing, "demo-server").is_none());
}

/// Whether `line` of standard error comes from the panic hook of the process for a panic of a
/// test server's list handler.
fn from_the_panic_hook(line: &str) -> bool {
    line.is_empty()
        || (line.starts_with("thread '") && line.contains("panicked at"))
        || line == "demo list panic"
        || line.starts_with("note: run with `RUST_BACKTRACE=1`")
}

/// Runs every test of this module whose name starts with `v` (the rows of the test table) in a
/// child process of this test binary, one at a time and with the output of the tests not
/// captured, and checks the output of the process: standard output holds the lines of the test
/// runner only, and standard error the lines of the panic hook for a list handler that panics.
#[test]
fn nothing_of_the_heartbeat_reaches_standard_output() {
    let binary = std::env::current_exe().unwrap();
    let output = std::process::Command::new(binary)
        .args([
            "server_info::tests::v",
            "--test-threads=1",
            "--nocapture",
            "--quiet",
        ])
        .env("RUST_BACKTRACE", "0")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}{stderr}");
    assert!(stdout.contains("test result: ok. "), "{stdout}");
    let runner = |line: &str| {
        line.is_empty()
            || line.starts_with("running ")
            || line.starts_with("test result: ok. ")
            || line.chars().all(|mark| mark == '.')
    };
    let stray: Vec<&str> = stdout.lines().filter(|line| !runner(line)).collect();
    assert_eq!(stray, [""; 0], "standard output: {stdout}");
    let stray: Vec<&str> = stderr
        .lines()
        .filter(|line| !from_the_panic_hook(line))
        .collect();
    assert_eq!(stray, [""; 0], "standard error: {stderr}");
}

/// A future that counts its polls and is ready at poll number `ready_at` (never when 0).
fn counted(polls: &AtomicUsize, ready_at: usize) -> impl Future<Output = usize> + '_ {
    poll_fn(move |_cx| {
        let poll = polls.fetch_add(1, Ordering::SeqCst) + 1;
        if poll == ready_at {
            Poll::Ready(poll)
        } else {
            Poll::Pending
        }
    })
}

/// Polls `future` one time with a waker that does nothing.
fn poll_once<T>(future: std::pin::Pin<&mut impl Future<Output = T>>) -> Poll<T> {
    future.poll(&mut Context::from_waker(noop_waker_ref()))
}

#[test]
fn beside_without_a_read_is_the_request() {
    let polls = AtomicUsize::new(0);
    let mut both = pin!(beside(counted(&polls, 2), None::<std::future::Ready<()>>));
    assert_eq!(poll_once(both.as_mut()), Poll::Pending);
    assert_eq!(poll_once(both.as_mut()), Poll::Ready(2));
}

#[test]
fn beside_polls_the_request_first_and_the_read_at_least_once() {
    // Both ready at once: the read ends in the one poll.
    let (request, read) = (AtomicUsize::new(0), AtomicUsize::new(0));
    let order = Mutex::new(Vec::new());
    let first = async {
        order.lock().unwrap().push("request");
        counted(&request, 1).await
    };
    let second = async {
        order.lock().unwrap().push("read");
        counted(&read, 1).await;
        order.lock().unwrap().push("read ended");
    };
    {
        let mut both = pin!(beside(first, Some(second)));
        assert_eq!(poll_once(both.as_mut()), Poll::Ready(1));
    }
    assert_eq!(*order.lock().unwrap(), ["request", "read", "read ended"]);

    // The request ready at once and a read that waits: one poll of the read, then it is dropped.
    let (request, read) = (AtomicUsize::new(0), AtomicUsize::new(0));
    let mut both = pin!(beside(
        counted(&request, 1),
        Some(async {
            counted(&read, 0).await;
        })
    ));
    assert_eq!(poll_once(both.as_mut()), Poll::Ready(1));
    assert_eq!(read.load(Ordering::SeqCst), 1);
}

#[test]
fn beside_polls_the_read_while_the_request_waits_and_not_when_it_is_ready() {
    let (request, read) = (AtomicUsize::new(0), AtomicUsize::new(0));
    let mut both = pin!(beside(
        counted(&request, 3),
        Some(async {
            counted(&read, 0).await;
        })
    ));
    assert_eq!(poll_once(both.as_mut()), Poll::Pending);
    assert_eq!(read.load(Ordering::SeqCst), 1);
    assert_eq!(poll_once(both.as_mut()), Poll::Pending);
    assert_eq!(read.load(Ordering::SeqCst), 2);
    // The request is ready in the third poll: its output returns without one more poll of the
    // read.
    assert_eq!(poll_once(both.as_mut()), Poll::Ready(3));
    assert_eq!(read.load(Ordering::SeqCst), 2);
}

#[test]
fn beside_does_not_poll_a_read_that_ended() {
    let (request, read) = (AtomicUsize::new(0), AtomicUsize::new(0));
    let mut both = pin!(beside(
        counted(&request, 4),
        Some(async {
            counted(&read, 2).await;
        })
    ));
    for _ in 0..3 {
        assert_eq!(poll_once(both.as_mut()), Poll::Pending);
    }
    assert_eq!(poll_once(both.as_mut()), Poll::Ready(4));
    assert_eq!(read.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn the_worker_sends_one_heartbeat_at_a_time_and_keeps_at_most_8_waiting() {
    let harness = Harness::with(
        PROJECT,
        Options::default(),
        Api::answering(&[Answer::Never]),
    );
    let sink = &harness.agentcat.inner().sink;
    sink.server_info(Report::Built(Box::new(snapshot())));
    // The worker takes the first one, whose POST never ends.
    let api = harness.api.clone();
    tokio::task::spawn_blocking(move || {
        let started = std::time::Instant::now();
        while api.heartbeats().is_empty() && started.elapsed() < WAIT {
            std::thread::sleep(Duration::from_millis(2));
        }
    })
    .await
    .unwrap();
    assert_eq!(harness.api.heartbeats().len(), 1);

    for _ in 0..8 {
        sink.server_info(Report::Built(Box::new(snapshot())));
    }
    assert!(harness.agentcat.flush(WAIT).await);
    assert_eq!(harness.lines(0).await.len(), 0);
    sink.server_info(Report::Built(Box::new(snapshot())));
    assert_eq!(
        harness.lines(1).await,
        ["Server info not sent: too many heartbeats wait for the delivery thread"]
    );
    // The eight that wait were not sent beside the first.
    assert!(harness.agentcat.flush(WAIT).await);
    assert_eq!(harness.api.heartbeats().len(), 1);

    // A shutdown drops the ones that wait and aborts the one in flight; later ones are dropped.
    assert!(harness.agentcat.shutdown(WAIT).await);
    sink.server_info(Report::Built(Box::new(snapshot())));
    assert!(harness.agentcat.flush(WAIT).await);
    assert_eq!(harness.api.heartbeats().len(), 1);
    assert_eq!(harness.lines(1).await.len(), 1);
}

#[tokio::test]
async fn the_worker_sends_the_next_heartbeat_when_one_ends() {
    let harness = Harness::new();
    let sink = &harness.agentcat.inner().sink;
    for _ in 0..3 {
        sink.server_info(Report::Built(Box::new(snapshot())));
    }
    sink.server_info(Report::Skipped("a reason".to_owned()));
    let lines = harness.lines(4).await;
    assert_eq!(harness.api.heartbeats().len(), 3);
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.starts_with("Server info sent: 4 tools, "))
            .count(),
        3
    );
    assert!(lines.contains(&"Server info not sent: a reason".to_owned()));
}

#[tokio::test]
async fn a_panic_in_the_send_of_a_heartbeat_leaves_the_worker_running() {
    memory::forget();
    let harness = Harness::with(
        PROJECT,
        Options::default(),
        Api::answering(&[Answer::Panics]),
    );
    let sink = &harness.agentcat.inner().sink;
    sink.server_info(Report::Built(Box::new(snapshot())));
    sink.server_info(Report::Built(Box::new(snapshot())));
    // The first send panicked in its task and wrote no line; the worker sent the second one.
    let lines = harness.lines(1).await;
    assert_eq!(lines.len(), 1);
    assert!(
        lines[0].starts_with("Server info sent: 4 tools, "),
        "{lines:?}"
    );
    assert_eq!(harness.api.heartbeats().len(), 2);

    // Events still go out, and a flush still gets its answer.
    let client = serve(&harness, Demo::new()).await;
    call_alpha(&client).await;
    assert!(harness.agentcat.flush(WAIT).await);
    assert_eq!(harness.api.events().len(), 1);
}

#[tokio::test]
async fn a_line_never_holds_the_message_of_an_error_of_the_customers_code() {
    let secret = "customer-data-in-a-message";
    let options =
        Options::default().redact_sensitive_information(move |_text: String| async move {
            Err(HookError::from(secret))
        });
    let mut lines = Vec::new();
    // A list that gives an error, a list that panics, and a hook that gives an error.
    for (tools, options) in [
        (Mode::Fails, Options::default()),
        (Mode::Panics, Options::default()),
        (Mode::Works, options),
    ] {
        memory::start_aged(LASTING_AGE);
        let harness = Harness::with(PROJECT, options, Api::default());
        let demo = Demo {
            tools,
            ..Demo::new()
        };
        let client = serve(&harness, demo).await;
        call_alpha(&client).await;
        lines.extend(harness.lines(1).await);
    }
    assert_eq!(
        lines,
        [
            "Server info not sent: the tools list gave a JSON-RPC error",
            "Server info not sent: the tools list panicked",
            "Server info not sent: the redaction did not work: the hook gave an error",
        ]
    );
    for message in ["demo list failure", "demo list panic", secret] {
        assert!(
            lines.iter().all(|line| !line.contains(message)),
            "{message}"
        );
    }
}

#[cfg(feature = "test-util")]
#[tokio::test]
async fn a_handle_of_the_test_fake_starts_no_heartbeat() {
    memory::start_aged(LASTING_AGE);
    let fake = crate::testing::Fake::new();
    let agentcat = fake.agentcat(PROJECT, Options::default());
    assert!(!can_send(agentcat.inner()));
    let demo = Demo::new();
    let listed = Arc::clone(&demo.listed);
    every_trigger(&agentcat, demo).await;
    fake.assert_tracked("alpha").await;
    // Only the listing of the client reached the handler.
    assert_eq!(listed.tools.load(Ordering::SeqCst), 1);
    assert_eq!(listed.resources.load(Ordering::SeqCst), 0);
    assert_eq!(listed.prompts.load(Ordering::SeqCst), 0);
}

//! `testing::Fake` records what a tracked server publishes, with no network.
#![expect(clippy::expect_used, reason = "test helpers")]

use agentcat_sdk::testing::Fake;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use agentcat_sdk::{CustomEvent, HookError, Options, ToolCall, UserIdentity};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolRequestParams, ClientCapabilities, Implementation, InitializeRequestParams,
};
use rmcp::service::{RoleClient, RunningService};
use rmcp::{ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize, JsonSchema)]
struct EchoArgs {
    msg: String,
    /// How long the tool takes before it answers, in milliseconds: 0 when absent.
    #[serde(default)]
    delay_ms: u64,
}

#[derive(Clone)]
struct Echo;

#[tool_router]
impl Echo {
    /// Answers `msg` after `delay_ms` milliseconds.
    #[tool(description = "Echoes msg")]
    async fn echo(&self, Parameters(args): Parameters<EchoArgs>) -> String {
        tokio::time::sleep(Duration::from_millis(args.delay_ms)).await;
        args.msg
    }
}

#[expect(
    clippy::unused_async_trait_impl,
    reason = "the methods #[tool_handler] generates are async without an await"
)]
#[tool_handler]
impl ServerHandler for Echo {}

type Client = RunningService<RoleClient, InitializeRequestParams>;

async fn connect(server: impl ServerHandler) -> Client {
    let (server_end, client_end) = tokio::io::duplex(64 * 1024);
    tokio::spawn(async move {
        if let Ok(running) = server.serve(server_end).await {
            let _ = running.waiting().await;
        }
    });
    InitializeRequestParams::new(
        ClientCapabilities::default(),
        Implementation::new("test", "1.0.0"),
    )
    .serve(client_end)
    .await
    .expect("the client connects")
}

async fn echo(client: &Client, msg: &str) {
    let Value::Object(arguments) = json!({"msg": msg}) else {
        unreachable!()
    };
    client
        .call_tool(CallToolRequestParams::new("echo").with_arguments(arguments))
        .await
        .expect("the call succeeds");
}

#[tokio::test]
async fn fake_records_tool_calls() {
    let fake = Fake::new();
    let client = connect(fake.agentcat("proj_1", Options::default()).track(Echo)).await;
    echo(&client, "hi").await;
    fake.assert_tracked("echo").await;
    fake.assert_tracked_count(1).await;
    let events = fake.events().await;
    assert_eq!(events[0].event_type, "mcp:tools/call");
    assert_eq!(events[0].project_id, "proj_1");
}

/// A telemetry-only handle (an empty project ID) publishes nothing, and the fake still records its
/// events, with the empty project.
#[tokio::test]
async fn telemetry_only_events_are_recorded_by_fake() {
    let fake = Fake::new();
    let client = connect(fake.agentcat("", Options::default()).track(Echo)).await;
    echo(&client, "hi").await;
    fake.assert_tracked("echo").await;
    let events = fake.events().await;
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0].event_type, "mcp:tools/call");
    assert_eq!(events[0].project_id, "");
    assert_eq!(events[0].resource_name.as_deref(), Some("echo"));
}

#[tokio::test]
async fn fake_is_instance_scoped() {
    let (one, two) = (Fake::new(), Fake::new());
    let client = connect(one.agentcat("proj_1", Options::default()).track(Echo)).await;
    echo(&client, "hi").await;
    one.assert_tracked_count(1).await;
    two.assert_nothing_tracked().await;
}

#[tokio::test]
#[should_panic(expected = "mcp:tools/call echo")]
async fn assert_tracked_count_panics_with_listing() {
    let fake = Fake::new();
    let client = connect(fake.agentcat("proj_1", Options::default()).track(Echo)).await;
    echo(&client, "hi").await;
    fake.assert_tracked_count(2).await;
}

/// A failing `assert_tracked` names the tool it looked for and lists what was recorded.
#[tokio::test]
#[should_panic(
    expected = "expected a tracked call of tool `missing`, recorded: [mcp:tools/call echo]"
)]
async fn failing_assert_tracked_panics_with_tool_name() {
    let fake = Fake::new();
    let client = connect(fake.agentcat("proj_1", Options::default()).track(Echo)).await;
    echo(&client, "hi").await;
    fake.assert_tracked("missing").await;
}

/// A failing `assert_custom_event_published` names the event it looked for and lists what was
/// recorded; a tool call with that resource name is not a custom event.
#[tokio::test]
#[should_panic(expected = "expected a custom event `echo`, recorded: [mcp:tools/call echo]")]
async fn failing_assert_custom_event_published_names_it() {
    let fake = Fake::new();
    let client = connect(fake.agentcat("proj_1", Options::default()).track(Echo)).await;
    echo(&client, "hi").await;
    fake.assert_custom_event_published("echo").await;
}

/// A failing assertion says when a handle did not finish delivering. The call's event waits for an
/// `identify` hook that never settles; a shutdown 20 ms in, with a 50 ms timeout, abandons it while
/// the assertion's flush waits, so that flush answers `false`.
#[tokio::test]
#[should_panic(expected = "recorded: [] (a handle did not finish delivering within the 5 s flush")]
async fn failing_assertion_notes_an_unfinished_delivery() {
    let fake = Fake::new();
    let options = Options::default()
        .identify(|_call| std::future::pending::<Result<Option<UserIdentity>, HookError>>());
    let agentcat = fake.agentcat("proj_1", options);
    let client = connect(agentcat.track(Echo)).await;
    echo(&client, "hi").await;
    let shutdown = async {
        tokio::time::sleep(Duration::from_millis(20)).await;
        agentcat.shutdown(Duration::from_millis(50)).await
    };
    let ((), _) = tokio::join!(fake.assert_tracked("echo"), shutdown);
}

#[tokio::test]
#[should_panic(expected = "nothing was tracked")]
async fn assert_nothing_tracked_panics_when_something_was() {
    let fake = Fake::new();
    let client = connect(fake.agentcat("proj_1", Options::default()).track(Echo)).await;
    echo(&client, "hi").await;
    fake.assert_nothing_tracked().await;
}

#[tokio::test]
async fn fake_records_custom_events() {
    let fake = Fake::new();
    let agentcat = fake.agentcat("proj_1", Options::default());
    let event = CustomEvent::new().resource_name("cache-warmed");
    agentcat
        .publish_custom_event("ses_1", "proj_1", event)
        .expect("queued");
    fake.assert_custom_event_published("cache-warmed").await;
    fake.assert_tracked_count(0).await;
}

#[tokio::test]
async fn fake_runs_hooks_and_redaction() {
    let fake = Fake::new();
    let options = Options::default()
        .identify(|_call| async { Ok::<_, HookError>(Some(UserIdentity::new("user_7"))) })
        .redact_event(|mut event| async move {
            event.response = Some(json!("[redacted]"));
            Ok::<_, HookError>(Some(event))
        });
    let client = connect(fake.agentcat("proj_1", options).track(Echo)).await;
    echo(&client, "secret").await;
    let events = fake.events().await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].identify_actor_given_id.as_deref(), Some("user_7"));
    assert_eq!(events[0].response, Some(json!("[redacted]")));
}

/// The call number in the `msg` of a call of the concurrency test below.
fn call_number(call: &ToolCall) -> u64 {
    call.arguments()
        .and_then(|arguments| arguments.get("msg"))
        .and_then(Value::as_str)
        .and_then(|msg| msg.parse().ok())
        .expect("msg is the call number")
}

/// Concurrent calls ("each event keeps its own `session_id`, `agent_id`, and actor under
/// interleaving"): ten calls at once, each with its own agent, each minting its own session, an
/// `identify` hook that names a user per call and answers in the reverse order of the calls, and a
/// tool that answers in the order of the calls (it waits too), so the hooks and the tools of the
/// calls overlap. Each published event keeps the session, the agent and the actor of its own call.
#[tokio::test]
async fn concurrent_calls_keep_their_own_session_agent_and_actor() {
    const CALLS: u64 = 10;
    let fake = Fake::new();
    let options =
        Options::default()
            .enable_agent_tracking(true)
            .identify(|call: ToolCall| async move {
                let n = call_number(&call);
                // The first call answers last, so the hooks of the calls interleave.
                tokio::time::sleep(Duration::from_millis(20 * (CALLS - n))).await;
                Ok::<_, HookError>(Some(UserIdentity::new(format!("user-{n}"))))
            });
    let client = connect(fake.agentcat("proj_1", options).track(Echo)).await;
    let calls = (0..CALLS).map(|n| {
        let client = &client;
        async move {
            let Value::Object(arguments) = json!({
                "msg": n.to_string(),
                "delay_ms": 10 * n,
                "session_id": "start",
                "agent_id": format!("agent-{n}"),
            }) else {
                unreachable!()
            };
            let result = client
                .call_tool(CallToolRequestParams::new("echo").with_arguments(arguments))
                .await
                .expect("the call succeeds");
            let mint_back = result.content[0]
                .as_text()
                .expect("the mint-back block")
                .text
                .clone();
            let session = mint_back
                .lines()
                .find_map(|line| line.strip_prefix("session_id: "))
                .expect("the minted session")
                .to_owned();
            (n, session)
        }
    });
    let sessions = futures::future::join_all(calls).await;
    assert_eq!(
        sessions
            .iter()
            .map(|(_, session)| session)
            .collect::<BTreeSet<_>>()
            .len(),
        10,
        "ten distinct sessions"
    );

    let events = fake.events().await;
    assert_eq!(events.len(), 10);
    let mut seen = BTreeSet::new();
    for event in events {
        let user = event
            .identify_actor_given_id
            .clone()
            .expect("each call names its user");
        let n: u64 = user
            .strip_prefix("user-")
            .and_then(|n| n.parse().ok())
            .expect("user-<n>");
        let tags = event.tags.clone().expect("the agent tags");
        assert_eq!(
            tags.get("agentcat_agent_id"),
            Some(&format!("agent-{n}")),
            "{event:?}"
        );
        let (_, session) = sessions
            .iter()
            .find(|(sent, _)| *sent == n)
            .expect("call n ran");
        assert_eq!(event.session_id.as_ref(), Some(session), "{event:?}");
        seen.insert(n);
    }
    assert_eq!(seen.len(), 10, "one event per call");
}

/// Identity is re-resolved per request, so a changed identity lands on the next event (v1 and v2):
/// the `identify` hook runs on every call, so when it names a new user on its second call, the
/// second event carries that user. The identities are the fixture's; there is no `identify` event.
#[tokio::test]
async fn identify_re_resolves_per_call() {
    let fake = Fake::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let options = Options::default().identify(move |_call| {
        let n = counter.fetch_add(1, Ordering::SeqCst);
        async move {
            let (id, name, seat) = if n == 0 {
                ("event-user-a", "First", "a")
            } else {
                ("event-user-b", "Second", "b")
            };
            let Value::Object(data) = json!({"seat": seat}) else {
                unreachable!()
            };
            Ok::<_, HookError>(Some(UserIdentity::new(id).user_name(name).user_data(data)))
        }
    });
    let client = connect(fake.agentcat("proj_1", options).track(Echo)).await;
    for msg in ["First call", "Second call"] {
        echo(&client, msg).await;
    }
    assert_eq!(calls.load(Ordering::SeqCst), 2);

    let events = fake.events().await;
    assert_eq!(
        events.len(),
        2,
        "two tool calls, no identify event: {events:?}"
    );
    let expected = [
        ("First call", "event-user-a", "First", "a"),
        ("Second call", "event-user-b", "Second", "b"),
    ];
    for (msg, id, name, seat) in expected {
        let event = events
            .iter()
            .find(|event| {
                let parameters = event.parameters.as_ref().expect("parameters");
                parameters["request"]["params"]["arguments"]["msg"] == msg
            })
            .unwrap_or_else(|| panic!("the event of {msg:?}"));
        assert_eq!(event.event_type, "mcp:tools/call");
        assert_eq!(event.identify_actor_given_id.as_deref(), Some(id), "{msg}");
        assert_eq!(event.identify_actor_name.as_deref(), Some(name), "{msg}");
        let data = event.identify_actor_data.clone().map(Value::Object);
        assert_eq!(data, Some(json!({"seat": seat})), "{msg}");
    }
}

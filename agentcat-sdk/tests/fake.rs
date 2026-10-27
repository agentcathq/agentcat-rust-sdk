//! `testing::Fake` records what a tracked server publishes, with no network (C43, D41).
#![expect(clippy::expect_used, reason = "test helpers")]

use agentcat_sdk::testing::Fake;
use std::collections::BTreeSet;
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
}

#[derive(Clone)]
struct Echo;

#[tool_router]
impl Echo {
    #[tool(description = "Echoes msg")]
    async fn echo(&self, Parameters(args): Parameters<EchoArgs>) -> String {
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

#[tokio::test]
async fn fake_records_telemetry_only_events() {
    let fake = Fake::new();
    let client = connect(fake.agentcat("", Options::default()).track(Echo)).await;
    echo(&client, "hi").await;
    fake.assert_tracked("echo").await;
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

/// TS's concurrency tests ("each event keeps its own `session_id`, `agent_id`, and actor under
/// interleaving", B-029): ten calls at once, each with its own agent, each minting its own
/// session, and an `identify` hook that names a user per call and answers in the reverse order
/// of the calls. Each published event keeps the session, the agent and the actor of its own call.
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
            let Value::Object(arguments) =
                json!({"msg": n.to_string(), "session_id": "start", "agent_id": format!("agent-{n}")})
            else {
                unreachable!()
            };
            let result = client
                .call_tool(CallToolRequestParams::new("echo").with_arguments(arguments))
                .await
                .expect("the call succeeds");
            let mint_back = result.content[0].as_text().expect("the mint-back block").text.clone();
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

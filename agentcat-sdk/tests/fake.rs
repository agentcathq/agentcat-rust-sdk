//! `testing::Fake` records what a tracked server publishes, with no network (C43, D41).
#![expect(clippy::expect_used, reason = "test helpers")]

use agentcat_sdk::testing::Fake;
use agentcat_sdk::{CustomEvent, HookError, Options, UserIdentity};
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

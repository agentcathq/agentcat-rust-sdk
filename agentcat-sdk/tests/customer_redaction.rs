//! A customer's unit tests of a `redact_event` hook, written where a customer writes them: outside
//! the crate, where `Event` is `#[non_exhaustive]` and no struct literal builds one, so
//! `Event::default()` is the way in; and through `testing::Fake`, whose handles run the hook as
//! production does.
#![expect(clippy::expect_used, reason = "test helpers")]

use agentcat_sdk::testing::Fake;
use agentcat_sdk::{Event, HookError, Options};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolRequestParams, ClientCapabilities, Implementation, InitializeRequestParams,
};
use rmcp::service::{RoleClient, RunningService};
use rmcp::{ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

/// The secret the customer's hook removes.
const SECRET: &str = "fake-secret-42";

/// What the hook puts in its place.
const MASK: &str = "[REDACTED]";

/// The customer's redaction: every string in `value` with the secret masked.
fn mask_secret(value: &mut Value) {
    match value {
        Value::String(text) => *text = text.replace(SECRET, MASK),
        Value::Array(items) => items.iter_mut().for_each(mask_secret),
        Value::Object(fields) => fields.values_mut().for_each(mask_secret),
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

/// The body of the customer's hook: masks the secret in the parameters and the response.
fn redact(event: &mut Event) {
    for value in [&mut event.parameters, &mut event.response]
        .into_iter()
        .flatten()
    {
        mask_secret(value);
    }
}

/// A customer builds an `Event` with `Event::default()`, sets the field the hook reads and runs the
/// hook on it directly, with no handle and no server.
#[tokio::test]
async fn customer_builds_event_for_redact_hook() {
    let mut event = Event::default();
    event.parameters = Some(json!({"query": "weather", "api_key": SECRET, "nested": [SECRET]}));
    event.resource_name = Some("lookup".to_owned());

    // The closure the customer passes to `Options::redact_event`.
    let hook = |mut event: Event| async move {
        redact(&mut event);
        Ok::<_, HookError>(Some(event))
    };
    let redacted = hook(event)
        .await
        .expect("the hook succeeds")
        .expect("the hook keeps the event");
    assert_eq!(
        redacted.parameters,
        Some(json!({"query": "weather", "api_key": MASK, "nested": [MASK]}))
    );
    assert_eq!(redacted.resource_name.as_deref(), Some("lookup"));
    assert_eq!(redacted.response, None);
}

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

/// A client connected to `server` over an in-memory duplex.
async fn connect(
    server: impl ServerHandler,
) -> RunningService<RoleClient, InitializeRequestParams> {
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

/// The same hook on a `Fake` handle. A tool call that carries the secret in its argument and its
/// response is recorded with the secret masked in both, and nowhere else in the event.
#[tokio::test]
async fn fake_records_redacted_event() {
    let fake = Fake::new();
    let options = Options::default().redact_event(|mut event: Event| async move {
        redact(&mut event);
        Ok::<_, HookError>(Some(event))
    });
    let client = connect(fake.agentcat("proj_1", options).track(Echo)).await;
    let Value::Object(arguments) = json!({"msg": format!("my key is {SECRET}")}) else {
        unreachable!()
    };
    client
        .call_tool(CallToolRequestParams::new("echo").with_arguments(arguments))
        .await
        .expect("the call succeeds");

    let events = fake.events().await;
    assert_eq!(events.len(), 1, "{events:?}");
    let event = &events[0];
    let masked = format!("my key is {MASK}");
    let parameters = event.parameters.as_ref().expect("the call's parameters");
    assert_eq!(parameters["request"]["params"]["arguments"]["msg"], masked);
    let response = event.response.as_ref().expect("the call's response");
    assert_eq!(response["content"][0]["text"], masked, "{response}");
    let text = format!("{event:?}");
    assert!(!text.contains(SECRET), "{text}");
}

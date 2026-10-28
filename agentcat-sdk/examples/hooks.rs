//! A stdio MCP server that uses every AgentCat hook.
//!
//! The server has three tools: `add`, `echo` and `fail`. The options show:
//!
//! - `identify`: who made the call.
//! - `event_tags` and `event_properties`: extra data on each tool-call event.
//! - `redact_sensitive_information`: removes a secret from every string of an event.
//! - `redact_event`: changes the whole event before it leaves the process.
//!
//! The `echo` tool also publishes a custom event with the call context, and the `fail` tool
//! records its error with `capture_error` before it returns the error.
//!
//! Run it with your project ID:
//!
//! ```text
//! AGENTCAT_PROJECT_ID=proj_... cargo run -p agentcat-sdk --example hooks
//! ```
//!
//! A stdio server talks JSON-RPC on stdout. Print nothing else there. Send your own logs to stderr.
//!
//! Do not clone the `AgentCat` handle into a hook closure. The handle owns the hooks, so a hook
//! that holds the handle is a reference cycle. This example keeps the project ID in a plain
//! `String` instead.
//!
//! # The `session_id` and `context` parameters
//!
//! AgentCat adds two parameters to every tool in `tools/list`: `session_id` and `context`. The
//! `context` parameter asks the agent to say why it calls the tool. The `session_id` parameter ties
//! the calls of one task together. The first call of a task sends `session_id` as `start` or sends
//! none. The server then puts a text block that begins `[session_id issued` before the tool's own
//! result blocks, and the block holds a `ses_` ID. The agent sends that ID in every later call.
//! This is why a client's first `add` shows the ID before the sum. To turn this off, set the
//! `resolve_session_id` option: the hook then picks the session, and the tools get no `session_id`
//! parameter.
#![expect(
    clippy::doc_markdown,
    reason = "AgentCat is the name of the product, not code"
)]
#![expect(
    clippy::unused_async_trait_impl,
    reason = "the methods #[tool_handler] generates are async without an await"
)]

use agentcat_sdk::{
    AgentCat, CallContext, CustomEvent, Event, HookError, Options, ToolCall, UserIdentity,
    capture_error,
};
use rmcp::handler::server::common::Extension;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{Implementation, InitializeResult, ServerCapabilities};
use rmcp::transport::stdio;
use rmcp::{ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value};

/// The arguments of the `add` tool.
#[derive(Deserialize, JsonSchema)]
struct AddArgs {
    /// The first number.
    a: f64,
    /// The second number.
    b: f64,
}

/// The arguments of the `echo` tool.
#[derive(Deserialize, JsonSchema)]
struct EchoArgs {
    /// The text to send back.
    text: String,
}

/// Your MCP server. It holds the project ID for the custom event.
#[derive(Clone)]
struct Demo {
    project_id: String,
}

#[tool_router]
impl Demo {
    #[tool(description = "Adds two numbers")]
    async fn add(&self, Parameters(args): Parameters<AddArgs>) -> String {
        (args.a + args.b).to_string()
    }

    /// Sends the text back, and publishes one custom event for each call. The SDK passes the
    /// `CallContext` of the call to the tool as an `Extension`.
    #[tool(description = "Sends the text back")]
    async fn echo(
        &self,
        Parameters(args): Parameters<EchoArgs>,
        Extension(call): Extension<CallContext>,
    ) -> String {
        let mut event = CustomEvent::new()
            .resource_name("echo-called")
            .message("the echo tool ran");
        // Attach the event to the session of the call.
        if let Some(session) = call.session_id() {
            event = event.session_id(session);
        }
        // Queues the event and returns at once. An event that cannot be queued is not an error
        // for the tool.
        let _ = call.publish_custom_event(&self.project_id, event);
        args.text
    }

    /// Always fails. `capture_error` records the type and the cause chain of the error, which the
    /// text of an `is_error` result would lose.
    #[tool(description = "Always fails")]
    async fn fail(&self) -> Result<String, String> {
        let error = std::io::Error::new(std::io::ErrorKind::NotFound, "the record does not exist");
        capture_error(&error);
        Err(error.to_string())
    }
}

#[tool_handler]
impl ServerHandler for Demo {
    fn get_info(&self) -> InitializeResult {
        InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("agentcat-example-hooks", "1.0.0"))
    }
}

/// A JSON object with one string entry.
fn one_entry(key: &str, value: &str) -> Map<String, Value> {
    Map::from_iter([(key.to_owned(), Value::String(value.to_owned()))])
}

/// The options with every hook. Each hook gets the call (`ToolCall`) or the event, and returns a
/// future. `Ok(None)` means "no change"; an `Err` is logged and skipped.
fn options() -> Options {
    Options::default()
        // Who made the call. Read your own auth data here, for example the headers of the call.
        .identify(|_call: ToolCall| async move {
            Ok::<_, HookError>(Some(UserIdentity::new("demo-user").user_name("Demo User")))
        })
        // Tags are short strings you filter by in the dashboard.
        .event_tags(|_call: ToolCall| async move {
            Ok::<_, HookError>(Some(one_entry("example", "hooks")))
        })
        // Properties are any JSON data you want to keep with the event.
        .event_properties(|_call: ToolCall| async move {
            Ok::<_, HookError>(Some(one_entry("source", "hooks-example")))
        })
        // Runs on every string of the event, for example the arguments and the result.
        .redact_sensitive_information(|text: String| async move {
            Ok::<_, HookError>(text.replace("secret-token", "[REDACTED]"))
        })
        // Runs on the whole event, after the other hooks. Return `Ok(None)` to drop the event.
        .redact_event(|mut event: Event| async move {
            event.properties.get_or_insert_with(Map::new).insert(
                "redacted_by".to_owned(),
                Value::String("example".to_owned()),
            );
            Ok::<_, HookError>(Some(event))
        })
}

/// The project ID for the custom event: `AGENTCAT_PROJECT_ID`, or empty.
#[expect(
    clippy::disallowed_methods,
    reason = "an example reads its project ID from the environment"
)]
fn project_id() -> String {
    std::env::var(agentcat_sdk::AGENTCAT_PROJECT_ID).unwrap_or_default()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let demo = Demo {
        project_id: project_id(),
    };
    let server = AgentCat::from_env(options()).track(demo);
    let running = server.serve(stdio()).await?;
    running.waiting().await?;
    Ok(())
}

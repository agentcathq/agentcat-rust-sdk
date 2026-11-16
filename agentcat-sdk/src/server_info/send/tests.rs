//! The send of a heartbeat, [`deliver`] with an [`Api`] that answers as told: the one line of
//! each attempt, the redaction of the body, and the answers of the API.
use std::future::pending;
use std::time::{Duration, Instant};

use rmcp::model::{Implementation, Resource};
use serde_json::{Value, json};

use super::{BUILD_TIMEOUT, deliver};
use crate::config::{HookError, Options, StringRedactor};
use crate::delivery::Publisher;
use crate::server_info::Snapshot;
use crate::server_info::lists::Lists;
use crate::test_support::server_info::{Answer, Api, PROJECT, snapshot, tool};

/// The string hook of `options`.
fn hook(options: &Options) -> StringRedactor {
    options.string_redactor().cloned().expect("a string hook")
}

/// A string hook that replaces `secret` with `[x]`.
fn replace_secret() -> StringRedactor {
    hook(
        &Options::default().redact_sensitive_information(|text: String| async move {
            Ok(text.replace("secret", "[x]"))
        }),
    )
}

/// Delivers `snapshot` to `api` with `redactor`, and gives the line and the prepare lines.
async fn run(
    api: &Api,
    redactor: Option<&StringRedactor>,
    snapshot: Snapshot,
) -> (String, Vec<String>) {
    let mut log = Vec::new();
    let line = deliver(PROJECT, redactor, api, snapshot, &mut log).await;
    (line, log)
}

/// The size of `body` as JSON text.
fn bytes(body: &Value) -> usize {
    serde_json::to_vec(body).unwrap().len()
}

#[test]
fn the_build_limit_is_30_seconds() {
    assert_eq!(BUILD_TIMEOUT, Duration::from_secs(30));
}

#[tokio::test]
async fn an_accepted_heartbeat_is_one_post_and_one_line_with_its_counts() {
    let api = Api::default();
    let (line, log) = run(&api, None, snapshot()).await;
    let heartbeats = api.heartbeats();
    assert_eq!(heartbeats.len(), 1);
    assert_eq!(heartbeats[0]["project_id"], PROJECT);
    assert_eq!(heartbeats[0]["server_name"], "demo-server");
    assert_eq!(
        line,
        format!("Server info sent: 4 tools, {} bytes", bytes(&heartbeats[0]))
    );
    assert_eq!(log.len(), 0, "{log:?}");
    assert_eq!(api.events().len(), 0);
}

#[tokio::test]
async fn the_line_names_the_lists_that_were_left_out() {
    let api = Api::default();
    let mut snapshot = snapshot();
    snapshot.lists.left_out = vec!["resources", "prompts"];
    snapshot.lists.resources = None;
    let (line, _log) = run(&api, None, snapshot).await;
    let heartbeats = api.heartbeats();
    assert_eq!(
        line,
        format!(
            "Server info sent: 4 tools, {} bytes, without the lists that did not work: resources, prompts",
            bytes(&heartbeats[0])
        )
    );
    assert!(heartbeats[0]["features"]["resources"].is_null());
}

#[tokio::test]
async fn v13_a_413_is_followed_by_one_body_without_schemas() {
    let api = Api::answering(&[Answer::TooLarge, Answer::Accepted]);
    let (line, _log) = run(&api, None, snapshot()).await;
    let heartbeats = api.heartbeats();
    assert_eq!(heartbeats.len(), 2);
    let (full, slim) = (&heartbeats[0], &heartbeats[1]);
    let full_tools = full["features"]["tools"].as_array().unwrap();
    let slim_tools = slim["features"]["tools"].as_array().unwrap();
    assert_eq!(full_tools.len(), slim_tools.len());
    for (full, slim) in full_tools.iter().zip(slim_tools) {
        assert!(full["inputSchema"].is_object());
        assert!(slim.get("inputSchema").is_none() && slim.get("outputSchema").is_none());
        for key in ["name", "title", "description"] {
            assert_eq!(full.get(key), slim.get(key), "{key}");
        }
    }
    assert!(full_tools[1]["outputSchema"].is_object());
    assert_eq!(slim["features"]["resources"], full["features"]["resources"]);
    assert_eq!(slim["instructions"], full["instructions"]);
    assert_eq!(
        line,
        format!("Server info sent: 4 tools, {} bytes", bytes(slim))
    );
}

#[tokio::test]
async fn v13_a_second_413_ends_the_attempt() {
    let api = Api::answering(&[Answer::TooLarge, Answer::TooLarge, Answer::Accepted]);
    let (line, _log) = run(&api, None, snapshot()).await;
    assert_eq!(api.heartbeats().len(), 2);
    assert_eq!(
        line,
        "Server info not sent: the body is too large for the API, also without the tool schemas"
    );
}

#[tokio::test]
async fn v10_any_other_answer_ends_the_attempt_with_one_post() {
    let api = Api::answering(&[Answer::ServerError]);
    let (line, _log) = run(&api, None, snapshot()).await;
    assert_eq!(api.heartbeats().len(), 1);
    assert_eq!(line, "Server info not sent: the API answered 500");

    let api = Api::answering(&[Answer::TooLarge, Answer::ServerError]);
    let (line, _log) = run(&api, None, snapshot()).await;
    assert_eq!(api.heartbeats().len(), 2);
    assert_eq!(line, "Server info not sent: the API answered 500");
}

/// A publisher that can never send.
struct NoClient;

impl Publisher for NoClient {
    fn publish(
        &self,
        _request: agentcat_api::models::PublishEventRequest,
    ) -> crate::sink::BoxFuture<'static, Result<(), String>> {
        Box::pin(pending())
    }

    fn prepare(&self, log: &mut Vec<String>) -> Result<(), String> {
        log.push("no client line".to_owned());
        Err("no client".to_owned())
    }
}

#[tokio::test]
async fn a_publisher_that_cannot_send_gets_no_attempt() {
    let mut log = Vec::new();
    let line = deliver(PROJECT, None, &NoClient, snapshot(), &mut log).await;
    assert_eq!(line, "Server info not sent: no client");
    assert_eq!(log, ["no client line"]);
}

/// A publisher that only handles events.
struct EventsOnly;

impl Publisher for EventsOnly {
    fn publish(
        &self,
        _request: agentcat_api::models::PublishEventRequest,
    ) -> crate::sink::BoxFuture<'static, Result<(), String>> {
        Box::pin(pending())
    }
}

#[tokio::test]
async fn a_publisher_for_events_only_takes_the_heartbeat_and_sends_nothing() {
    let mut log = Vec::new();
    let line = deliver(PROJECT, None, &EventsOnly, snapshot(), &mut log).await;
    assert!(line.starts_with("Server info sent: 4 tools, "), "{line}");
}

/// A snapshot whose server name, instructions, the tool and the resource hold `secret`.
fn secret_snapshot() -> Snapshot {
    let schema = json!({
        "type": "object",
        "properties": {"secret": {"type": "string", "description": "a secret"}},
    });
    let secret_tool = rmcp::model::Tool::new(
        "secret_tool",
        "secret tool",
        schema.as_object().cloned().unwrap(),
    )
    .with_title("The secret title");
    Snapshot {
        server: Implementation::new("secret-server", "secret-1").with_title("A secret title"),
        instructions: Some("the secret plan".to_owned()),
        lists: Lists {
            tool_pages: vec![vec![secret_tool, tool("alpha")]],
            resources: Some(vec![
                Resource::new("file:///secret.txt", "secret name")
                    .with_description("a secret file"),
            ]),
            resource_templates: Some(vec![
                rmcp::model::ResourceTemplate::new("file:///secret/{name}", "secret files")
                    .with_title("secret templates"),
            ]),
            prompts: Some(vec![rmcp::model::Prompt::new(
                "secret_prompt",
                Some("a secret prompt"),
                Some(vec![
                    rmcp::model::PromptArgument::new("secret_argument")
                        .with_description("a secret argument"),
                ]),
            )]),
            left_out: Vec::new(),
        },
        ..snapshot()
    }
}

#[tokio::test]
async fn v11_the_redaction_runs_on_the_text_and_keeps_the_names() {
    let api = Api::default();
    let (line, _log) = run(&api, Some(&replace_secret()), secret_snapshot()).await;
    let heartbeats = api.heartbeats();
    assert_eq!(heartbeats.len(), 1, "{line}");
    let body = &heartbeats[0];
    assert_eq!(body["project_id"], PROJECT);
    assert_eq!(body["server_name"], "secret-server");
    assert_eq!(body["server_version"], "secret-1");
    assert_eq!(body["server_title"], "A [x] title");
    assert_eq!(body["instructions"], "the [x] plan");

    let tool = &body["features"]["tools"][0];
    assert_eq!(tool["name"], "secret_tool");
    assert_eq!(tool["title"], "The [x] title");
    assert_eq!(tool["description"], "[x] tool");
    // A key is never changed, also a property named `secret`.
    let property = &tool["inputSchema"]["properties"]["secret"];
    assert_eq!(property["description"], "a [x]");
    assert_eq!(property["type"], "string");

    let resource = &body["features"]["resources"][0];
    assert_eq!(resource["uri"], "file:///secret.txt");
    assert_eq!(resource["name"], "secret name");
    assert_eq!(resource["description"], "a [x] file");

    let template = &body["features"]["resourceTemplates"][0];
    assert_eq!(template["uriTemplate"], "file:///secret/{name}");
    assert_eq!(template["name"], "secret files");
    assert_eq!(template["title"], "[x] templates");

    let prompt = &body["features"]["prompts"][0];
    assert_eq!(prompt["name"], "secret_prompt");
    assert_eq!(prompt["description"], "a [x] prompt");
    // Only the name of the item itself is kept: the name of an argument is text.
    assert_eq!(prompt["arguments"][0]["name"], "[x]_argument");
    assert_eq!(prompt["arguments"][0]["description"], "a [x] argument");
}

#[tokio::test]
async fn v11_the_smaller_body_after_a_413_is_redacted_too() {
    let api = Api::answering(&[Answer::TooLarge]);
    let _ = run(&api, Some(&replace_secret()), secret_snapshot()).await;
    let heartbeats = api.heartbeats();
    assert_eq!(heartbeats.len(), 2);
    for body in &heartbeats {
        assert!(
            !body["features"]["tools"]
                .to_string()
                .contains("secret tool"),
            "{body}"
        );
        assert_eq!(body["instructions"], "the [x] plan");
    }
}

#[tokio::test]
async fn v12_a_redaction_hook_that_fails_sends_nothing() {
    let fails = hook(
        &Options::default().redact_sensitive_information(|text: String| async move {
            if text.contains("secret tool") {
                Err(HookError::from("no redaction today"))
            } else {
                Ok(text)
            }
        }),
    );
    let api = Api::default();
    let (line, _log) = run(&api, Some(&fails), secret_snapshot()).await;
    assert_eq!(api.heartbeats().len(), 0);
    assert_eq!(
        line,
        "Server info not sent: the redaction did not work: the hook gave an error"
    );
}

#[tokio::test]
async fn v12_a_redaction_hook_that_panics_sends_nothing() {
    let panics = hook(
        &Options::default()
            .redact_sensitive_information(|_text: String| async move { panic!("redaction panic") }),
    );
    let api = Api::default();
    let (line, _log) = run(&api, Some(&panics), secret_snapshot()).await;
    assert_eq!(api.heartbeats().len(), 0);
    assert_eq!(
        line,
        "Server info not sent: the redaction did not work: the hook panicked"
    );
}

#[tokio::test]
async fn a_body_without_a_runtime_for_the_hook_is_not_sent() {
    let api = Api::default();
    let snapshot = Snapshot {
        runtime: None,
        ..secret_snapshot()
    };
    let (line, _log) = run(&api, Some(&replace_secret()), snapshot).await;
    assert_eq!(api.heartbeats().len(), 0);
    assert_eq!(
        line,
        "Server info not sent: the redaction did not work: no tokio runtime"
    );
    // Without a hook, no runtime is needed.
    let snapshot = Snapshot {
        runtime: None,
        ..secret_snapshot()
    };
    let (line, _log) = run(&api, None, snapshot).await;
    assert!(line.starts_with("Server info sent: "), "{line}");
    assert_eq!(api.heartbeats()[0]["instructions"], "the secret plan");
}

#[tokio::test]
async fn v10_a_redaction_hook_that_hangs_ends_at_the_build_limit_and_sends_nothing() {
    let (dropped, signal) = std::sync::mpsc::channel::<()>();
    let hangs = hook(
        &Options::default().redact_sensitive_information(move |_text: String| {
            let dropped = dropped.clone();
            async move {
                let _dropped = dropped;
                pending::<Result<String, HookError>>().await
            }
        }),
    );
    let api = Api::default();
    // The read started 100 ms less than the limit ago, so the hook has 100 ms left.
    let short = Duration::from_millis(100);
    let snapshot = Snapshot {
        started: Instant::now()
            .checked_sub(BUILD_TIMEOUT.saturating_sub(short))
            .expect("the clock is past 30 s"),
        ..secret_snapshot()
    };
    let begun = Instant::now();
    let (line, _log) = run(&api, Some(&hangs), snapshot).await;
    let took = begun.elapsed();
    assert_eq!(api.heartbeats().len(), 0);
    assert_eq!(
        line,
        "Server info not sent: the redaction did not work: it did not end within 30000ms of the start of the read"
    );
    assert!(took >= Duration::from_millis(95), "{took:?}");
    assert!(took < Duration::from_secs(10), "{took:?}");
    // The hook's task is aborted, so its future is dropped.
    drop(hangs);
    let waited = Instant::now();
    while signal.try_recv() != Err(std::sync::mpsc::TryRecvError::Disconnected) {
        assert!(
            waited.elapsed() < Duration::from_secs(5),
            "the task still runs"
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

#[test]
fn a_hook_whose_runtime_is_gone_is_cancelled_and_sends_nothing() {
    let gone = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("a runtime");
    let handle = gone.handle().clone();
    drop(gone);
    let worker = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("a runtime");
    let api = Api::default();
    let snapshot = Snapshot {
        runtime: Some(handle),
        ..secret_snapshot()
    };
    let (line, _log) = worker.block_on(run(&api, Some(&replace_secret()), snapshot));
    assert_eq!(api.heartbeats().len(), 0);
    assert_eq!(
        line,
        "Server info not sent: the redaction did not work: the task of the hook was cancelled"
    );
}

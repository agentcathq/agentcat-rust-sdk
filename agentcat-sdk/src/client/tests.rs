//! `AgentCat`: clones share one handle, `track` wraps a handler with a handle, `Debug` prints the
//! project ID and the options only, `from_env` reads the project ID, `AGENTCAT_ENABLED` turns a
//! handle off, and the lifecycle methods.
use std::sync::Arc;
use std::time::Duration;

use rmcp::ServerHandler;

use super::{AgentCat, NO_PROJECT_WARNING, Transport, track};
use crate::config::Options;
use crate::config::env::{AGENTCAT_ENABLED, AGENTCAT_PROJECT_ID, MapEnv};
use crate::context::CallContext;
use crate::custom::CustomEvent;
use crate::delivery::DeliveryConfig;
use crate::test_support::queue::CapturedLines;
use crate::test_support::rmcp::{Probe, RecordingSink, Sample, connect};

/// A handler that can be told apart from another one.
#[derive(Debug, PartialEq, Eq)]
struct Named(&'static str);

impl ServerHandler for Named {}

#[test]
fn clones_share_one_inner() {
    let sink = Arc::new(RecordingSink::default());
    let agentcat = AgentCat::with_sink("proj_1", Options::default(), sink.clone());
    let clone = agentcat.clone();
    assert!(std::ptr::eq(agentcat.inner(), clone.inner()));

    clone.inner().sink.log("from the clone".to_owned());
    assert_eq!(sink.logs(), ["from the clone"]);

    let tracked = agentcat.track(Probe);
    assert!(std::ptr::eq(agentcat.inner(), tracked.agentcat().inner()));
}

#[test]
fn track_returns_a_wrapper_around_the_handler() {
    let mut tracked = track(
        Named("mine"),
        "proj_1",
        Options::default().custom_context_description("Why."),
    );
    assert_eq!(tracked.inner(), &Named("mine"));
    let handle = tracked.agentcat().inner();
    assert_eq!(handle.project_id, "proj_1");
    assert_eq!(handle.options.context_description(), "Why.");

    tracked.inner_mut().0 = "changed";
    assert_eq!(tracked.into_inner(), Named("changed"));
}

/// The `Debug` output of a handle, and of a `Tracked` or `CallContext` that holds one, has the
/// project ID and the options, not the registries (their tool entries and pipeline state) or the
/// sink. Rust-only (design rule: every public type has `Debug`).
#[test]
fn debug_prints_project_and_options_only() {
    let sink = Arc::new(RecordingSink::default());
    let agentcat = AgentCat::with_sink("proj_1", Options::default(), sink);
    agentcat.inner().with_registry::<Named, _>(|registry| {
        registry
            .state_mut()
            .declared_session_params
            .insert("secret_tool".to_owned())
    });
    let options = format!("{:?}", Options::default());
    let expected = format!("AgentCat {{ project_id: \"proj_1\", options: {options}, .. }}");
    assert_eq!(format!("{agentcat:?}"), expected);
    assert_eq!(
        format!("{:?}", agentcat.inner()),
        expected.replacen("AgentCat", "Inner", 1)
    );

    let tracked = format!("{:?}", agentcat.track(Named("mine")));
    let call = format!(
        "{:?}",
        CallContext::new(String::new(), None, agentcat.clone())
    );
    for text in [tracked, call] {
        assert!(text.contains(&expected), "{text}");
        assert!(
            !text.contains("secret_tool") && !text.contains("Sink"),
            "{text}"
        );
    }
}

/// The transport of a handle that logs to `lines` and sends nothing.
fn offline(lines: &CapturedLines, startup: &[&str]) -> Transport {
    Transport {
        publisher: Arc::new(super::OfflinePublisher),
        startup_lines: startup.iter().map(|&line| line.to_owned()).collect(),
        write: Some(lines.writer()),
    }
}

/// An empty project ID makes the worker write the no-project warning first, then the transport's
/// own lines.
#[test]
fn empty_project_logs_the_warning_first() {
    let lines = CapturedLines::default();
    let agentcat = AgentCat::assemble(
        String::new(),
        Options::default(),
        offline(&lines, &["from the transport"]),
        DeliveryConfig::default(),
    );
    assert!(agentcat.flush_blocking(Duration::from_secs(2)));
    assert_eq!(
        lines.lines(),
        [NO_PROJECT_WARNING, "from the transport"],
        "the warning is the no-project text"
    );
    assert_eq!(
        NO_PROJECT_WARNING,
        "Warning: No project_id provided (AgentCat::from_env reads it from AGENTCAT_PROJECT_ID). Events will not be sent anywhere."
    );
}

/// A project ID gives no warning.
#[test]
fn project_logs_no_warning() {
    let lines = CapturedLines::default();
    let agentcat = AgentCat::assemble(
        "proj_1".to_owned(),
        Options::default(),
        offline(&lines, &[]),
        DeliveryConfig::default(),
    );
    assert!(agentcat.flush_blocking(Duration::from_secs(2)));
    assert_eq!(lines.lines().len(), 0);
}

/// The number of threads of this process named `agentcat-delivery` (Linux truncates names to 15
/// bytes). Under `cargo nextest` each test has its own process, so the count is this test's.
#[cfg(target_os = "linux")]
fn delivery_threads() -> usize {
    std::fs::read_dir("/proc/self/task")
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|task| std::fs::read_to_string(task.path().join("comm")).ok())
        .filter(|name| name.trim() == "agentcat-delive")
        .count()
}

/// Building a handle, tracking a handler and listing start no thread; the first event does.
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn new_starts_no_thread() {
    let agentcat = AgentCat::new("proj_1", Options::default());
    let tracked = agentcat.track(Sample);
    assert_eq!(delivery_threads(), 0);
    let client = connect(tracked, "thread-client").await;
    assert_eq!(delivery_threads(), 0, "a connection is not an event");
    let params = rmcp::model::CallToolRequestParams::new("echo").with_arguments(
        serde_json::json!({"msg": "hi"})
            .as_object()
            .cloned()
            .unwrap(),
    );
    client.call_tool(params).await.unwrap();
    assert!(agentcat.flush(Duration::from_secs(2)).await);
    assert_eq!(delivery_threads(), 1);
    client.cancel().await.unwrap();
}

/// A handle built by `AgentCat::new` after one tool call, the call's runtime dropped.
fn handle_after_one_call() -> AgentCat {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let agentcat = AgentCat::new("proj_1", Options::default());
    runtime.block_on(async {
        let client = connect(agentcat.track(Sample), "lifecycle-client").await;
        let params = rmcp::model::CallToolRequestParams::new("echo").with_arguments(
            serde_json::json!({"msg": "hi"})
                .as_object()
                .cloned()
                .unwrap(),
        );
        client.call_tool(params).await.unwrap();
        client.cancel().await.unwrap();
    });
    agentcat
}

/// `flush` settles the event of a call and returns `true`; with nothing queued it returns
/// `true` at once.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn flush_settles_pending_events() {
    let agentcat = AgentCat::new("proj_1", Options::default());
    assert!(agentcat.flush(Duration::from_secs(1)).await);
    let client = connect(agentcat.track(Sample), "lifecycle-client").await;
    let params = rmcp::model::CallToolRequestParams::new("echo").with_arguments(
        serde_json::json!({"msg": "hi"})
            .as_object()
            .cloned()
            .unwrap(),
    );
    client.call_tool(params).await.unwrap();
    assert!(agentcat.flush(Duration::from_secs(2)).await);
    client.cancel().await.unwrap();
}

/// `shutdown` settles the queue, returns `true`, and is terminal: later calls still return `true`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_settles_and_is_terminal() {
    let agentcat = AgentCat::new("proj_1", Options::default());
    let client = connect(agentcat.track(Sample), "lifecycle-client").await;
    let params = rmcp::model::CallToolRequestParams::new("echo").with_arguments(
        serde_json::json!({"msg": "hi"})
            .as_object()
            .cloned()
            .unwrap(),
    );
    client.call_tool(params.clone()).await.unwrap();
    assert!(agentcat.shutdown(Duration::from_secs(2)).await);
    client.call_tool(params).await.unwrap();
    assert!(agentcat.flush(Duration::from_secs(2)).await);
    assert!(agentcat.shutdown(Duration::from_secs(2)).await);
    client.cancel().await.unwrap();
}

/// `flush_blocking` needs no runtime and returns `true` once the event settled.
#[test]
fn flush_blocking_settles_pending_events() {
    let agentcat = handle_after_one_call();
    assert!(agentcat.flush_blocking(Duration::from_secs(2)));
}

/// `shutdown_blocking` needs no runtime and returns `true`; a second one returns `true` too.
#[test]
fn shutdown_blocking_settles_pending_events() {
    let agentcat = handle_after_one_call();
    assert!(agentcat.shutdown_blocking(Duration::from_secs(2)));
    assert!(agentcat.shutdown_blocking(Duration::from_secs(2)));
}

/// `from_env` takes the project ID from `AGENTCAT_PROJECT_ID`, and the handle is on.
#[test]
fn from_env_reads_project_id() {
    let env = MapEnv::new(&[(AGENTCAT_PROJECT_ID, "proj_from_env")]);
    let agentcat = AgentCat::from_env_in(Options::default(), &env);
    assert_eq!(agentcat.inner().project_id, "proj_from_env");
    assert!(agentcat.inner().enabled);
}

/// Without `AGENTCAT_PROJECT_ID`, or with it empty, `from_env` gives a telemetry-only handle
/// (project `""`); `AGENTCAT_ENABLED` turns it off as it does `new`.
#[test]
fn from_env_without_project_is_telemetry_only() {
    for env in [MapEnv::new(&[]), MapEnv::new(&[(AGENTCAT_PROJECT_ID, "")])] {
        let agentcat = AgentCat::from_env_in(Options::default(), &env);
        assert_eq!(agentcat.inner().project_id, "", "{env:?}");
        assert!(agentcat.inner().enabled, "{env:?}");
    }
    let off = MapEnv::new(&[(AGENTCAT_PROJECT_ID, "proj_1"), (AGENTCAT_ENABLED, " Off ")]);
    let agentcat = AgentCat::from_env_in(Options::default(), &off);
    assert_eq!(agentcat.inner().project_id, "proj_1");
    assert!(!agentcat.inner().enabled);
}

/// A handle that `AGENTCAT_ENABLED` turned off starts no thread: not for a listing, a tool call, a
/// custom event in any of its three forms (which returns `Ok(())`, also for an empty project ID) or
/// the lifecycle methods, which return `true`.
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disabled_handle_starts_no_thread() {
    let env = MapEnv::new(&[(AGENTCAT_ENABLED, "false")]);
    let agentcat = AgentCat::with_env("", Options::default(), &env);
    assert!(!agentcat.inner().enabled);
    let client = connect(agentcat.track(Sample), "disabled-client").await;
    client.list_all_tools().await.unwrap();
    let params = rmcp::model::CallToolRequestParams::new("echo").with_arguments(
        serde_json::json!({"msg": "hi"})
            .as_object()
            .cloned()
            .unwrap(),
    );
    client.call_tool(params).await.unwrap();
    let tracked = agentcat.track(Sample);
    let call = CallContext::new(String::new(), None, agentcat.clone());
    let event = || CustomEvent::new().resource_name("disabled");
    for project in ["", "proj_1"] {
        let string = agentcat.publish_custom_event("ses_1", project, event());
        assert!(
            string.is_ok(),
            "string form, project {project:?}: {string:?}"
        );
        let context = call.publish_custom_event(project, event());
        assert!(
            context.is_ok(),
            "CallContext form, project {project:?}: {context:?}"
        );
        let server = tracked.publish_custom_event(project, event());
        assert!(
            server.is_ok(),
            "Tracked form, project {project:?}: {server:?}"
        );
    }
    assert!(agentcat.flush(Duration::from_secs(1)).await);
    assert!(agentcat.flush_blocking(Duration::from_secs(1)));
    assert_eq!(delivery_threads(), 0);
    assert!(agentcat.shutdown(Duration::from_secs(1)).await);
    assert!(agentcat.shutdown_blocking(Duration::from_secs(1)));
    assert_eq!(delivery_threads(), 0);
    client.cancel().await.unwrap();
}

/// Under `cfg(test)`, `AgentCat::new` and `track` build their handle in an empty environment, so an
/// `AGENTCAT_ENABLED=false` in the developer's shell cannot turn the handles of unit tests off. The
/// test runs [`new_handle_is_on`] in a child process of this test binary with that variable set, as
/// `forbid(unsafe_code)` rules out `std::env::set_var`.
#[test]
fn new_ignores_the_process_environment() {
    let binary = std::env::current_exe().unwrap();
    let output = std::process::Command::new(binary)
        .args([
            "--exact",
            "client::tests::new_handle_is_on",
            "--test-threads=1",
        ])
        .env(AGENTCAT_ENABLED, "false")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}{stderr}");
    assert!(stdout.contains("1 passed"), "the test ran: {stdout}");
}

/// The handles of `AgentCat::new` and `track` are on; [`new_ignores_the_process_environment`]
/// runs this with `AGENTCAT_ENABLED=false` in the process environment.
#[test]
fn new_handle_is_on() {
    assert!(AgentCat::new("", Options::default()).inner().enabled);
    let tracked = track(Named("on"), "", Options::default());
    assert!(tracked.agentcat().inner().enabled);
}

//! `AgentCat`: clones share one handle, `track` wraps a handler with a handle, and `Debug` prints
//! the project ID and the options only.
use std::sync::Arc;

use rmcp::ServerHandler;

use super::{AgentCat, track};
use crate::config::Options;
use crate::context::CallContext;
use crate::test_support::rmcp::{Probe, RecordingSink};

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

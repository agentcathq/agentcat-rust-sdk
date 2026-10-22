//! `AgentCat`: clones share one handle, and `track` wraps a handler with a handle.
use std::sync::Arc;

use rmcp::ServerHandler;

use super::{AgentCat, track};
use crate::config::Options;
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

//! [`Fake`]: a test double that records what a tracked server publishes, so a customer's test can
//! assert on it (C43, D41). Enable the `test-util` feature of the crate to use it.

use std::fmt::Write as _;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use crate::AgentCat;
use crate::config::Options;
use crate::delivery::Observer;
use crate::event::Event;
use crate::pipeline::TsEvent;

/// The event type of a tool call.
const TOOL_CALL: &str = "mcp:tools/call";
/// The event type of a custom event.
const CUSTOM: &str = "agentcat:custom";
/// How long [`Fake::events`] waits for each handle to deliver.
const FLUSH: Duration = Duration::from_secs(5);

/// The events that finished the pipeline of the handles of one [`Fake`], in that order.
#[derive(Debug, Default)]
struct Recorder {
    events: Mutex<Vec<Event>>,
}

impl Observer for Recorder {
    fn finished(&self, event: &TsEvent) {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Event::from_ts(event));
    }
}

/// A test double for `AgentCat`: it builds handles whose events never leave the process and
/// records every event that finishes the pipeline, telemetry-only events (an empty project ID)
/// included. The `identify`, tag, property and `redact_event` hooks run as they do in production,
/// so a recorded event is the one that would have been published.
///
/// A `Fake` is an instance, not a global: two fakes in one process see only their own handles.
/// Clones share the record. Its handles are always on (they ignore `AGENTCAT_ENABLED`), never
/// configure the process logger (so they never write `~/agentcat.log`) and use a publisher that
/// never sends, so a test needs no network.
///
/// ```
/// # async fn run() {
/// use agentcat_sdk::Options;
/// use agentcat_sdk::testing::Fake;
///
/// # struct MyServer;
/// # impl rmcp::ServerHandler for MyServer {}
/// let fake = Fake::new();
/// let server = fake.agentcat("proj_123", Options::default()).track(MyServer);
/// // ... serve `server` and make calls against it ...
/// # let _ = server;
/// fake.assert_nothing_tracked().await;
/// # }
/// ```
///
/// The assertion methods panic with a message that lists the recorded events' types and
/// resource names.
#[derive(Clone, Debug, Default)]
pub struct Fake {
    recorder: Arc<Recorder>,
    handles: Arc<Mutex<Vec<AgentCat>>>,
}

impl Fake {
    /// A fake with no handles and no events.
    pub fn new() -> Self {
        Self::default()
    }

    /// A handle for `project_id` (empty means telemetry only) whose events this fake records and
    /// whose publisher never sends. Wrap a server with [`AgentCat::track`].
    pub fn agentcat(&self, project_id: impl Into<String>, options: Options) -> AgentCat {
        let handle = AgentCat::offline_observed(
            project_id.into(),
            options,
            Arc::clone(&self.recorder) as Arc<dyn Observer>,
        );
        self.handles
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(handle.clone());
        handle
    }

    /// Flushes every handle this fake built, for up to 5 s each, then returns the recorded events
    /// in the order they finished.
    pub async fn events(&self) -> Vec<Event> {
        let handles: Vec<AgentCat> = self
            .handles
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        for handle in handles {
            let _ = handle.flush(FLUSH).await;
        }
        self.recorder
            .events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Asserts that a tool call of the tool `tool_name` was tracked.
    ///
    /// # Panics
    ///
    /// When no recorded tool-call event has that resource name.
    pub async fn assert_tracked(&self, tool_name: &str) {
        let events = self.events().await;
        let found = events
            .iter()
            .any(|e| e.event_type == TOOL_CALL && e.resource_name.as_deref() == Some(tool_name));
        assert!(
            found,
            "expected a tracked call of tool `{tool_name}`, recorded: {}",
            listing(&events)
        );
    }

    /// Asserts that exactly `count` tool calls were tracked (custom events do not count).
    ///
    /// # Panics
    ///
    /// When the number of recorded tool-call events differs.
    pub async fn assert_tracked_count(&self, count: usize) {
        let events = self.events().await;
        let actual = events.iter().filter(|e| e.event_type == TOOL_CALL).count();
        assert!(
            actual == count,
            "expected {count} tracked tool calls, found {actual}, recorded: {}",
            listing(&events)
        );
    }

    /// Asserts that no event of any kind was recorded.
    ///
    /// # Panics
    ///
    /// When any event was recorded.
    pub async fn assert_nothing_tracked(&self) {
        let events = self.events().await;
        assert!(
            events.is_empty(),
            "expected nothing was tracked, recorded: {}",
            listing(&events)
        );
    }

    /// Asserts that a custom event with the resource name `resource_name` was published.
    ///
    /// # Panics
    ///
    /// When no recorded custom event has that resource name.
    pub async fn assert_custom_event_published(&self, resource_name: &str) {
        let events = self.events().await;
        let found = events
            .iter()
            .any(|e| e.event_type == CUSTOM && e.resource_name.as_deref() == Some(resource_name));
        assert!(
            found,
            "expected a custom event `{resource_name}`, recorded: {}",
            listing(&events)
        );
    }
}

/// The types and resource names of `events`, such as `[mcp:tools/call echo, agentcat:custom x]`.
fn listing(events: &[Event]) -> String {
    let mut out = String::from("[");
    for (i, e) in events.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        let _ = write!(
            out,
            "{} {}",
            e.event_type,
            e.resource_name.as_deref().unwrap_or("-")
        );
    }
    out.push(']');
    out
}

//! [`CallContext`]: what a tool can read about its own call while `AgentCat` tracks it.

use std::time::SystemTime;

use rmcp::model::Implementation;

use crate::client::AgentCat;
use crate::custom::{CustomEvent, publish_server};
use crate::error::Error;
use crate::event::ClientIdentity;

/// The tracked call a tool is running in. The SDK puts one into the request context of every tool
/// call it passes to the handler, so a tool can take it as an `Extension<CallContext>` (rmcp's
/// `rmcp::handler::server::common::Extension`) next to its parameters. A call the SDK does not
/// trace (tracing off, or no session ID could be issued) gets one too, with no session and no
/// agent.
///
/// ```no_run
/// use agentcat_sdk::CallContext;
/// use rmcp::handler::server::common::Extension;
///
/// // A parameter of a `#[tool]` method, next to its `Parameters<..>`.
/// fn session_of(Extension(call): Extension<CallContext>) -> String {
///     call.session_id().unwrap_or("no session").to_owned()
/// }
/// # let _ = session_of;
/// ```
#[derive(Clone, Debug)]
pub struct CallContext {
    /// The session of the call; `""` means none.
    session_id: String,
    /// The agent the call names.
    agent_id: Option<String>,
    agentcat: AgentCat,
    /// The client of the call (B-085), for the events the tool publishes.
    client: Option<ClientIdentity>,
    /// The server info of the handler at the start of the call (B-086).
    server: Option<Implementation>,
}

impl CallContext {
    /// The context of a call with the session `session_id` (`""` for none) and the agent
    /// `agent_id`, tracked by `agentcat`.
    pub(crate) fn new(session_id: String, agent_id: Option<String>, agentcat: AgentCat) -> Self {
        Self {
            session_id,
            agent_id,
            agentcat,
            client: None,
            server: None,
        }
    }

    /// This context with the client and the server info of the call. A call the SDK does not
    /// trace has none.
    pub(crate) fn with_identity(
        mut self,
        client: Option<ClientIdentity>,
        server: Option<Implementation>,
    ) -> Self {
        self.client = client;
        self.server = server;
        self
    }

    /// The session ID the call belongs to: the one the agent sent, or the one the SDK issued for
    /// this call. `None` when the call has no session: the agent sent a value this server did not
    /// issue, the tool declares its own `session_id` parameter, or the SDK does not trace the
    /// call. Also always `None` with a
    /// [`resolve_session_id`](crate::Options::resolve_session_id) hook set: the hook decides the
    /// session after the call starts, so the context has none.
    pub fn session_id(&self) -> Option<&str> {
        (!self.session_id.is_empty()).then_some(self.session_id.as_str())
    }

    /// The `agent_id` the agent sent, trimmed; `None` when agent tracking is off, the agent sent
    /// none, or the SDK does not trace the call.
    pub fn agent_id(&self) -> Option<&str> {
        self.agent_id.as_deref()
    }

    /// The handle that tracks the call.
    pub fn agentcat(&self) -> &AgentCat {
        &self.agentcat
    }
}

impl CallContext {
    /// Publishes a custom event with this call's client and server identity, as TypeScript's
    /// `publishCustomEvent(server, projectId, eventData)` does for a tracked server (B-009, C16).
    /// It only queues the event, so it returns at once.
    ///
    /// The event goes to the project of the handle ([`AgentCat::new`](crate::AgentCat::new)), not
    /// to `project_id`, which only has to be non-empty (K-03). Nothing is queued while tracing is
    /// off. The `redact_event` hook of the options runs on the event, on the runtime of the
    /// caller; the string hook does not. The event has no session unless the [`CustomEvent`]
    /// names one, and TypeScript takes none from the call either (C47): pass
    /// [`session_id()`](Self::session_id) in the [`CustomEvent`] to attribute the event to the
    /// call's session. A `duration` of zero or none is the time since the event was built.
    ///
    /// ```no_run
    /// # fn run(call: agentcat_sdk::CallContext) -> Result<(), agentcat_sdk::Error> {
    /// use agentcat_sdk::CustomEvent;
    ///
    /// let mut event = CustomEvent::new().resource_name("cache-warmed");
    /// if let Some(session) = call.session_id() {
    ///     event = event.session_id(session);
    /// }
    /// call.publish_custom_event("proj_123", event)?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::ProjectIdRequired`] when `project_id` is empty.
    pub fn publish_custom_event(&self, project_id: &str, event: CustomEvent) -> Result<(), Error> {
        let created = SystemTime::now();
        publish_server(
            &self.agentcat,
            project_id,
            event,
            self.client.as_ref(),
            self.server.as_ref(),
            created,
            SystemTime::now(),
        )
    }
}

//! [`CallContext`]: what a tool can read about its own call while `AgentCat` tracks it.

use crate::client::AgentCat;

/// The tracked call a tool is running in. While tracing is on, the SDK puts one into the request
/// context of each tool call before the call reaches the handler, so a tool can take it as an
/// `Extension<CallContext>` (rmcp's `rmcp::handler::server::common::Extension`) next to its
/// parameters.
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
}

impl CallContext {
    /// The context of a call with the session `session_id` (`""` for none) and the agent
    /// `agent_id`, tracked by `agentcat`.
    pub(crate) fn new(session_id: String, agent_id: Option<String>, agentcat: AgentCat) -> Self {
        Self {
            session_id,
            agent_id,
            agentcat,
        }
    }

    /// The session ID the call belongs to: the one the agent sent, or the one the SDK issued for
    /// this call. `None` when the call has no session: the agent sent a value this server did not
    /// issue, or the tool declares its own `session_id` parameter.
    pub fn session_id(&self) -> Option<&str> {
        (!self.session_id.is_empty()).then_some(self.session_id.as_str())
    }

    /// The `agent_id` the agent sent, trimmed; `None` when agent tracking is off or the agent sent
    /// none.
    pub fn agent_id(&self) -> Option<&str> {
        self.agent_id.as_deref()
    }

    /// The handle that tracks the call.
    pub fn agentcat(&self) -> &AgentCat {
        &self.agentcat
    }
}

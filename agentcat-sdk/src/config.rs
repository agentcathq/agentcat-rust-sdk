//! [`Options`]: what `AgentCat` adds to a server's tools and what it records (B-004).

use crate::constants::DEFAULT_CONTEXT_PARAMETER_DESCRIPTION;

#[cfg(test)]
mod tests;

/// Settings for an [`AgentCat`](crate::AgentCat) handle. [`Options::default`] has the defaults
/// of the TypeScript SDK; each setter consumes the options and returns them changed.
///
/// ```
/// use agentcat_sdk::Options;
///
/// let options = Options::default()
///     .enable_agent_tracking(true)
///     .custom_context_description("Say why you call this tool.");
/// # let _ = options;
/// ```
#[derive(Clone, Debug)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "the four boolean switches of the TS options (B-004)"
)]
pub struct Options {
    report_missing: bool,
    tracing: bool,
    tool_call_context: bool,
    agent_tracking: bool,
    custom_context_description: String,
}

impl Default for Options {
    /// The TypeScript SDK defaults: missing-tool reports on, tracing on, the `context` parameter
    /// on, agent tracking off, and the default `context` description.
    fn default() -> Self {
        Self {
            report_missing: true,
            tracing: true,
            tool_call_context: true,
            agent_tracking: false,
            custom_context_description: String::new(),
        }
    }
}

impl Options {
    /// Whether tool listings get the `get_more_tools` tool, through which agents report a tool
    /// they miss.
    #[must_use]
    pub fn enable_report_missing(mut self, on: bool) -> Self {
        self.report_missing = on;
        self
    }

    /// Whether tool calls are recorded and tools get the `session_id` parameter. With tracing
    /// off, tools still get the `context` parameter and `get_more_tools`.
    #[must_use]
    pub fn enable_tracing(mut self, on: bool) -> Self {
        self.tracing = on;
        self
    }

    /// Whether tools get a required `context` parameter, in which agents say why they call the
    /// tool. The text is recorded as the intent of the call.
    #[must_use]
    pub fn enable_tool_call_context(mut self, on: bool) -> Self {
        self.tool_call_context = on;
        self
    }

    /// Whether tools get a required `agent_id` parameter, with which agents identify themselves.
    #[must_use]
    pub fn enable_agent_tracking(mut self, on: bool) -> Self {
        self.agent_tracking = on;
        self
    }

    /// The description of the `context` parameter that agents read. An empty text means the
    /// default description.
    #[must_use]
    pub fn custom_context_description(mut self, text: impl Into<String>) -> Self {
        self.custom_context_description = text.into();
        self
    }

    /// Whether tool listings get `get_more_tools`.
    pub(crate) fn report_missing(&self) -> bool {
        self.report_missing
    }

    /// Whether tool calls are recorded and handles injected.
    pub(crate) fn tracing(&self) -> bool {
        self.tracing
    }

    /// Whether tools get the `context` parameter.
    pub(crate) fn tool_call_context(&self) -> bool {
        self.tool_call_context
    }

    /// Whether tools get the `agent_id` parameter.
    pub(crate) fn agent_tracking(&self) -> bool {
        self.agent_tracking
    }

    /// The description of the `context` parameter: the custom text, or the default where it is
    /// empty (TS treats a falsy description as none).
    pub(crate) fn context_description(&self) -> &str {
        if self.custom_context_description.is_empty() {
            DEFAULT_CONTEXT_PARAMETER_DESCRIPTION
        } else {
            &self.custom_context_description
        }
    }

    /// Whether a `resolve_session_id` hook decides the session (B-064, "hook mode"). There is no
    /// such hook yet, so this is `false`.
    #[expect(
        clippy::unused_self,
        reason = "the resolve_session_id hook joins later in plan 3"
    )]
    pub(crate) fn hook_mode(&self) -> bool {
        false
    }
}

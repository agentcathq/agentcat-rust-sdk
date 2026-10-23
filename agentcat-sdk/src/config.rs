//! [`Options`]: what `AgentCat` adds to a server's tools and what it records (B-004).

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::{Map, Value};

use crate::constants::DEFAULT_CONTEXT_PARAMETER_DESCRIPTION;
use crate::hooks::ToolCall;

#[cfg(test)]
mod tests;

/// The error a hook returns. The SDK logs it and goes on without what the hook would have given.
pub type HookError = Box<dyn std::error::Error + Send + Sync + 'static>;

/// Who calls the tools, as the `identify` hook reports it (D19).
///
/// ```
/// use agentcat_sdk::UserIdentity;
///
/// let user = UserIdentity::new("user-42").user_name("Ada");
/// # let _ = user;
/// ```
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct UserIdentity {
    /// The caller's id in the customer's system.
    pub user_id: String,
    /// The caller's display name.
    pub user_name: Option<String>,
    /// Any other data about the caller.
    pub user_data: Option<Map<String, Value>>,
}

impl UserIdentity {
    /// An identity with only the id.
    pub fn new(user_id: impl Into<String>) -> Self {
        Self {
            user_id: user_id.into(),
            user_name: None,
            user_data: None,
        }
    }

    /// Sets the display name.
    #[must_use]
    pub fn user_name(mut self, name: impl Into<String>) -> Self {
        self.user_name = Some(name.into());
        self
    }

    /// Sets the other data about the caller.
    #[must_use]
    pub fn user_data(mut self, data: Map<String, Value>) -> Self {
        self.user_data = Some(data);
        self
    }
}

/// The future a hook returns.
pub(crate) type HookFuture<T> = Pin<Box<dyn Future<Output = Result<Option<T>, HookError>> + Send>>;

/// A customer hook, shared between the clones of the [`Options`] that hold it.
pub(crate) struct Hook<T>(Arc<dyn Fn(ToolCall) -> HookFuture<T> + Send + Sync>);

impl<T: 'static> Hook<T> {
    fn new<F, Fut>(hook: F) -> Self
    where
        F: Fn(ToolCall) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Option<T>, HookError>> + Send + 'static,
    {
        Self(Arc::new(move |call| Box::pin(hook(call))))
    }

    /// The future of one call of the hook. The hook's own code, including a closure body that is
    /// not `async`, runs when the future is first polled, not here.
    pub(crate) fn call(&self, call: ToolCall) -> HookFuture<T> {
        let hook = Arc::clone(&self.0);
        Box::pin(async move { hook(call).await })
    }
}

impl<T> Clone for Hook<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T> fmt::Debug for Hook<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt("<hook>", f)
    }
}

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
///
/// The four hooks (`resolve_session_id`, `identify`, `event_tags`, `event_properties`) are async
/// closures over a [`ToolCall`]. The SDK starts them on the tokio runtime that serves the call and
/// never waits for them, so a slow hook cannot delay a tool call. A hook that returns an error or
/// `None` gives nothing. `rmcp`'s `local` feature is not supported, because hooks run as `Send`
/// tasks.
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
    resolve_session_id: Option<Hook<String>>,
    identify: Option<Hook<UserIdentity>>,
    event_tags: Option<Hook<Map<String, Value>>>,
    event_properties: Option<Hook<Map<String, Value>>>,
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
            resolve_session_id: None,
            identify: None,
            event_tags: None,
            event_properties: None,
        }
    }
}

impl Options {
    /// Decides the session of each tool call (hook mode, B-064). With this hook set, tools get no
    /// `session_id` parameter and the SDK mints nothing back to the agent. The hook returns the
    /// session id, or `None` to let the SDK mint one.
    #[must_use]
    pub fn resolve_session_id<F, Fut>(mut self, hook: F) -> Self
    where
        F: Fn(ToolCall) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Option<String>, HookError>> + Send + 'static,
    {
        self.resolve_session_id = Some(Hook::new(hook));
        self
    }

    /// Says who made each tool call. The hook runs on every call; `None` leaves the call
    /// anonymous.
    #[must_use]
    pub fn identify<F, Fut>(mut self, hook: F) -> Self
    where
        F: Fn(ToolCall) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Option<UserIdentity>, HookError>> + Send + 'static,
    {
        self.identify = Some(Hook::new(hook));
        self
    }

    /// Adds the returned tags to the event of each tool call.
    #[must_use]
    pub fn event_tags<F, Fut>(mut self, hook: F) -> Self
    where
        F: Fn(ToolCall) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Option<Map<String, Value>>, HookError>> + Send + 'static,
    {
        self.event_tags = Some(Hook::new(hook));
        self
    }

    /// Adds the returned properties to the event of each tool call.
    #[must_use]
    pub fn event_properties<F, Fut>(mut self, hook: F) -> Self
    where
        F: Fn(ToolCall) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Option<Map<String, Value>>, HookError>> + Send + 'static,
    {
        self.event_properties = Some(Hook::new(hook));
        self
    }

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

    /// Whether a `resolve_session_id` hook decides the session (B-064, "hook mode").
    pub(crate) fn hook_mode(&self) -> bool {
        self.resolve_session_id.is_some()
    }

    /// The `resolve_session_id` hook.
    pub(crate) fn resolve_session_id_hook(&self) -> Option<&Hook<String>> {
        self.resolve_session_id.as_ref()
    }

    /// The `identify` hook.
    pub(crate) fn identify_hook(&self) -> Option<&Hook<UserIdentity>> {
        self.identify.as_ref()
    }

    /// The `event_tags` hook.
    pub(crate) fn event_tags_hook(&self) -> Option<&Hook<Map<String, Value>>> {
        self.event_tags.as_ref()
    }

    /// The `event_properties` hook.
    pub(crate) fn event_properties_hook(&self) -> Option<&Hook<Map<String, Value>>> {
        self.event_properties.as_ref()
    }
}

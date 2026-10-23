//! [`Options`]: what `AgentCat` adds to a server's tools and what it records (B-004).

use std::fmt;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::{Map, Value};

use crate::constants::DEFAULT_CONTEXT_PARAMETER_DESCRIPTION;
use crate::event::Event;
use crate::hooks::ToolCall;
use crate::js;

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

/// The future a redaction hook returns.
pub(crate) type RedactFuture<T> = Pin<Box<dyn Future<Output = Result<T, HookError>> + Send>>;

/// A customer redaction hook from `A` to `T`, shared between the clones of the [`Options`] that
/// hold it.
pub(crate) struct Redactor<A, T>(Arc<dyn Fn(A) -> RedactFuture<T> + Send + Sync>);

/// The hook of [`Options::redact_sensitive_information`].
pub(crate) type StringRedactor = Redactor<String, String>;

/// The hook of [`Options::redact_event`].
pub(crate) type EventRedactor = Redactor<Event, Option<Event>>;

impl<A, T> Redactor<A, T> {
    fn new<F, Fut>(hook: F) -> Self
    where
        F: Fn(A) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<T, HookError>> + Send + 'static,
    {
        Self(Arc::new(move |input| Box::pin(hook(input))))
    }

    /// The future of one call of the hook. The hook's own code runs when the future is first
    /// polled, not here.
    pub(crate) fn call(&self, input: A) -> RedactFuture<T>
    where
        A: Send + 'static,
        T: 'static,
    {
        let hook = Arc::clone(&self.0);
        Box::pin(async move { hook(input).await })
    }
}

impl<A, T> Clone for Redactor<A, T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<A, T> fmt::Debug for Redactor<A, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt("<hook>", f)
    }
}

/// Where the SDK writes its own log lines (B-117, D38). The destination is process-wide: the
/// first handle built decides it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum LogDestination {
    /// The file `agentcat.log` in the user's home directory, as the TypeScript SDK writes it.
    #[default]
    File,
    /// `tracing` events with the target `agentcat`.
    Tracing,
}

/// The environment variable that holds the project ID when none is passed to the SDK.
pub const AGENTCAT_PROJECT_ID: &str = "AGENTCAT_PROJECT_ID";

/// The environment variable that holds the base URL of the `AgentCat` API.
pub const AGENTCAT_API_URL: &str = "AGENTCAT_API_URL";

/// The environment variable of the TypeScript SDK's former name for [`AGENTCAT_API_URL`], read
/// when that one is unset or empty.
pub const MCPCAT_API_URL: &str = "MCPCAT_API_URL";

/// The environment variable that turns the SDK off: `false`, `0`, `no` or `off` (any case,
/// surrounding white space ignored) disables it.
pub const AGENTCAT_ENABLED: &str = "AGENTCAT_ENABLED";

/// The base URL of the `AgentCat` API when neither the options nor the environment name one.
pub(crate) const DEFAULT_API_BASE_URL: &str = "https://api.agentcat.com";

/// Where the SDK reads its environment: the process, or a map in tests (the one place that
/// touches `std::env`).
pub(crate) trait Env: Send + Sync + fmt::Debug {
    /// The value of the variable `name`, `None` when it is unset or not text.
    fn var(&self, name: &str) -> Option<String>;

    /// The user's home directory.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "read by the log writer (plan 4 task 3)")
    )]
    fn home_dir(&self) -> Option<PathBuf>;
}

/// The environment of the running process.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "built by the handle constructor (plan 4 task 9)")
)]
pub(crate) struct ProcessEnv;

impl Env for ProcessEnv {
    #[expect(
        clippy::disallowed_methods,
        reason = "the one place that reads the process environment"
    )]
    fn var(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }

    fn home_dir(&self) -> Option<PathBuf> {
        std::env::home_dir()
    }
}

/// A fixed environment for tests.
#[cfg(test)]
#[derive(Debug, Clone, Default)]
pub(crate) struct MapEnv {
    vars: std::collections::HashMap<String, String>,
    home: Option<PathBuf>,
}

#[cfg(test)]
impl MapEnv {
    /// An environment with the variables `vars` and no home directory.
    pub(crate) fn new(vars: &[(&str, &str)]) -> Self {
        Self {
            vars: vars
                .iter()
                .map(|&(name, value)| (name.to_owned(), value.to_owned()))
                .collect(),
            home: None,
        }
    }

    /// The same environment with the home directory `home`.
    pub(crate) fn with_home(mut self, home: PathBuf) -> Self {
        self.home = Some(home);
        self
    }
}

#[cfg(test)]
impl Env for MapEnv {
    fn var(&self, name: &str) -> Option<String> {
        self.vars.get(name).cloned()
    }

    fn home_dir(&self) -> Option<PathBuf> {
        self.home.clone()
    }
}

/// The base URL events go to (B-006): the first non-empty of the option, `AGENTCAT_API_URL`,
/// `MCPCAT_API_URL`, else the default. The value is used as given, with no trailing slash
/// trimmed, as in TypeScript.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "read by the delivery worker (plan 4 task 8)")
)]
pub(crate) fn resolve_api_base_url(options: &Options, env: &dyn Env) -> String {
    options
        .api_base_url_option()
        .map(str::to_owned)
        .or_else(|| env.var(AGENTCAT_API_URL).filter(|url| !url.is_empty()))
        .or_else(|| env.var(MCPCAT_API_URL).filter(|url| !url.is_empty()))
        .unwrap_or_else(|| DEFAULT_API_BASE_URL.to_owned())
}

/// Whether the SDK is on (D44, R10): off when `AGENTCAT_ENABLED`, trimmed and lower-cased, is
/// `false`, `0`, `no` or `off`; on otherwise, also when unset.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "read by the handle constructor (plan 4 task 9)")
)]
pub(crate) fn sdk_enabled(env: &dyn Env) -> bool {
    let Some(value) = env.var(AGENTCAT_ENABLED) else {
        return true;
    };
    let value = js::trim(&value).to_lowercase();
    !matches!(value.as_str(), "false" | "0" | "no" | "off")
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
    api_base_url: String,
    string_redactor: Option<StringRedactor>,
    event_redactor: Option<EventRedactor>,
    log_destination: LogDestination,
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
            api_base_url: String::new(),
            string_redactor: None,
            event_redactor: None,
            log_destination: LogDestination::File,
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

    /// The base URL of the `AgentCat` API, in place of the environment's and the default. An empty
    /// text means none is set. The URL is used as given.
    #[must_use]
    pub fn api_base_url(mut self, url: impl Into<String>) -> Self {
        self.api_base_url = url.into();
        self
    }

    /// Redacts every string of each event before it leaves the process: the hook gets one string
    /// at a time and returns the string to send. It runs on the tokio runtime that serves the
    /// call. When it fails, the SDK drops the event rather than send it unredacted.
    #[must_use]
    pub fn redact_sensitive_information<F, Fut>(mut self, hook: F) -> Self
    where
        F: Fn(String) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<String, HookError>> + Send + 'static,
    {
        self.string_redactor = Some(Redactor::new(hook));
        self
    }

    /// Changes each event before it leaves the process: the hook gets the whole [`Event`] and
    /// returns the event to send, or `Ok(None)` to drop it. It runs on the tokio runtime that
    /// serves the call. When it fails, the SDK drops the event.
    #[must_use]
    pub fn redact_event<F, Fut>(mut self, hook: F) -> Self
    where
        F: Fn(Event) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Option<Event>, HookError>> + Send + 'static,
    {
        self.event_redactor = Some(Redactor::new(hook));
        self
    }

    /// Where the SDK writes its own log lines. The first handle built in the process decides
    /// (D38). The default is [`LogDestination::File`].
    #[must_use]
    pub fn log_destination(mut self, destination: LogDestination) -> Self {
        self.log_destination = destination;
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
    /// off, tools get neither `session_id` nor `agent_id` (also with agent tracking on), each call
    /// goes to the tool with its arguments as the agent sent them, and nothing is recorded; tools
    /// still get the `context` parameter and `get_more_tools`.
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
    /// Only with tracing on ([`enable_tracing`](Self::enable_tracing)).
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

    /// The base URL set with [`api_base_url`](Self::api_base_url), when it is not empty.
    pub(crate) fn api_base_url_option(&self) -> Option<&str> {
        Some(self.api_base_url.as_str()).filter(|url| !url.is_empty())
    }

    /// The `redact_sensitive_information` hook.
    pub(crate) fn string_redactor(&self) -> Option<&StringRedactor> {
        self.string_redactor.as_ref()
    }

    /// The `redact_event` hook.
    pub(crate) fn event_redactor(&self) -> Option<&EventRedactor> {
        self.event_redactor.as_ref()
    }

    /// Where the SDK writes its log lines.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "read by the log writer (plan 4 task 3)")
    )]
    pub(crate) fn log_to(&self) -> LogDestination {
        self.log_destination
    }
}

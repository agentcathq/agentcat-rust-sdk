//! The configured tracker: [`AgentCat`] and its builder, custom events, and
//! flushing.

use std::future::Future;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use serde_json::{Map, Value};

use crate::constants::{AGENTCAT_CUSTOM_EVENT_TYPE, SDK_LANGUAGE, SDK_VERSION};
use crate::event::Event;
use crate::exporters::ExporterConfig;
use crate::hooks::{EventRedactor, StringRedactor};
use crate::inject::{InjectConfig, InjectMemory};
use crate::logging::write_to_log;
use crate::publisher::{self, Pending, QueuedEvent};

#[cfg(feature = "rmcp")]
use crate::tool_call::{IdentifyHook, PropertiesHook, SessionHook, TagsHook, ToolCallRequest};

/// Errors from the AgentCat API surface. Tracking itself never fails: a
/// problem in AgentCat degrades to an untracked call, never a failed one.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// `publish_custom_event` needs a project ID.
    MissingProjectId,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::MissingProjectId => write!(f, "projectId is required for publishCustomEvent"),
        }
    }
}

impl std::error::Error for Error {}

/// Resolved options, shared by every tracked server of one [`AgentCat`].
pub(crate) struct Options {
    pub(crate) enable_report_missing: bool,
    pub(crate) enable_tracing: bool,
    pub(crate) enable_tool_call_context: bool,
    pub(crate) custom_context_description: Option<String>,
    pub(crate) enable_agent_tracking: bool,
    #[cfg(feature = "rmcp")]
    pub(crate) resolve_session_id: Option<SessionHook>,
    #[cfg(feature = "rmcp")]
    pub(crate) identify: Option<IdentifyHook>,
    #[cfg(feature = "rmcp")]
    pub(crate) event_tags: Option<TagsHook>,
    #[cfg(feature = "rmcp")]
    pub(crate) event_properties: Option<PropertiesHook>,
    pub(crate) redact_sensitive_information: Option<StringRedactor>,
    pub(crate) redact_event: Option<EventRedactor>,
    pub(crate) api_base_url: Option<String>,
    pub(crate) disable_diagnostics: bool,
    pub(crate) exporters: Vec<(String, ExporterConfig)>,
    pub(crate) log_file: bool,
    pub(crate) capture_panics: bool,
    pub(crate) server_name: Option<String>,
    pub(crate) server_version: Option<String>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            enable_report_missing: true,
            enable_tracing: true,
            enable_tool_call_context: true,
            custom_context_description: None,
            enable_agent_tracking: false,
            #[cfg(feature = "rmcp")]
            resolve_session_id: None,
            #[cfg(feature = "rmcp")]
            identify: None,
            #[cfg(feature = "rmcp")]
            event_tags: None,
            #[cfg(feature = "rmcp")]
            event_properties: None,
            redact_sensitive_information: None,
            redact_event: None,
            api_base_url: None,
            disable_diagnostics: false,
            exporters: Vec::new(),
            log_file: true,
            capture_panics: true,
            server_name: None,
            server_version: None,
        }
    }
}

impl Options {
    #[cfg_attr(
        not(feature = "rmcp"),
        allow(clippy::unused_self, reason = "hook mode needs the rmcp-only resolver")
    )]
    pub(crate) fn hook_mode(&self) -> bool {
        #[cfg(feature = "rmcp")]
        {
            self.resolve_session_id.is_some()
        }
        #[cfg(not(feature = "rmcp"))]
        {
            false
        }
    }
}

pub(crate) struct Shared {
    pub(crate) project_id: Option<String>,
    pub(crate) options: Options,
    pub(crate) inject: InjectConfig,
    pub(crate) memory: InjectMemory,
    /// The last server identity a tracked server reported, for custom events.
    pub(crate) server_info: RwLock<Option<(Option<String>, Option<String>)>>,
}

impl Shared {
    /// Queues an event with this configuration's redaction hooks.
    pub(crate) fn publish(&self, event: Event, pending: Pending) {
        publisher::global().enqueue(QueuedEvent {
            event,
            pending,
            event_redactor: self.options.redact_event.clone(),
            string_redactor: self.options.redact_sensitive_information.clone(),
            approx_bytes: 0,
        });
    }

    /// An event stamped with the SDK and server metadata.
    pub(crate) fn base_event(&self, event_type: &str) -> Event {
        let mut event = Event {
            project_id: self.project_id.clone(),
            event_type: event_type.to_owned(),
            timestamp: crate::util::now_millis(),
            sdk_language: Some(SDK_LANGUAGE.to_owned()),
            agentcat_version: Some(SDK_VERSION.to_owned()),
            identify_actor_data: Some(Value::Object(Map::new())),
            ..Event::default()
        };
        if let Ok(guard) = self.server_info.read()
            && let Some((name, version)) = guard.as_ref()
        {
            event.server_name.clone_from(name);
            event.server_version.clone_from(version);
        }
        event
    }
}

/// AgentCat analytics for MCP servers.
///
/// Build one with [`AgentCat::builder`] (or [`AgentCat::new`] for the
/// defaults), then wrap your rmcp server handler with [`AgentCat::track`].
/// `AgentCat` is cheap to clone; clone it into per-request factories.
///
/// ```no_run
/// # #[cfg(feature = "rmcp")]
/// # async fn demo<H: rmcp::ServerHandler>(server: H) -> anyhow::Result<()> {
/// use rmcp::{ServiceExt, transport::stdio};
///
/// let agentcat = agentcat::AgentCat::new("proj_0000000000000000000000000000");
/// agentcat.track(server).serve(stdio()).await?.waiting().await?;
/// agentcat::shutdown(std::time::Duration::from_secs(5)).await;
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct AgentCat {
    pub(crate) shared: Arc<Shared>,
}

impl std::fmt::Debug for AgentCat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentCat")
            .field("project_id", &self.shared.project_id)
            .finish_non_exhaustive()
    }
}

impl AgentCat {
    /// `AgentCat` with the default options, publishing to `project_id`.
    pub fn new(project_id: impl Into<String>) -> Self {
        Self::builder(project_id).build()
    }

    /// A builder publishing to `project_id`.
    pub fn builder(project_id: impl Into<String>) -> AgentCatBuilder {
        let project_id: String = project_id.into();
        AgentCatBuilder {
            project_id: (!project_id.is_empty()).then_some(project_id),
            options: Options::default(),
        }
    }

    /// A builder without an AgentCat project: events go to the configured
    /// exporters only.
    pub fn telemetry_only() -> AgentCatBuilder {
        AgentCatBuilder {
            project_id: None,
            options: Options::default(),
        }
    }

    /// The AgentCat project ID, or `None` in telemetry-only mode.
    pub fn project_id(&self) -> Option<&str> {
        self.shared.project_id.as_deref()
    }

    /// Publishes an `agentcat:custom` event for this project.
    ///
    /// The `redact_event` and `redact_sensitive_information` hooks apply. Set
    /// [`CustomEvent::session_id`] to attribute the event to a task; without
    /// it the event has no session and the API assigns one.
    pub fn publish_custom_event(&self, data: CustomEvent) {
        let session_id = if let Some(s) = data.session_id.clone().filter(|s| !s.is_empty()) {
            s
        } else {
            write_to_log(
                "publishCustomEvent: no sessionId provided; event will be published without a session. Pass eventData.sessionId to attribute it to a task.",
            );
            String::new()
        };
        let mut event = self.shared.base_event(AGENTCAT_CUSTOM_EVENT_TYPE);
        fill_custom(&mut event, session_id, data);
        // As in TS, a tracked custom event without a duration reports the
        // time since it was built: zero.
        event.duration.get_or_insert(0);
        log_custom(&event.session_id);
        self.shared.publish(event, Pending::default());
    }

    /// Waits until queued events are delivered, or `timeout` passes.
    /// Returns true when everything was sent.
    pub async fn flush(&self, timeout: Duration) -> bool {
        flush(timeout).await
    }

    /// Flushes, then stops accepting events. See [`shutdown`].
    pub async fn shutdown(&self, timeout: Duration) -> bool {
        shutdown(timeout).await
    }
}

/// Configures an [`AgentCat`]. Option names follow the TypeScript SDK.
#[must_use]
pub struct AgentCatBuilder {
    project_id: Option<String>,
    options: Options,
}

impl AgentCatBuilder {
    /// Adds a `get_more_tools` tool that lets agents report missing
    /// functionality. Default true.
    pub fn enable_report_missing(mut self, enabled: bool) -> Self {
        self.options.enable_report_missing = enabled;
        self
    }

    /// Tracks tool calls (events, session handles, mint-back). Default true.
    pub fn enable_tracing(mut self, enabled: bool) -> Self {
        self.options.enable_tracing = enabled;
        self
    }

    /// Adds a required `context` parameter to every tool, which captures the
    /// agent's intent. Default true.
    pub fn enable_tool_call_context(mut self, enabled: bool) -> Self {
        self.options.enable_tool_call_context = enabled;
        self
    }

    /// Replaces the description of the injected `context` parameter, to give
    /// agents domain-specific guidance.
    pub fn custom_context_description(mut self, description: impl Into<String>) -> Self {
        self.options.custom_context_description = Some(description.into());
        self
    }

    /// Adds a required `agent_id` parameter, so each agent (including every
    /// subagent) is identifiable. Default false.
    pub fn enable_agent_tracking(mut self, enabled: bool) -> Self {
        self.options.enable_agent_tracking = enabled;
        self
    }

    /// Hook mode: you supply the session identifier per request, and AgentCat
    /// injects no `session_id` parameter and adds no issuance text. The
    /// value is combined with the project ID into a deterministic `ses_` ID.
    /// Return `None` to mint silently.
    #[cfg(feature = "rmcp")]
    pub fn resolve_session_id<F, Fut>(mut self, hook: F) -> Self
    where
        F: Fn(ToolCallRequest) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Option<String>> + Send + 'static,
    {
        self.options.resolve_session_id = Some(Arc::new(move |req| Box::pin(hook(req))));
        self
    }

    /// Identifies the actor behind each tool call. Runs on every call, in the
    /// background: it never delays the tool. The result is stamped on that
    /// call's event.
    #[cfg(feature = "rmcp")]
    pub fn identify<F, Fut>(mut self, hook: F) -> Self
    where
        F: Fn(ToolCallRequest) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Option<crate::UserIdentity>> + Send + 'static,
    {
        self.options.identify = Some(Arc::new(move |req| Box::pin(hook(req))));
        self
    }

    /// Attaches indexed string tags to each tool-call event. Keys: at most 32
    /// UTF-16 units of `[A-Za-z0-9$_.:- ]`; values: at most 200 units, no
    /// newlines; at most 50 entries. Invalid entries are
    /// dropped with a log line.
    #[cfg(feature = "rmcp")]
    pub fn event_tags<F, Fut, T, K, V>(mut self, hook: F) -> Self
    where
        F: Fn(ToolCallRequest) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Option<T>> + Send + 'static,
        T: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        let hook = Arc::new(hook);
        self.options.event_tags = Some(Arc::new(move |req| {
            let fut = hook(req);
            Box::pin(async move {
                fut.await.map(|tags| {
                    tags.into_iter()
                        .map(|(k, v)| (k.into(), Value::String(v.into())))
                        .collect::<Vec<(String, Value)>>()
                })
            })
        }));
        self
    }

    /// Attaches free-form JSON properties (a JSON object) to each tool-call
    /// event.
    #[cfg(feature = "rmcp")]
    pub fn event_properties<F, Fut>(mut self, hook: F) -> Self
    where
        F: Fn(ToolCallRequest) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Option<Value>> + Send + 'static,
    {
        self.options.event_properties = Some(Arc::new(move |req| Box::pin(hook(req))));
        self
    }

    /// Redacts sensitive text before events leave the process. Applied to
    /// every string in parameters, responses, intent and errors.
    pub fn redact_sensitive_information<F>(mut self, redact: F) -> Self
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        self.options.redact_sensitive_information = Some(StringRedactor::Sync(Arc::new(redact)));
        self
    }

    /// Async variant of [`AgentCatBuilder::redact_sensitive_information`].
    pub fn redact_sensitive_information_async<F, Fut>(mut self, redact: F) -> Self
    where
        F: Fn(String) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = String> + Send + 'static,
    {
        self.options.redact_sensitive_information =
            Some(StringRedactor::Async(Arc::new(move |s| Box::pin(redact(s)))));
        self
    }

    /// Inspects or rewrites each whole event before it is published; return
    /// `None` to drop it. Runs before string redaction, on raw values. The
    /// system fields `id`, `session_id`, `project_id`, `event_type` and
    /// `timestamp` cannot be changed. A panic drops the event.
    pub fn redact_event<F>(mut self, hook: F) -> Self
    where
        F: Fn(Event) -> Option<Event> + Send + Sync + 'static,
    {
        self.options.redact_event = Some(EventRedactor::Sync(Arc::new(hook)));
        self
    }

    /// Async variant of [`AgentCatBuilder::redact_event`].
    pub fn redact_event_async<F, Fut>(mut self, hook: F) -> Self
    where
        F: Fn(Event) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Option<Event>> + Send + 'static,
    {
        self.options.redact_event = Some(EventRedactor::Async(Arc::new(move |e| Box::pin(hook(e)))));
        self
    }

    /// The AgentCat API URL. Falls back to `AGENTCAT_API_URL`, then
    /// `MCPCAT_API_URL`, then `https://api.agentcat.com`. Process-wide: the
    /// first configured value wins.
    pub fn api_base_url(mut self, url: impl Into<String>) -> Self {
        self.options.api_base_url = Some(url.into());
        self
    }

    /// Turns off anonymous SDK diagnostics. Also `DISABLE_DIAGNOSTICS=1`.
    pub fn disable_diagnostics(mut self, disabled: bool) -> Self {
        self.options.disable_diagnostics = disabled;
        self
    }

    /// Adds a telemetry exporter. Process-wide: the first `AgentCat` built
    /// with exporters sets them.
    pub fn exporter(mut self, name: impl Into<String>, config: impl Into<ExporterConfig>) -> Self {
        self.options.exporters.push((name.into(), config.into()));
        self
    }

    /// Writes SDK log lines to `~/agentcat.log` (or `$AGENTCAT_LOG_FILE`).
    /// Default true. The log never goes to stdout, which is the protocol
    /// channel of stdio servers.
    pub fn log_file(mut self, enabled: bool) -> Self {
        self.options.log_file = enabled;
        self
    }

    /// Records panics inside tool handlers (message, location, backtrace)
    /// through a panic hook that chains to the existing one. The panic still
    /// propagates exactly as before. Default true.
    pub fn capture_panics(mut self, enabled: bool) -> Self {
        self.options.capture_panics = enabled;
        self
    }

    /// Overrides the server name and version on events. By default they come
    /// from the server's `get_info()`.
    pub fn server_info(mut self, name: impl Into<String>, version: impl Into<String>) -> Self {
        self.options.server_name = Some(name.into());
        self.options.server_version = Some(version.into());
        self
    }

    /// Builds the tracker. Never fails: problems are logged.
    pub fn build(self) -> AgentCat {
        let AgentCatBuilder { project_id, options } = self;
        crate::diagnostics::init(project_id.as_deref(), options.disable_diagnostics);
        if !options.log_file {
            crate::logging::set_file_logging(false);
        }
        let label = project_id.as_deref().unwrap_or("(telemetry-only)");
        write_to_log(&format!(
            "AgentCat setup started | project {label} | sdk rust/{} | mcp {}",
            SDK_VERSION,
            crate::runtime::mcp_library_version().unwrap_or("unknown")
        ));

        let publisher = publisher::global();
        let api_base_url = options
            .api_base_url
            .clone()
            .filter(|u| !u.is_empty())
            .or_else(|| std::env::var("AGENTCAT_API_URL").ok().filter(|u| !u.is_empty()))
            .or_else(|| std::env::var("MCPCAT_API_URL").ok().filter(|u| !u.is_empty()));
        if let Some(url) = api_base_url
            && publisher.config.api_base_url.set(url.clone()).is_err()
            && publisher.config.api_base_url.get() != Some(&url)
        {
            write_to_log(&format!(
                "Warning: AgentCat API URL is already set to {}; ignoring {url}",
                publisher
                    .config
                    .api_base_url
                    .get()
                    .map(String::as_str)
                    .unwrap_or_default()
            ));
        }

        if !options.exporters.is_empty() && publisher.config.exporters.get().is_none() {
            let mut created = Vec::new();
            for (name, config) in &options.exporters {
                match crate::exporters::create(config) {
                    Ok(exporter) => {
                        write_to_log(&format!("Initialized telemetry exporter: {name}"));
                        created.push((name.clone(), exporter));
                    }
                    Err(e) => write_to_log(&format!("Failed to initialize exporter {name}: {e}")),
                }
            }
            if publisher.config.exporters.set(created).is_ok() {
                write_to_log(&format!(
                    "Initialized telemetry with {} exporters",
                    options.exporters.len()
                ));
            }
        }

        if project_id.is_none() && options.exporters.is_empty() {
            write_to_log(
                "Warning: No projectId provided and no exporters configured. Events will not be sent anywhere.",
            );
        }
        if let Some(p) = project_id.as_deref()
            && !p.starts_with("proj_")
        {
            write_to_log(&format!(
                "Warning: project ID \"{p}\" does not start with \"proj_\"; the AgentCat API will drop its events."
            ));
        }
        if !json_keeps_insertion_order() {
            write_to_log(
                "Warning: serde_json sorts object keys in this build (its preserve_order feature is off, for example through default-features = false). Injected parameters and the mcp_session field will not come first; enable the agentcat-sdk preserve-order feature.",
            );
        }
        if options.capture_panics && options.enable_tracing {
            crate::exceptions::install_panic_hook();
        }

        let inject = InjectConfig::new(
            options.enable_tracing,
            options.hook_mode(),
            options.enable_agent_tracking,
            options.enable_tool_call_context,
            options.custom_context_description.as_deref(),
            options.enable_report_missing,
        );
        write_to_log(&format!(
            "AgentCat setup complete | project {label} | tracing={} context={} reportMissing={} exporters={}",
            options.enable_tracing,
            options.enable_tool_call_context,
            options.enable_report_missing,
            options.exporters.len()
        ));
        let server_info = (options.server_name.is_some() || options.server_version.is_some())
            .then(|| (options.server_name.clone(), options.server_version.clone()));
        AgentCat {
            shared: Arc::new(Shared {
                project_id,
                options,
                inject,
                memory: InjectMemory::default(),
                server_info: RwLock::new(server_info),
            }),
        }
    }
}

/// True when `serde_json` keeps insertion order (its `preserve_order` feature
/// is on anywhere in the build).
fn json_keeps_insertion_order() -> bool {
    let mut probe = Map::new();
    probe.insert("b".into(), Value::Null);
    probe.insert("a".into(), Value::Null);
    probe.keys().next().map(String::as_str) == Some("b")
}

/// Data for an `agentcat:custom` event.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct CustomEvent {
    /// The session (task) to attribute the event to, used verbatim.
    pub session_id: Option<String>,
    /// What the event is about.
    pub resource_name: Option<String>,
    /// Any JSON input.
    pub parameters: Option<Value>,
    /// Any JSON output.
    pub response: Option<Value>,
    /// A message, stored as the event's intent.
    pub message: Option<String>,
    /// Duration in milliseconds.
    pub duration: Option<u64>,
    /// Marks the event as an error.
    pub is_error: Option<bool>,
    /// Error details, for example `ErrorData::from_error(&e).to_value()`.
    pub error: Option<Value>,
    /// Indexed string tags (validated like `event_tags`).
    pub tags: Option<Vec<(String, String)>>,
    /// Free-form JSON properties.
    pub properties: Option<Map<String, Value>>,
}

impl CustomEvent {
    /// An empty custom event.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the session.
    #[must_use]
    pub fn session_id(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    /// Sets the resource name.
    #[must_use]
    pub fn resource_name(mut self, name: impl Into<String>) -> Self {
        self.resource_name = Some(name.into());
        self
    }

    /// Sets the parameters.
    #[must_use]
    pub fn parameters(mut self, parameters: Value) -> Self {
        self.parameters = Some(parameters);
        self
    }

    /// Sets the response.
    #[must_use]
    pub fn response(mut self, response: Value) -> Self {
        self.response = Some(response);
        self
    }

    /// Sets the message.
    #[must_use]
    pub fn message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }

    /// Sets the duration in milliseconds.
    #[must_use]
    pub fn duration(mut self, millis: u64) -> Self {
        self.duration = Some(millis);
        self
    }

    /// Marks the event as an error with the given details.
    #[must_use]
    pub fn error(mut self, error: impl Into<Value>) -> Self {
        self.is_error = Some(true);
        self.error = Some(error.into());
        self
    }

    /// Adds a tag.
    #[must_use]
    pub fn tag(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.tags
            .get_or_insert_with(Vec::new)
            .push((key.into(), value.into()));
        self
    }

    /// Sets the properties. Values that are not JSON objects are ignored.
    #[must_use]
    pub fn properties(mut self, properties: Value) -> Self {
        if let Value::Object(map) = properties {
            self.properties = Some(map);
        }
        self
    }
}

impl From<crate::event::ErrorData> for Value {
    fn from(error: crate::event::ErrorData) -> Self {
        error.to_value()
    }
}

/// The API keeps session IDs of at most 31 characters.
const MAX_SESSION_ID_CHARS: usize = 31;

fn fill_custom(event: &mut Event, session_id: String, data: CustomEvent) {
    event.session_id = if session_id.chars().count() > MAX_SESSION_ID_CHARS {
        write_to_log(&format!(
            "Warning: custom event session ID \"{session_id}\" is longer than {MAX_SESSION_ID_CHARS} characters, which the AgentCat API rejects; publishing the event without a session"
        ));
        String::new()
    } else {
        session_id
    };
    event.resource_name = data.resource_name;
    event.parameters = data.parameters;
    event.response = data.response;
    event.user_intent = data.message;
    event.duration = data.duration;
    event.is_error = data.is_error;
    event.error = data.error;
    if let Some(tags) = data.tags {
        event.tags = crate::validation::validate_tags(tags.into_iter().map(|(k, v)| (k, Value::String(v))))
            .map(|t| t.into_iter().collect());
    }
    event.properties = data.properties.filter(|p| !p.is_empty());
}

fn log_custom(session_id: &str) {
    let target = if session_id.is_empty() {
        "without a session".to_owned()
    } else {
        format!("for session {session_id}")
    };
    write_to_log(&format!(
        "Published custom event {target} with type 'agentcat:custom'"
    ));
}

/// Publishes an `agentcat:custom` event to `project_id` under a session ID
/// used verbatim, without a tracked server. No redaction hooks apply.
/// [`CustomEvent::session_id`], when set, wins over `session_id`.
///
/// # Errors
///
/// [`Error::MissingProjectId`] when `project_id` is empty.
pub fn publish_custom_event(session_id: &str, project_id: &str, data: CustomEvent) -> Result<(), Error> {
    if project_id.is_empty() {
        return Err(Error::MissingProjectId);
    }
    let session = data
        .session_id
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| session_id.to_owned());
    let mut event = Event {
        project_id: Some(project_id.to_owned()),
        event_type: AGENTCAT_CUSTOM_EVENT_TYPE.to_owned(),
        timestamp: crate::util::now_millis(),
        ..Event::default()
    };
    fill_custom(&mut event, session, data);
    log_custom(&event.session_id);
    publisher::global().enqueue(QueuedEvent {
        event,
        pending: Pending::default(),
        event_redactor: None,
        string_redactor: None,
        approx_bytes: 0,
    });
    Ok(())
}

/// Waits until every queued event is delivered (or given up on) and every
/// export has finished, or until `timeout`. Returns true when idle.
pub async fn flush(timeout: Duration) -> bool {
    match publisher::try_global() {
        Some(p) => p.flush(timeout).await,
        None => true,
    }
}

/// Blocking [`flush`], for code without an async runtime (for example in
/// `Drop` or after the runtime has stopped).
pub fn flush_blocking(timeout: Duration) -> bool {
    match publisher::try_global() {
        Some(p) => p.flush_blocking(timeout),
        None => true,
    }
}

/// Stops accepting events, then waits up to `timeout` for the queue to drain.
/// Call it before your process exits: stdio servers end when `main` returns,
/// which would drop queued events. Returns true when everything was sent.
pub async fn shutdown(timeout: Duration) -> bool {
    match publisher::try_global() {
        Some(p) => {
            p.close();
            p.flush(timeout).await
        }
        None => true,
    }
}

/// Blocking [`shutdown`].
pub fn shutdown_blocking(timeout: Duration) -> bool {
    match publisher::try_global() {
        Some(p) => {
            p.close();
            p.flush_blocking(timeout)
        }
        None => true,
    }
}

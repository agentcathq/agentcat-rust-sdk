//! [`AgentCat`]: the handle that every [`Tracked`] server of a project shares, and [`track`].

use std::any::TypeId;
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use rmcp::ServerHandler;

use crate::config::Options;
#[cfg(test)]
use crate::config::env::MapEnv;
#[cfg(not(test))]
use crate::config::env::ProcessEnv;
use crate::config::env::{AGENTCAT_PROJECT_ID, Env, resolve_api_base_url, sdk_enabled};
use crate::custom::{CustomEvent, publish_string};
#[cfg(feature = "test-util")]
use crate::delivery::Observer;
use crate::delivery::http::ApiPublisher;
use crate::delivery::{Delivery, DeliveryConfig, LineWriter, Publisher};
use crate::error::Error;
use crate::event::CallRecord;
use crate::event::assemble::EventJob;
use crate::logging;
use crate::registry::ToolRegistry;
use crate::sink::Sink;
use crate::tracked::Tracked;

#[cfg(test)]
mod tests;

/// A handle to `AgentCat` for one project. Clones are cheap and share everything, so one handle
/// serves every server instance: a streamable HTTP service builds one handler per session or per
/// request, and each of them is wrapped with [`AgentCat::track`].
///
/// ```
/// use agentcat_sdk::{AgentCat, Options};
///
/// # struct MyServer;
/// # impl rmcp::ServerHandler for MyServer {}
/// let agentcat = AgentCat::new("proj_123", Options::default());
/// let make_server = {
///     let agentcat = agentcat.clone();
///     move || agentcat.track(MyServer)
/// };
/// # let _ = make_server();
/// ```
///
/// Its `Debug` output shows the project ID and the options only.
///
/// # Dropping the last handle
///
/// A handle delivers its events from its own thread, which the first event or log line starts.
/// When the last clone drops (the [`Tracked`] servers hold clones too) while that thread runs,
/// the drop runs [`shutdown`](Self::shutdown) with a 5 s timeout and blocks the dropping thread
/// until the delivery thread answers: at most about 5 s, less when the events finish sooner. The
/// events that have not finished at the deadline are dropped.
///
/// The hooks of [`Options`] (`resolve_session_id`, `identify`, `event_tags`, `event_properties`,
/// `redact_sensitive_information` and `redact_event`) run on the tokio runtime of the tool call.
/// On a current-thread runtime, such as `#[tokio::main(flavor = "current_thread")]`, a drop on
/// the runtime's thread blocks its only thread, so no hook can run during the wait: an event that
/// still needs a hook waits for the whole 5 s and is then dropped. Call `shutdown(timeout).await`
/// before the runtime ends, while the hooks can still run:
///
/// ```no_run
/// # async fn run(agentcat: agentcat_sdk::AgentCat) {
/// use std::time::Duration;
///
/// // ... serve until the client goes away ...
/// let delivered = agentcat.shutdown(Duration::from_secs(5)).await;
/// # let _ = delivered;
/// # }
/// ```
///
/// A hook of [`Options`] that holds a clone of the handle makes a reference cycle: the handle
/// holds its options, and the options hold the hook. The handle then never drops, so the drain
/// of the last drop never runs. Call [`shutdown`](Self::shutdown) before the process ends, or let
/// the hook capture only what it needs. A tool reaches the handle through
/// [`CallContext::agentcat`](crate::CallContext::agentcat), so it needs no clone.
///
/// A drop on the delivery thread itself does not wait. A handle whose delivery thread never
/// started has nothing to drain. The drain does not run at all after `std::process::exit` or a
/// panic under `panic = "abort"`.
#[derive(Clone)]
pub struct AgentCat {
    inner: Arc<Inner>,
}

impl fmt::Debug for AgentCat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.inner.debug_fields(f, "AgentCat")
    }
}

/// What the clones of one [`AgentCat`] share.
pub(crate) struct Inner {
    /// The project that receives the events; `""` means telemetry only (B-003).
    pub(crate) project_id: String,
    pub(crate) options: Options,
    /// Whether the SDK is on for this handle (C45). `AGENTCAT_ENABLED` turns it off: then the
    /// [`Tracked`] servers forward every method untouched and the sink drops everything.
    pub(crate) enabled: bool,
    /// Where the SDK hands its own log lines, its records and its events.
    pub(crate) sink: Arc<dyn Sink>,
    /// What the listings taught the SDK, one registry per inner handler type (C19).
    registries: Mutex<HashMap<TypeId, ToolRegistry>>,
}

impl fmt::Debug for Inner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.debug_fields(f, "Inner")
    }
}

impl Inner {
    /// `Debug` output under the name `name` with the project ID and the options only, not the
    /// registries or the sink.
    fn debug_fields(&self, f: &mut fmt::Formatter<'_>, name: &str) -> fmt::Result {
        f.debug_struct(name)
            .field("project_id", &self.project_id)
            .field("options", &self.options)
            .finish_non_exhaustive()
    }

    /// Runs `f` on the registry of the handler type `H`, made on first use. A poisoned lock is
    /// taken over. The lock is held for `f` only, never across an `.await`.
    pub(crate) fn with_registry<H: 'static, R>(&self, f: impl FnOnce(&mut ToolRegistry) -> R) -> R {
        let mut registries = self
            .registries
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        f(registries.entry(TypeId::of::<H>()).or_default())
    }
}

/// The line TS writes when no project ID is given (B-003).
const NO_PROJECT_WARNING: &str =
    "Warning: No projectId provided and no exporters configured. Events will not be sent anywhere.";

/// What a handle posts with, the lines it logs first and where its worker writes them.
struct Transport {
    publisher: Arc<dyn Publisher>,
    startup_lines: Vec<String>,
    /// Where the worker writes its lines; `None` is the process logger.
    write: Option<LineWriter>,
}

impl Transport {
    /// The transport of the handles that [`AgentCat::new`] and [`AgentCat::from_env`] build: the
    /// real one, [`Transport::api`].
    #[cfg(not(test))]
    fn process(options: &Options, env: &dyn Env) -> Self {
        Self::api(options, env)
    }

    /// The real transport: the process logger is set to the destination of `options` (the first
    /// handle wins, D38) and events go to the API (C44), with the home directory and the API URL
    /// of `env`. A unit test reaches it through `AgentCat::with_api_transport` only.
    fn api(options: &Options, env: &dyn Env) -> Self {
        logging::configure(options.log_to(), env.home_dir());
        let mut startup_lines = Vec::new();
        let publisher = ApiPublisher::new(resolve_api_base_url(options, env), &mut startup_lines);
        Self {
            publisher: Arc::new(publisher),
            startup_lines,
            write: None,
        }
    }

    /// The transport of a handle that [`AgentCat::new`] builds in a unit test (P15): events are
    /// accepted and dropped, nothing reaches a network, and the process logger stays as it is
    /// (unset, so the real home directory is not written to). A test that needs the real
    /// publisher builds it itself, or its handle with [`AgentCat::with_api_transport`], against a
    /// loopback server and a temp home.
    #[cfg(test)]
    fn process(options: &Options, env: &dyn Env) -> Self {
        let _ = (options, env);
        Self {
            publisher: Arc::new(OfflinePublisher),
            startup_lines: Vec::new(),
            write: None,
        }
    }
}

/// A publisher that accepts every event and sends none (P15; also the test fake's, C43).
#[cfg(any(test, feature = "test-util"))]
#[derive(Debug)]
struct OfflinePublisher;

#[cfg(any(test, feature = "test-util"))]
impl Publisher for OfflinePublisher {
    fn publish(
        &self,
        _request: agentcat_api::models::PublishEventRequest,
    ) -> crate::sink::BoxFuture<'static, Result<(), String>> {
        Box::pin(std::future::ready(Ok(())))
    }
}

/// The environment that [`AgentCat::new`] and [`AgentCat::from_env`] read: the process's.
#[cfg(not(test))]
fn default_env() -> ProcessEnv {
    ProcessEnv
}

/// The environment that [`AgentCat::new`] and [`AgentCat::from_env`] read in unit tests: an empty
/// one, so an `AGENTCAT_ENABLED` or `AGENTCAT_PROJECT_ID` in the developer's shell cannot change
/// the handles of tests (P17), as [`Transport::process`] keeps them off the network and the log
/// file. A test that needs a variable builds its handle with [`AgentCat::with_env`].
#[cfg(test)]
fn default_env() -> MapEnv {
    MapEnv::new(&[])
}

/// The sink of a handle that `AGENTCAT_ENABLED` turned off (C45): it drops every line, record and
/// event, so nothing starts a thread, and it answers `flush` and `shutdown` with `true`.
#[derive(Debug)]
struct DisabledSink;

impl Sink for DisabledSink {
    fn log(&self, _message: String) {}

    fn record(&self, _record: CallRecord) {}

    fn event(&self, _job: EventJob) {}
}

/// The sink of a handle that publishes: a [`Delivery`] for `project_id` with the redaction hooks
/// of `options`, posting through `publisher`.
#[cfg(test)]
fn delivery_sink(
    project_id: &str,
    options: &Options,
    publisher: Arc<dyn Publisher>,
    config: DeliveryConfig,
) -> Arc<dyn Sink> {
    Arc::new(Delivery::new(
        project_id.to_owned(),
        options,
        publisher,
        None,
        config,
    ))
}

const _: () = {
    const fn send_sync_static<T: Send + Sync + 'static>() {}
    send_sync_static::<AgentCat>();
};

impl AgentCat {
    /// A handle for `project_id`. Building one cannot fail and starts no thread until the first
    /// event or log line. An empty `project_id` means telemetry only: events are built but never
    /// sent to `AgentCat`.
    ///
    /// The environment variable [`AGENTCAT_ENABLED`](crate::AGENTCAT_ENABLED) set to `false`, `0`,
    /// `no` or `off` (any case, surrounding white space ignored) turns the handle off: it never
    /// starts a thread, its [`Tracked`] servers forward every method to the wrapped handler
    /// untouched (a tool still gets its [`CallContext`](crate::CallContext), with no session and
    /// no agent), custom events return `Ok(())` at once and publish nothing (also for an empty
    /// project ID), and the lifecycle methods return `true`. The same holds for [`track`] and
    /// [`AgentCat::from_env`].
    pub fn new(project_id: impl Into<String>, options: Options) -> Self {
        Self::build(project_id.into(), options, &default_env())
    }

    /// A handle like [`AgentCat::new`] for the project that the environment variable
    /// [`AGENTCAT_PROJECT_ID`] names. When it is unset or empty, the handle is telemetry only, as
    /// `AgentCat::new` with an empty project ID.
    ///
    /// ```no_run
    /// use agentcat_sdk::{AgentCat, Options};
    ///
    /// // AGENTCAT_PROJECT_ID=proj_123 in the environment.
    /// let agentcat = AgentCat::from_env(Options::default());
    /// # let _ = agentcat;
    /// ```
    pub fn from_env(options: Options) -> Self {
        Self::from_env_in(options, &default_env())
    }

    /// [`AgentCat::from_env`] in the environment `env` (D36).
    fn from_env_in(options: Options, env: &dyn Env) -> Self {
        let project_id = env.var(AGENTCAT_PROJECT_ID).unwrap_or_default();
        Self::build(project_id, options, env)
    }

    /// A handle like [`AgentCat::new`] in the environment `env`.
    #[cfg(test)]
    pub(crate) fn with_env(project_id: &str, options: Options, env: &dyn Env) -> Self {
        Self::build(project_id.to_owned(), options, env)
    }

    /// A handle like [`AgentCat::with_env`] with the transport that `AgentCat::new` uses outside
    /// unit tests ([`Transport::api`]), for a test that points `env` at a loopback server and a
    /// temp home.
    #[cfg(test)]
    pub(crate) fn with_api_transport(project_id: &str, options: Options, env: &dyn Env) -> Self {
        Self::build_with(project_id.to_owned(), options, env, Transport::api)
    }

    /// [`AgentCat::new`] in the environment `env`: a handle that is off when `env` turns the SDK
    /// off (C45, D44), else one that publishes through the process transport.
    fn build(project_id: String, options: Options, env: &dyn Env) -> Self {
        Self::build_with(project_id, options, env, Transport::process)
    }

    /// [`AgentCat::build`] with the transport that `transport` makes.
    fn build_with(
        project_id: String,
        options: Options,
        env: &dyn Env,
        transport: fn(&Options, &dyn Env) -> Transport,
    ) -> Self {
        if !sdk_enabled(env) {
            return Self::from_inner(Inner {
                project_id,
                options,
                enabled: false,
                sink: Arc::new(DisabledSink),
                registries: Mutex::default(),
            });
        }
        let transport = transport(&options, env);
        Self::assemble(project_id, options, transport, DeliveryConfig::default())
    }

    /// A handle whose events go through a [`Delivery`] over `transport`. An empty `project_id`
    /// queues TS's warning (B-003), then the transport's own lines; the first line starts the
    /// worker, which writes it.
    fn assemble(
        project_id: String,
        options: Options,
        transport: Transport,
        config: DeliveryConfig,
    ) -> Self {
        let project = project_id.clone();
        let delivery = match transport.write {
            None => Delivery::new(project, &options, transport.publisher, None, config),
            Some(write) => {
                Delivery::with_writer(project, &options, transport.publisher, None, config, write)
            }
        };
        if project_id.is_empty() {
            delivery.log(NO_PROJECT_WARNING.to_owned());
        }
        for line in transport.startup_lines {
            delivery.log(line);
        }
        Self::from_inner(Inner {
            project_id,
            options,
            enabled: true,
            sink: Arc::new(delivery),
            registries: Mutex::default(),
        })
    }

    /// A handle like [`AgentCat::new`] whose events go through a delivery queue that posts with
    /// `publisher` and uses `config`.
    #[cfg(test)]
    pub(crate) fn with_delivery(
        project_id: &str,
        options: Options,
        publisher: Arc<dyn Publisher>,
        config: DeliveryConfig,
    ) -> Self {
        let sink = delivery_sink(project_id, &options, publisher, config);
        Self::with_sink(project_id, options, sink)
    }

    /// A handle for the test fake (C43, P18): always enabled, whatever `AGENTCAT_ENABLED` says;
    /// it never configures the process logger; its publisher never sends; `observer` is told
    /// about every event that finishes the pipeline. Built here, not through [`Transport`],
    /// because a customer's tests compile the library without `cfg(test)`.
    #[cfg(feature = "test-util")]
    pub(crate) fn offline_observed(
        project_id: String,
        options: Options,
        observer: Arc<dyn Observer>,
    ) -> Self {
        let delivery = Delivery::new(
            project_id.clone(),
            &options,
            Arc::new(OfflinePublisher),
            Some(observer),
            DeliveryConfig::default(),
        );
        Self::from_inner(Inner {
            project_id,
            options,
            enabled: true,
            sink: Arc::new(delivery),
            registries: Mutex::default(),
        })
    }

    /// A handle like [`AgentCat::new`] that hands its log lines to `sink`.
    #[cfg(test)]
    pub(crate) fn with_sink(project_id: &str, options: Options, sink: Arc<dyn Sink>) -> Self {
        Self::from_inner(Inner {
            project_id: project_id.to_owned(),
            options,
            enabled: true,
            sink,
            registries: Mutex::default(),
        })
    }

    /// A handle that `AGENTCAT_ENABLED` turned off (C45), whose sink is `sink`, so a test sees
    /// that nothing reaches it.
    #[cfg(test)]
    pub(crate) fn disabled_with_sink(
        project_id: &str,
        options: Options,
        sink: Arc<dyn Sink>,
    ) -> Self {
        Self::from_inner(Inner {
            project_id: project_id.to_owned(),
            options,
            enabled: false,
            sink,
            registries: Mutex::default(),
        })
    }

    fn from_inner(inner: Inner) -> Self {
        Self {
            inner: Arc::new(inner),
        }
    }

    /// Wraps `handler` so that this handle sees its tool listings and tool calls. Cheap: it
    /// clones the handle and does no other work.
    pub fn track<H: ServerHandler>(&self, handler: H) -> Tracked<H> {
        Tracked::new(handler, self.clone())
    }

    /// Publishes a custom event to the project `project_id` for the session `session_id`, as
    /// TypeScript's `publishCustomEvent(sessionId, projectId, eventData)` does (B-009, C11). It
    /// only queues the event, so it returns at once.
    ///
    /// The session is the event's own [`session_id`](CustomEvent::session_id) when it is not
    /// empty, else `session_id` verbatim (it may be empty for none). The event has no server or
    /// client fields and goes through no redaction hook. It is queued even while tracing is off.
    /// A handle that `AGENTCAT_ENABLED` turned off returns `Ok(())` at once and publishes nothing.
    ///
    /// ```no_run
    /// use agentcat_sdk::{AgentCat, CustomEvent, Options};
    ///
    /// let agentcat = AgentCat::new("proj_123", Options::default());
    /// let event = CustomEvent::new().resource_name("cache-warmed").message("40 keys");
    /// agentcat.publish_custom_event("ses_1", "proj_123", event)?;
    /// # Ok::<(), agentcat_sdk::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::ProjectIdRequired`] when `project_id` is empty, except on a handle that
    /// `AGENTCAT_ENABLED` turned off, which returns `Ok(())` and does nothing.
    pub fn publish_custom_event(
        &self,
        session_id: &str,
        project_id: &str,
        event: CustomEvent,
    ) -> Result<(), Error> {
        publish_string(self, session_id, project_id, event, SystemTime::now())
    }

    /// What the clones of this handle share.
    pub(crate) fn inner(&self) -> &Inner {
        &self.inner
    }

    /// Waits until every event this handle took so far is delivered, for at most `timeout`;
    /// `true` when that happened in time. Events taken while it waits are waited for too.
    ///
    /// The SDK's delivery thread keeps the time, so the calling runtime needs no timer. Events
    /// still go out after a `false`. Use it before a short-lived process ends, or in a test.
    ///
    /// It returns `true` at once when the handle has taken no event yet, and after a
    /// [`shutdown`](Self::shutdown) has ended, also one that timed out: nothing is left to
    /// deliver. Once polled, the wait stays with the delivery thread until the events are
    /// delivered or `timeout` passes, also when the future is dropped first (for example by a
    /// timeout).
    ///
    /// ```no_run
    /// # async fn run(agentcat: agentcat_sdk::AgentCat) {
    /// use std::time::Duration;
    ///
    /// let delivered = agentcat.flush(Duration::from_secs(5)).await;
    /// # let _ = delivered;
    /// # }
    /// ```
    pub async fn flush(&self, timeout: Duration) -> bool {
        self.inner.sink.flush(timeout).await
    }

    /// Like [`flush`](Self::flush), then stops this handle for good: every event it takes from
    /// the call on is dropped, for all its clones. When `timeout` passes first, the events left
    /// are abandoned and it returns `false`.
    ///
    /// Once polled, the shutdown runs to its end, also when the future is dropped first (for
    /// example by a timeout): the handle drops later events and stops once its events are
    /// delivered or `timeout` passes. A second `shutdown` after the first has ended returns
    /// `true` at once.
    ///
    /// Without it, the drop of the last clone runs the same shutdown with a 5 s timeout and
    /// blocks the dropping thread while it runs: see [dropping the last
    /// handle](AgentCat#dropping-the-last-handle). `std::process::exit` and a panic under
    /// `panic = "abort"` skip that drain.
    pub async fn shutdown(&self, timeout: Duration) -> bool {
        self.inner.sink.shutdown(timeout).await
    }

    /// [`flush`](Self::flush) that blocks the calling thread for at most `timeout` (and a few
    /// milliseconds more). It needs no tokio runtime; on a runtime thread it blocks that thread.
    pub fn flush_blocking(&self, timeout: Duration) -> bool {
        self.inner.sink.flush_blocking(timeout)
    }

    /// [`shutdown`](Self::shutdown) that blocks the calling thread for at most `timeout` (and a
    /// few milliseconds more). It needs no tokio runtime; on a runtime thread it blocks that
    /// thread.
    pub fn shutdown_blocking(&self, timeout: Duration) -> bool {
        self.inner.sink.shutdown_blocking(timeout)
    }
}

/// Wraps `handler` with a new [`AgentCat`] handle for `project_id`: the one-line setup for a
/// server with one handler, such as a stdio server. The handle is [`AgentCat::new`]'s, so
/// `AGENTCAT_ENABLED` turns it off in the same way.
///
/// The returned server holds the only clone of the handle, so its drop drains the events and can
/// block the dropping thread for up to 5 s: see [dropping the last
/// handle](AgentCat#dropping-the-last-handle). On a current-thread runtime, clone the handle with
/// [`Tracked::agentcat`] before you serve, and call its [`shutdown`](AgentCat::shutdown) before
/// the runtime ends.
///
/// ```no_run
/// use agentcat_sdk::{Options, track};
/// use rmcp::ServiceExt;
///
/// # struct MyServer;
/// # impl rmcp::ServerHandler for MyServer {}
/// # async fn run(transport: tokio::io::DuplexStream) -> Result<(), Box<dyn std::error::Error>> {
/// let server = track(MyServer, "proj_123", Options::default());
/// let running = server.serve(transport).await?;
/// running.waiting().await?;
/// # Ok(())
/// # }
/// ```
pub fn track<H: ServerHandler>(
    handler: H,
    project_id: impl Into<String>,
    options: Options,
) -> Tracked<H> {
    AgentCat::new(project_id, options).track(handler)
}

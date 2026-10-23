//! [`AgentCat`]: the handle that every [`Tracked`] server of a project shares, and [`track`].

use std::any::TypeId;
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use rmcp::ServerHandler;

use crate::config::Options;
use crate::delivery::{Delivery, DeliveryConfig, Publisher};
use crate::registry::ToolRegistry;
use crate::sink::{DiscardSink, Sink};
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
    /// Where the SDK hands its own log lines.
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

/// The sink of a handle that publishes: a [`Delivery`] for `project_id` with the redaction hooks
/// of `options`, posting through `publisher`.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "called by AgentCat::new (plan 4 task 8)")
)]
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
    /// A handle for `project_id`. Building one cannot fail and starts nothing. An empty
    /// `project_id` means telemetry only: events are built but never sent to `AgentCat`.
    pub fn new(project_id: impl Into<String>, options: Options) -> Self {
        Self::from_inner(Inner {
            project_id: project_id.into(),
            options,
            sink: Arc::new(DiscardSink),
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

    /// A handle like [`AgentCat::new`] that hands its log lines to `sink`.
    #[cfg(test)]
    pub(crate) fn with_sink(project_id: &str, options: Options, sink: Arc<dyn Sink>) -> Self {
        Self::from_inner(Inner {
            project_id: project_id.to_owned(),
            options,
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
    /// Without it, the drop of the last clone runs the same shutdown with a 5 s timeout.
    /// `std::process::exit` and a panic under `panic = "abort"` skip that drain.
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
/// server with one handler, such as a stdio server.
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

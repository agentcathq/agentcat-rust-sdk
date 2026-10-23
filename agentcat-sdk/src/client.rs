//! [`AgentCat`]: the handle that every [`Tracked`] server of a project shares, and [`track`].

use std::any::TypeId;
use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use rmcp::ServerHandler;

use crate::config::Options;
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

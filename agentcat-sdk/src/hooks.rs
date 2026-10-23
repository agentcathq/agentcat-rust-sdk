//! The customer hooks of [`Options`]: the [`ToolCall`] snapshot they receive and [`fire`], which
//! starts them for one tool call (B-022 step 7, B-026). The hooks run as `Send` tasks on the
//! tokio runtime the call runs on (C14, C26), and the call never waits for them: plan 4 settles
//! the [`PendingHooks`] when it assembles the event. `rmcp/local` is not supported (C27).
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use http::request::Parts;
use rmcp::RoleServer;
use rmcp::model::{CallToolRequestParams, JsonObject};
use rmcp::service::RequestContext;
use serde_json::{Map, Value};
use tokio::runtime::Handle;
use tokio::task::{JoinError, JoinHandle};

use crate::config::{Hook, HookError, Options, UserIdentity};
use crate::event::{
    ClientIdentity, project_headers, protocol_version, redact_headers, transport_session_id,
};
use crate::sink::Sink;

#[cfg(test)]
mod tests;

/// One tool call as a hook sees it (D35): the request as the client sent it, with the injected
/// arguments (`session_id`, `agent_id`, `context`) still in it, and what the SDK knows about the
/// client and the transport. Clones are independent copies.
///
/// Its `Debug` output hides credentials: the values of the `authorization`,
/// `proxy-authorization`, `cookie` and `set-cookie` headers print as `"<redacted>"`, and the
/// request context prints as its request ID and whether it holds HTTP request parts.
///
/// ```
/// # use agentcat_sdk::{Options, ToolCall};
/// let options = Options::default().event_tags(|call: ToolCall| async move {
///     let mut tags = serde_json::Map::new();
///     tags.insert("tool".into(), call.tool_name().into());
///     Ok(Some(tags))
/// });
/// # let _ = options;
/// ```
#[derive(Clone)]
pub struct ToolCall {
    tool_name: String,
    arguments: Option<JsonObject>,
    meta: JsonObject,
    headers: Option<Map<String, Value>>,
    client: Option<ClientIdentity>,
    protocol_version: Option<String>,
    transport_session_id: Option<String>,
    request_context: RequestContext<RoleServer>,
}

impl ToolCall {
    /// The snapshot of `request` before the SDK changes it. `client` is the identity of the
    /// request's client ([`client_identity`](crate::event::client_identity)).
    pub(crate) fn new(
        request: &CallToolRequestParams,
        context: &RequestContext<RoleServer>,
        client: Option<&ClientIdentity>,
    ) -> Self {
        let meta: &JsonObject = &context.meta;
        Self {
            tool_name: request.name.to_string(),
            arguments: request.arguments.clone(),
            meta: meta.clone(),
            headers: context
                .extensions
                .get::<Parts>()
                .map(|parts| project_headers(&parts.headers)),
            client: client.cloned(),
            protocol_version: protocol_version(meta),
            transport_session_id: transport_session_id(context),
            request_context: context.clone(),
        }
    }

    /// The name of the tool.
    #[must_use]
    pub fn tool_name(&self) -> &str {
        &self.tool_name
    }

    /// The arguments as the client sent them, the `session_id`, `agent_id` and `context` that the
    /// SDK added to the tool's listing included.
    #[must_use]
    pub fn arguments(&self) -> Option<&JsonObject> {
        self.arguments.as_ref()
    }

    /// The `_meta` of the request; empty when the client sent none.
    #[must_use]
    pub fn meta(&self) -> &JsonObject {
        &self.meta
    }

    /// The HTTP headers of the request, every header included, `authorization` and `cookie` too
    /// (D06), as TS's WHATWG `Headers` gives them: names in lower case, in byte order, after the
    /// names that are array indexes in numeric order; values trimmed of leading and trailing HTTP
    /// whitespace; the values of a repeated name joined in arrival order with `", "` (`"; "` for
    /// `cookie`), except `set-cookie`, which keeps only its last value; a value that is not UTF-8
    /// read lossily. `None` on a transport without HTTP, such as stdio.
    #[must_use]
    pub fn headers(&self) -> Option<&Map<String, Value>> {
        self.headers.as_ref()
    }

    /// The name the client gave itself, when it did.
    #[must_use]
    pub fn client_name(&self) -> Option<&str> {
        self.client.as_ref()?.name.as_deref()
    }

    /// The version the client gave itself, when it did.
    #[must_use]
    pub fn client_version(&self) -> Option<&str> {
        self.client.as_ref()?.version.as_deref()
    }

    /// The protocol version the request names in its `_meta`, when it does (C28).
    #[must_use]
    pub fn protocol_version(&self) -> Option<&str> {
        self.protocol_version.as_deref()
    }

    /// The `mcp-session-id` of the streamable HTTP session; `None` on stdio and on a stateless
    /// HTTP server.
    #[must_use]
    pub fn transport_session_id(&self) -> Option<&str> {
        self.transport_session_id.as_deref()
    }

    /// The `rmcp` request context of the call, for anything the other getters do not give. The
    /// SDK has not yet added its [`CallContext`](crate::CallContext) to it.
    #[must_use]
    pub fn request_context(&self) -> &RequestContext<RoleServer> {
        &self.request_context
    }
}

impl fmt::Debug for ToolCall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToolCall")
            .field("tool_name", &self.tool_name)
            .field("arguments", &self.arguments)
            .field("meta", &self.meta)
            .field("headers", &self.headers.as_ref().map(redact_headers))
            .field("client", &self.client)
            .field("protocol_version", &self.protocol_version)
            .field("transport_session_id", &self.transport_session_id)
            .field("request_context", &ContextSummary(&self.request_context))
            .finish()
    }
}

/// A request context in `Debug` output: its request ID and whether it holds HTTP request parts,
/// never the parts themselves, whose headers carry credentials.
struct ContextSummary<'a>(&'a RequestContext<RoleServer>);

impl fmt::Debug for ContextSummary<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RequestContext")
            .field("id", &self.0.id)
            .field("http_parts", &self.0.extensions.get::<Parts>().is_some())
            .finish_non_exhaustive()
    }
}

/// What a hook gives back once it settles.
type Settled<T> = Result<Option<T>, HookError>;

/// The tasks of the hooks started for one call, `None` for a hook that is not set or did not
/// start. Dropping it aborts every task that has not finished (RD-35's reason: a detached task
/// would leak), as when the queue drops the event (overflow, a shutdown deadline). Take a handle
/// out with `Option::take` to keep its task.
#[derive(Debug, Default)]
pub(crate) struct PendingHooks {
    /// The `resolve_session_id` task; only in hook mode.
    pub(crate) resolve_session_id: Option<JoinHandle<Settled<String>>>,
    /// The `identify` task.
    pub(crate) identify: Option<JoinHandle<Settled<UserIdentity>>>,
    /// The `event_tags` task.
    pub(crate) event_tags: Option<JoinHandle<Settled<Map<String, Value>>>>,
    /// The `event_properties` task.
    pub(crate) event_properties: Option<JoinHandle<Settled<Map<String, Value>>>>,
}

impl PendingHooks {
    /// Whether any hook started.
    pub(crate) fn any(&self) -> bool {
        self.resolve_session_id.is_some()
            || self.identify.is_some()
            || self.event_tags.is_some()
            || self.event_properties.is_some()
    }
}

impl Drop for PendingHooks {
    fn drop(&mut self) {
        // `abort` does nothing to a task that has finished.
        if let Some(task) = &self.resolve_session_id {
            task.abort();
        }
        if let Some(task) = &self.identify {
            task.abort();
        }
        if let Some(task) = &self.event_tags {
            task.abort();
        }
        if let Some(task) = &self.event_properties {
            task.abort();
        }
    }
}

/// A task handle that aborts its task when it is dropped, so a customer hook whose event is
/// dropped (a cancelled pipeline, a shutdown deadline) does not keep running detached (RD-35's
/// reason). Awaiting it awaits the task.
#[derive(Debug)]
pub(crate) struct AbortOnDrop<T>(JoinHandle<T>);

impl<T> AbortOnDrop<T> {
    /// Takes over `task`.
    pub(crate) fn new(task: JoinHandle<T>) -> Self {
        Self(task)
    }
}

impl<T> Future for AbortOnDrop<T> {
    type Output = Result<T, JoinError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.0).poll(cx)
    }
}

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        // `abort` does nothing to a task that has finished.
        self.0.abort();
    }
}

/// Starts the hooks of `options` for `call`, in TS's order: `resolve_session_id` (in hook mode
/// only), `identify`, `event_tags`, `event_properties` (B-022 step 7). Each hook runs as a task on
/// the current tokio runtime (C14), and none is awaited (B-026). A panic in a hook stays in its
/// task. Without a runtime no hook starts, and `sink` gets one log line for each hook that would
/// have.
pub(crate) fn fire(
    options: &Options,
    call: &ToolCall,
    hook_mode: bool,
    sink: &dyn Sink,
) -> PendingHooks {
    let handle = Handle::try_current().ok();
    let start = Start {
        handle: handle.as_ref(),
        call,
        sink,
    };
    PendingHooks {
        resolve_session_id: start.hook(
            "resolveSessionId",
            options.resolve_session_id_hook().filter(|_| hook_mode),
        ),
        identify: start.hook("identify", options.identify_hook()),
        event_tags: start.hook("eventTags", options.event_tags_hook()),
        event_properties: start.hook("eventProperties", options.event_properties_hook()),
    }
}

/// What starting one hook needs.
struct Start<'a> {
    handle: Option<&'a Handle>,
    call: &'a ToolCall,
    sink: &'a dyn Sink,
}

impl Start<'_> {
    /// Spawns `hook`, if it is set, on the runtime; without a runtime, logs that it did not start.
    fn hook<T: Send + 'static>(
        &self,
        label: &str,
        hook: Option<&Hook<T>>,
    ) -> Option<JoinHandle<Settled<T>>> {
        let hook = hook?;
        let Some(handle) = self.handle else {
            self.sink.log(format!(
                "Warning: {label} hook not started - no tokio runtime"
            ));
            return None;
        };
        Some(handle.spawn(hook.call(self.call.clone())))
    }
}

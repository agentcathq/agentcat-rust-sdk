//! The view of a tool call that customer hooks receive.

use std::sync::Arc;

use rmcp::RoleServer;
use rmcp::model::{CallToolRequestParams, JsonObject, RequestMetaObject};
use rmcp::service::RequestContext;
use serde_json::Value;

use crate::hooks::{BoxFuture, UserIdentity};

pub(crate) type SessionHook = Arc<dyn Fn(ToolCallRequest) -> BoxFuture<Option<String>> + Send + Sync>;
pub(crate) type IdentifyHook = Arc<dyn Fn(ToolCallRequest) -> BoxFuture<Option<UserIdentity>> + Send + Sync>;
pub(crate) type TagsHook =
    Arc<dyn Fn(ToolCallRequest) -> BoxFuture<Option<Vec<(String, String)>>> + Send + Sync>;
pub(crate) type PropertiesHook = Arc<dyn Fn(ToolCallRequest) -> BoxFuture<Option<Value>> + Send + Sync>;

/// A tool call as the `identify`, `resolve_session_id`, `event_tags` and
/// `event_properties` hooks see it: the raw request (AgentCat's injected
/// parameters included) and the handler's request context.
///
/// Cheap to clone.
#[derive(Clone)]
pub struct ToolCallRequest {
    inner: Arc<Inner>,
}

struct Inner {
    params: CallToolRequestParams,
    context: RequestContext<RoleServer>,
}

impl std::fmt::Debug for ToolCallRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolCallRequest")
            .field("tool_name", &self.tool_name())
            .field("arguments", &self.arguments())
            .finish_non_exhaustive()
    }
}

impl ToolCallRequest {
    pub(crate) fn new(params: CallToolRequestParams, context: RequestContext<RoleServer>) -> Self {
        Self {
            inner: Arc::new(Inner { params, context }),
        }
    }

    /// The tool's name.
    pub fn tool_name(&self) -> &str {
        &self.inner.params.name
    }

    /// The raw arguments, including AgentCat's injected `session_id`,
    /// `agent_id` and `context`.
    pub fn arguments(&self) -> Option<&JsonObject> {
        self.inner.params.arguments.as_ref()
    }

    /// One raw argument.
    pub fn argument(&self, name: &str) -> Option<&Value> {
        self.arguments()?.get(name)
    }

    /// The raw request parameters.
    pub fn params(&self) -> &CallToolRequestParams {
        &self.inner.params
    }

    /// The handler's request context: request ID, `_meta`, extensions and
    /// the peer.
    pub fn context(&self) -> &RequestContext<RoleServer> {
        &self.inner.context
    }

    /// The request's `_meta`.
    pub fn meta(&self) -> &RequestMetaObject {
        &self.inner.context.meta
    }

    /// The HTTP request head, when the call arrived over streamable HTTP.
    /// Its `extensions` carry what your HTTP middleware inserted, such as
    /// authenticated claims.
    pub fn http_request(&self) -> Option<&http::request::Parts> {
        self.inner.context.extensions.get::<http::request::Parts>()
    }

    /// An HTTP request header as text (case-insensitive name). `None` on
    /// stdio, for a missing header, or for a value that is not valid text.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.http_request()?.headers.get(name)?.to_str().ok()
    }
}

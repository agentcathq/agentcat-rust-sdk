//! The record of a tool call, [`CallRecord`], and the fields of it that come from the request:
//! `parameters.request` (B-082), `parameters.extra` (D34, D06), the client identity (B-085, C22)
//! and the protocol version of the handle tags (B-067, C28). `event/tests.rs` checks the client
//! identity and the protocol version against `tests/vectors/event.json`, and the shapes of
//! `request` and `extra`.
use std::fmt;
use std::time::SystemTime;

use http::header::{COOKIE, HOST, SET_COOKIE};
use http::request::Parts;
use http::uri::PathAndQuery;
use http::{HeaderMap, HeaderName};
use rmcp::RoleServer;
use rmcp::model::{CallToolRequestParams, Implementation, JsonObject};
use rmcp::service::RequestContext;
use serde_json::{Map, Value};

use crate::constants::{META_CLIENT_INFO_KEY, META_PROTOCOL_VERSION_KEY};
use crate::handles::HandleResolution;
use crate::hooks::PendingHooks;
use crate::js;

mod public;
#[cfg(test)]
mod tests;

pub use public::Event;

/// The JSON-RPC method of a tool call.
const CALL_TOOL_METHOD: &str = "tools/call";

/// The JSON-RPC version of every message.
const JSONRPC_VERSION: &str = "2.0";

/// The key of a request's metadata, in its params and in `extra`.
const META_KEY: &str = "_meta";

/// The header of the transport session of streamable HTTP, as `http` names it (lower case).
const SESSION_ID_HEADER: &str = "mcp-session-id";

/// The headers that carry credentials. The data keeps them (D06); `Debug` output hides their
/// values ([`redact_headers`]).
const CREDENTIAL_HEADERS: [&str; 4] = [
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
];

/// What `Debug` output shows in place of a credential header's value.
const REDACTED: &str = "<redacted>";

/// The client a request names (B-085): its name and version, each when it is a string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClientIdentity {
    /// The client's name.
    pub(crate) name: Option<String>,
    /// The client's version.
    pub(crate) version: Option<String>,
}

impl From<&Implementation> for ClientIdentity {
    fn from(client: &Implementation) -> Self {
        Self {
            name: Some(client.name.clone()),
            version: Some(client.version.clone()),
        }
    }
}

/// One tool call as the SDK records it (B-077). The call path builds it and hands it to the
/// [`Sink`](crate::sink::Sink); plan 4 assembles the published event from it. Its `Debug` output
/// hides the values of the credential headers in `extra` ([`redact_headers`]).
pub(crate) struct CallRecord {
    /// When the call started: the event's `timestamp`.
    pub(crate) started_at: SystemTime,
    /// How long the call took, in milliseconds: the event's `duration`.
    pub(crate) duration_ms: u64,
    /// The session handles of the call (B-052 to B-069).
    pub(crate) resolution: HandleResolution,
    /// The tool name, or `Unknown Tool` for an empty one: the event's `resource_name`.
    pub(crate) resource_name: String,
    /// `parameters.request`: [`request_value`].
    pub(crate) request: Value,
    /// `parameters.extra`: [`extra_value`].
    pub(crate) extra: Value,
    /// The token estimate of the raw arguments (B-070 to B-076).
    pub(crate) input_tokens: Option<i32>,
    /// The token estimate of the customer's result; none without a result.
    pub(crate) output_tokens: Option<i32>,
    /// The SDK's own tags (B-025, B-096).
    pub(crate) tags: Map<String, Value>,
    /// The `context` argument as sent (B-083, K-08).
    pub(crate) user_intent: Option<Value>,
    /// The customer's original result (B-059); none for a JSON-RPC error or a panic.
    pub(crate) response: Option<Value>,
    /// Whether the call failed (B-097, RD-05).
    pub(crate) is_error: bool,
    /// The event's `error` (B-097, RD-06, C23).
    pub(crate) error: Option<Map<String, Value>>,
    /// The name in the inner handler's `server_info` (B-086).
    pub(crate) server_name: Option<String>,
    /// The version in the inner handler's `server_info` (B-086).
    pub(crate) server_version: Option<String>,
    /// The client of the call: [`client_identity`].
    pub(crate) client: Option<ClientIdentity>,
    /// The customer hooks started for the call (B-022 step 7).
    pub(crate) pending: PendingHooks,
}

impl fmt::Debug for CallRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CallRecord")
            .field("started_at", &self.started_at)
            .field("duration_ms", &self.duration_ms)
            .field("resolution", &self.resolution)
            .field("resource_name", &self.resource_name)
            .field("request", &self.request)
            .field("extra", &redact_extra(&self.extra))
            .field("input_tokens", &self.input_tokens)
            .field("output_tokens", &self.output_tokens)
            .field("tags", &self.tags)
            .field("user_intent", &self.user_intent)
            .field("response", &self.response)
            .field("is_error", &self.is_error)
            .field("error", &self.error)
            .field("server_name", &self.server_name)
            .field("server_version", &self.server_version)
            .field("client", &self.client)
            .field("pending", &self.pending)
            .finish()
    }
}

/// `parameters.request` of a `tools/call` request, the raw JSON-RPC request TS records (B-082):
/// `method`, `params`, `jsonrpc` and `id`, in that order. `params` is `params` as rmcp serializes
/// it, with the raw arguments (the injected parameters included), and the request's `_meta` last
/// when it sent one. rmcp moves that `_meta` out of the params into `context.meta`, so a `_meta`
/// key the params still hold is dropped.
pub(crate) fn request_value(
    params: &CallToolRequestParams,
    context: &RequestContext<RoleServer>,
) -> Value {
    let mut fields: Map<String, Value> = match serde_json::to_value(params) {
        Ok(Value::Object(fields)) => fields
            .into_iter()
            .filter(|(key, _)| key != META_KEY)
            .collect(),
        // rmcp's params always serialize to an object.
        _ => Map::new(),
    };
    if let Some(meta) = request_meta(context) {
        fields.insert(META_KEY.to_owned(), meta);
    }
    let mut request = Map::new();
    request.insert(
        "method".to_owned(),
        Value::String(CALL_TOOL_METHOD.to_owned()),
    );
    request.insert("params".to_owned(), Value::Object(fields));
    request.insert(
        "jsonrpc".to_owned(),
        Value::String(JSONRPC_VERSION.to_owned()),
    );
    request.insert("id".to_owned(), context.id.clone().into_json_value());
    Value::Object(request)
}

/// `parameters.extra` of a request (D34), with the keys in this order: `sessionId`, the
/// `mcp-session-id` header of a stateful HTTP request; `requestId`, the JSON-RPC id as sent;
/// `_meta`, when the request sent one; and `requestInfo`, when rmcp's streamable HTTP server put
/// the HTTP request parts in the context ([`request_info`]). On stdio it has no `sessionId` and no
/// `requestInfo`.
pub(crate) fn extra_value(context: &RequestContext<RoleServer>) -> Value {
    let parts = context.extensions.get::<Parts>();
    let mut extra = Map::new();
    if let Some(session) = transport_session_id(context) {
        extra.insert("sessionId".to_owned(), Value::String(session));
    }
    extra.insert("requestId".to_owned(), context.id.clone().into_json_value());
    if let Some(meta) = request_meta(context) {
        extra.insert(META_KEY.to_owned(), meta);
    }
    if let Some(parts) = parts {
        extra.insert("requestInfo".to_owned(), Value::Object(request_info(parts)));
    }
    Value::Object(extra)
}

/// The `mcp-session-id` header of a stateful HTTP request, as lossy text: `extra.sessionId`.
pub(crate) fn transport_session_id(context: &RequestContext<RoleServer>) -> Option<String> {
    let parts = context.extensions.get::<Parts>()?;
    let session = parts.headers.get(SESSION_ID_HEADER)?;
    Some(lossy(session.as_bytes()))
}

/// The `requestInfo` of `extra`: `{headers, url}`, the [`project_headers`] of the request and its
/// [`request_url`], without `url` when the request has none.
fn request_info(parts: &Parts) -> Map<String, Value> {
    let mut info = Map::new();
    info.insert(
        "headers".to_owned(),
        Value::Object(project_headers(&parts.headers)),
    );
    if let Some(url) = request_url(parts) {
        info.insert("url".to_owned(), Value::String(url));
    }
    info
}

/// The request headers as `extra` holds them (D06), as TS records them: `Object.fromEntries` over
/// a WHATWG `Headers` of the request (B-081). Every header, `Authorization` and `Cookie` included,
/// is under its lower-case name. Each value loses its leading and trailing HTTP whitespace; the
/// values of a repeated name are joined in arrival order with `", "` (`"; "` for `cookie`), and a
/// repeated `set-cookie` keeps its last value. The names are in byte order, except that names
/// which are array indexes come first in numeric order (JavaScript's key order). A value that is
/// not UTF-8 is read lossily.
pub(crate) fn project_headers(headers: &HeaderMap) -> Map<String, Value> {
    let mut names: Vec<&HeaderName> = headers.keys().collect();
    names.sort_unstable_by(|a, b| a.as_str().cmp(b.as_str()));
    let sorted: Map<String, Value> = names
        .into_iter()
        .map(|name| {
            let mut values = headers
                .get_all(name)
                .iter()
                .map(|value| trim_http_whitespace(&lossy(value.as_bytes())).to_owned());
            let value = if name == SET_COOKIE {
                values.next_back().unwrap_or_default()
            } else {
                let separator = if name == COOKIE { "; " } else { ", " };
                values.collect::<Vec<_>>().join(separator)
            };
            (name.as_str().to_owned(), Value::String(value))
        })
        .collect();
    js::own_entries(&sorted)
        .into_iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}

/// `headers`, a [`project_headers`] map, for `Debug` output: the value of each credential header
/// (`authorization`, `proxy-authorization`, `cookie`, `set-cookie`, in any case) becomes
/// `"<redacted>"`; the names and the other headers stay as they are.
pub(crate) fn redact_headers(headers: &Map<String, Value>) -> Map<String, Value> {
    headers
        .iter()
        .map(|(name, value)| {
            let credential = CREDENTIAL_HEADERS
                .iter()
                .any(|credential| name.eq_ignore_ascii_case(credential));
            let value = if credential {
                Value::String(REDACTED.to_owned())
            } else {
                value.clone()
            };
            (name.clone(), value)
        })
        .collect()
}

/// `extra`, an [`extra_value`], for `Debug` output: its `requestInfo.headers` through
/// [`redact_headers`].
fn redact_extra(extra: &Value) -> Value {
    let mut extra = extra.clone();
    if let Some(Value::Object(headers)) = extra.pointer_mut("/requestInfo/headers") {
        *headers = redact_headers(headers);
    }
    extra
}

/// `text` without its leading and trailing HTTP whitespace (tab, line feed, carriage return and
/// space), as `Headers` normalizes a value.
fn trim_http_whitespace(text: &str) -> &str {
    text.trim_matches(['\t', '\n', '\r', ' '])
}

/// The URL of a request: an absolute request URI (HTTP/2, a proxy request) as it is, else
/// `http://<Host><path and query>`; none for an origin-form request without a `Host` header.
pub(crate) fn request_url(parts: &Parts) -> Option<String> {
    let uri = &parts.uri;
    if uri.scheme().is_some() && uri.authority().is_some() {
        return Some(uri.to_string());
    }
    let host = parts.headers.get(HOST)?;
    let path = uri.path_and_query().map_or("/", PathAndQuery::as_str);
    Some(format!("http://{}{path}", lossy(host.as_bytes())))
}

/// TS's `narrowClientInfo` (B-085): a JSON object with a string `name` or a string `version` names
/// a client, and a field that is not a string becomes `None`. Anything else names none.
pub(crate) fn narrow_client_info(value: Option<&Value>) -> Option<ClientIdentity> {
    let Some(Value::Object(client)) = value else {
        return None;
    };
    let text = |key: &str| client.get(key).and_then(Value::as_str).map(str::to_owned);
    let (name, version) = (text("name"), text("version"));
    (name.is_some() || version.is_some()).then_some(ClientIdentity { name, version })
}

/// The client of a request (B-085): the client the request's `_meta` names (rmcp keeps TS's v2
/// envelope keys there), else the client of the peer's `initialize`. An HTTP request without an
/// `mcp-session-id` header skips the second (C22): rmcp's stateless mode fabricates that peer info
/// with its own name and version, and TS's per-request server never saw an `initialize` either.
pub(crate) fn client_identity(context: &RequestContext<RoleServer>) -> Option<ClientIdentity> {
    if let Some(client) = narrow_client_info(context.meta.get(META_CLIENT_INFO_KEY)) {
        return Some(client);
    }
    let stateless = context
        .extensions
        .get::<Parts>()
        .is_some_and(|parts| !parts.headers.contains_key(SESSION_ID_HEADER));
    if stateless {
        return None;
    }
    let info = context.peer.peer_info()?;
    Some(ClientIdentity::from(&info.client_info))
}

/// The protocol version a request names in its `_meta`, the value of the
/// `agentcat_protocol_version` tag (B-067): a non-empty string under
/// `io.modelcontextprotocol/protocolVersion`. Only the `_meta` counts (C28), never the version the
/// session negotiated.
pub(crate) fn protocol_version(meta: &JsonObject) -> Option<String> {
    match meta.get(META_PROTOCOL_VERSION_KEY) {
        Some(Value::String(version)) if !version.is_empty() => Some(version.clone()),
        _ => None,
    }
}

/// The request's `_meta` as JSON, when it sent a non-empty one.
fn request_meta(context: &RequestContext<RoleServer>) -> Option<Value> {
    let meta: &JsonObject = &context.meta;
    (!meta.is_empty()).then(|| Value::Object(meta.clone()))
}

/// `bytes` as text, each byte sequence that is not UTF-8 replaced by U+FFFD.
fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

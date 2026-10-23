//! `event` against the TS SDK: the cases of `tests/vectors/event.json` (the client identity ladder
//! and the protocol version), and Rust-only tests of the shapes of `parameters.request` (B-082) and
//! `parameters.extra` (D34, D06), of the stateless HTTP identity (C22), of the records a tracked
//! server makes over HTTP, and of the [`RecordingSink`]'s records. The context tests run over an
//! in-memory connection and over streamable HTTP on a loopback port.
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use http::request::Parts;
use http::{HeaderMap, HeaderName, HeaderValue, Request};
use rmcp::RoleServer;
use rmcp::model::{
    CallToolRequestParams, ClientCapabilities, Implementation, InitializeRequestParams, JsonObject,
    RequestId, RequestMetaObject,
};
use rmcp::service::{RequestContext, RoleClient, RunningService};
use rmcp::transport::streamable_http_server::StreamableHttpServerConfig;
use serde_json::{Map, Value, json};

use super::{
    CallRecord, ClientIdentity, client_identity, extra_value, project_headers, protocol_version,
    request_info, request_url, request_value,
};
use crate::client::AgentCat;
use crate::config::Options;
use crate::constants::META_CLIENT_INFO_KEY;
use crate::handles::{HandleResolution, SessionSource};
use crate::hooks::PendingHooks;
use crate::sink::Sink;
use crate::test_support::rmcp::{
    CREDENTIAL_HEADERS, ContextEcho, RecordingSink, Sample, assert_credentials_hidden,
    bare_context, connect, connect_http, credential_parts, identity_json, serve_http,
};
use crate::test_support::vectors;

/// The case name prefixes of `event.json`.
const KINDS: [&str; 3] = ["client-info", "protocol-version", "headers"];

/// The kind (name prefix) of the case `name`.
fn kind_of(name: &str) -> &'static str {
    KINDS
        .into_iter()
        .find(|kind| {
            name.strip_prefix(kind)
                .is_some_and(|rest| rest.starts_with(':'))
        })
        .unwrap_or_else(|| panic!("{name}: unknown kind"))
}

/// The `_meta` of a case input: its `meta` object, or an empty one when it has none (rmcp's
/// `RequestContext::meta` is always an object).
fn meta_of(input: &Value) -> JsonObject {
    match input.get("meta") {
        None => JsonObject::new(),
        Some(Value::Object(meta)) => meta.clone(),
        Some(other) => panic!("the meta {other} is not an object"),
    }
}

/// The client of TS's `getClientInfoForRequest` result as the event reads it: its `name` and
/// `version` (B-077), or `null` for `undefined`. TS hands back a handshake client whole, its other
/// fields included.
fn ts_client(output: Option<&Value>) -> Value {
    let Some(output) = output else {
        return Value::Null;
    };
    let picked: Map<String, Value> = ["name", "version"]
        .into_iter()
        .filter_map(|key| Some((key.to_owned(), output.get(key)?.clone())))
        .collect();
    Value::Object(picked)
}

/// A "client-info:" case: [`client_identity`] of a context with the case's `_meta` and, as the
/// peer's handshake info, its `initialize` client. The context has no HTTP request parts (stdio).
fn check_client_info(input: &Value, output: Option<&Value>, name: &str) {
    let peer = input.get("initialize").map(|client| {
        let client: Implementation = serde_json::from_value(client.clone())
            .unwrap_or_else(|e| panic!("{name}: rmcp cannot hold the client {client}: {e}"));
        InitializeRequestParams::new(ClientCapabilities::default(), client)
    });
    let mut context = bare_context(peer);
    let meta: &mut JsonObject = &mut context.meta;
    meta.extend(meta_of(input));
    let actual = identity_json(client_identity(&context).as_ref());
    assert_eq!(actual, ts_client(output), "{name}");
}

/// A "headers:" case: [`project_headers`] of a header map that holds the case's `[name, value]`
/// pairs in their order, against TS's object, in its key order too.
fn check_headers(input: &Value, output: Option<&Value>, name: &str) {
    let mut headers = HeaderMap::new();
    for pair in input.as_array().unwrap() {
        let header = HeaderName::from_bytes(pair[0].as_str().unwrap().as_bytes())
            .unwrap_or_else(|e| panic!("{name}: http refuses the name {}: {e}", pair[0]));
        let value = HeaderValue::from_str(pair[1].as_str().unwrap())
            .unwrap_or_else(|e| panic!("{name}: http refuses the value {}: {e}", pair[1]));
        headers.append(header, value);
    }
    let actual = Value::Object(project_headers(&headers));
    let ts = output.unwrap_or_else(|| panic!("{name}: no output"));
    assert_eq!(
        serde_json::to_string(&actual).unwrap(),
        serde_json::to_string(ts).unwrap(),
        "{name}"
    );
}

#[tokio::test]
async fn vectors_match_ts() {
    let mut seen = BTreeSet::new();
    for case in vectors::cases("event") {
        let name = vectors::name(&case);
        let kind = kind_of(name);
        seen.insert(kind);
        let (input, output) = (&case["input"], case.get("output"));
        match kind {
            "client-info" => check_client_info(input, output, name),
            "protocol-version" => {
                let actual = protocol_version(&meta_of(input)).map(Value::String);
                assert_eq!(actual.as_ref(), output, "{name}");
            }
            _ => check_headers(input, output, name),
        }
    }
    assert_eq!(
        seen,
        BTreeSet::from(KINDS),
        "event.json must have cases of every kind"
    );
}

/// The keys of the JSON object `value`, in order.
fn keys(value: &Value) -> Vec<&str> {
    value
        .as_object()
        .unwrap_or_else(|| panic!("{value} is not an object"))
        .keys()
        .map(String::as_str)
        .collect()
}

/// The arguments the echo calls send: the customer's `msg` and the injected parameters.
fn raw_arguments() -> JsonObject {
    let arguments =
        json!({"msg": "hi", "session_id": "ses_1", "agent_id": "agt_1", "context": "why"});
    arguments.as_object().unwrap().clone()
}

/// Calls the tool `echo` of a [`ContextEcho`] with [`raw_arguments`] and returns its structured
/// content.
async fn call_echo(client: &RunningService<RoleClient, InitializeRequestParams>) -> Value {
    let params = CallToolRequestParams::new("echo").with_arguments(raw_arguments());
    let result = client.call_tool(params).await.expect("the call succeeds");
    result
        .structured_content
        .expect("ContextEcho answers with structured content")
}

/// B-082: `method`, `params`, `jsonrpc`, `id` in that order; `params` holds the raw arguments,
/// injected parameters included, and the context's `_meta` last, only when the request sent one.
/// A `_meta` left in the params is not the request's: rmcp moves the request's into the context.
#[tokio::test]
async fn request_value_has_the_b082_keys() {
    let mut params = CallToolRequestParams::new("add_todo").with_arguments(raw_arguments());
    let mut stale = RequestMetaObject::new();
    stale.insert("stale".to_owned(), json!(true));
    params.meta = Some(stale);
    let mut context = bare_context(None);
    context.id = RequestId::Number(7);
    let arguments = r#"{"msg":"hi","session_id":"ses_1","agent_id":"agt_1","context":"why"}"#;
    assert_eq!(
        serde_json::to_string(&request_value(&params, &context)).unwrap(),
        format!(
            r#"{{"method":"tools/call","params":{{"name":"add_todo","arguments":{arguments}}},"jsonrpc":"2.0","id":7}}"#
        )
    );

    context.id = RequestId::String("req-1".into());
    context.meta.insert("progressToken".to_owned(), json!(3));
    assert_eq!(
        serde_json::to_string(&request_value(&params, &context)).unwrap(),
        format!(
            r#"{{"method":"tools/call","params":{{"name":"add_todo","arguments":{arguments},"_meta":{{"progressToken":3}}}},"jsonrpc":"2.0","id":"req-1"}}"#
        )
    );

    // Over a connection, rmcp's client sends a `_meta` (its progress token) with every request.
    let client = connect(ContextEcho, "stdio-client").await;
    let request = call_echo(&client).await["request"].take();
    assert_eq!(keys(&request), ["method", "params", "jsonrpc", "id"]);
    assert_eq!(keys(&request["params"]), ["name", "arguments", "_meta"]);
    assert_eq!(
        request["params"]["arguments"],
        Value::Object(raw_arguments())
    );
    assert!(
        request["params"]["_meta"].get("progressToken").is_some(),
        "{request}"
    );
    assert!(request["id"].is_number(), "{request}");
    client.cancel().await.unwrap();
}

/// D34: on stdio (no HTTP request parts) `extra` is `{requestId, _meta}`: no `sessionId` and no
/// `requestInfo`.
#[tokio::test]
async fn extra_stdio_has_no_request_info() {
    let client = connect(ContextEcho, "stdio-client").await;
    let echo = call_echo(&client).await;
    let extra = &echo["extra"];
    assert_eq!(keys(extra), ["requestId", "_meta"]);
    assert_eq!(extra["requestId"], echo["request"]["id"]);
    assert_eq!(extra["_meta"], echo["request"]["params"]["_meta"]);
    client.cancel().await.unwrap();
}

/// B-085: on stdio the client of a legacy session is the client of its `initialize`.
#[tokio::test]
async fn stdio_client_is_the_handshake_client() {
    let client = connect(ContextEcho, "stdio-client").await;
    let echo = call_echo(&client).await;
    let expected = json!({"name": "stdio-client", "version": "1.0.0"});
    assert_eq!(echo["client"], expected);
    client.cancel().await.unwrap();
}

/// The request parts of a POST to `uri` with `headers`, in that order.
fn parts(uri: &str, headers: &[(&str, &[u8])]) -> Parts {
    let mut builder = Request::builder().method("POST").uri(uri);
    for (name, value) in headers {
        builder = builder.header(*name, HeaderValue::from_bytes(value).unwrap());
    }
    builder.body(()).unwrap().into_parts().0
}

/// D06: `requestInfo` is `{headers, url}`, with every header of the request under its lower-case
/// name and the values of a repeated header joined with ", " in arrival order. (The order of the
/// names, and TS's other rules, are the "headers:" cases of `vectors_match_ts`.)
#[test]
fn extra_joins_repeated_headers() {
    let parts = parts(
        "/mcp",
        &[
            ("Host", b"localhost:8080"),
            ("X-Multi", b"a"),
            ("accept", b"application/json"),
            ("x-multi", b"b, c"),
            ("X-MULTI", b"d"),
            ("Authorization", b"Bearer t"),
        ],
    );
    let info = Value::Object(request_info(&parts));
    assert_eq!(keys(&info), ["headers", "url"]);
    let mut names = keys(&info["headers"]);
    names.sort_unstable();
    assert_eq!(names, ["accept", "authorization", "host", "x-multi"]);
    assert_eq!(info["headers"]["x-multi"], "a, b, c, d");
    assert_eq!(info["headers"]["authorization"], "Bearer t");
    assert_eq!(info["url"], "http://localhost:8080/mcp");
}

/// D06: a header value that is not UTF-8 is read lossily (each bad byte becomes U+FFFD), also in
/// the `Host` of the URL.
#[test]
fn extra_non_utf8_header_is_lossy() {
    let parts = parts(
        "/mcp",
        &[
            ("host", b"h\xe9st"),
            ("x-latin1", b"caf\xe9"),
            ("x-utf8", "café 😀".as_bytes()),
            ("x-latin1", b"\xff"),
        ],
    );
    let headers = project_headers(&parts.headers);
    assert_eq!(headers["x-latin1"], "caf\u{FFFD}, \u{FFFD}");
    assert_eq!(headers["x-utf8"], "café 😀");
    assert_eq!(
        request_url(&parts).as_deref(),
        Some("http://h\u{FFFD}st/mcp")
    );
}

/// D34: the URL of an origin-form request is `http://<Host><path and query>`; an absolute request
/// URI is the URL as it is, whatever the `Host` header says.
#[test]
fn extra_url_from_host() {
    let origin = parts("/mcp?x=1&y=%20", &[("host", b"example.com:8080")]);
    assert_eq!(
        request_url(&origin).as_deref(),
        Some("http://example.com:8080/mcp?x=1&y=%20")
    );
    let absolute = parts(
        "https://api.example.com/mcp?q=1",
        &[("host", b"other.example")],
    );
    assert_eq!(
        request_url(&absolute).as_deref(),
        Some("https://api.example.com/mcp?q=1")
    );
    let absolute_without_host = parts("http://127.0.0.1:9/mcp", &[]);
    assert_eq!(
        request_url(&absolute_without_host).as_deref(),
        Some("http://127.0.0.1:9/mcp")
    );
}

/// D34: an origin-form request without a `Host` header has no URL, so `requestInfo` has only its
/// headers.
#[test]
fn extra_no_host_no_url() {
    let parts = parts("/mcp", &[("x-a", b"1")]);
    assert_eq!(request_url(&parts), None);
    assert_eq!(
        Value::Object(request_info(&parts)),
        json!({"headers": {"x-a": "1"}})
    );
}

/// [`client_identity`] of `context` as [`identity_json`].
fn identity(context: &RequestContext<RoleServer>) -> Value {
    identity_json(client_identity(context).as_ref())
}

/// C22: the handshake client counts on stdio and on a request with an `mcp-session-id` header, not
/// on an HTTP request without one (rmcp fabricates that peer info). A `_meta` client counts
/// everywhere.
#[tokio::test]
async fn client_identity_skips_peer_info_on_stateless_http() {
    let handshake = || {
        let client = Implementation::new("handshake", "1.0.0");
        Some(InitializeRequestParams::new(
            ClientCapabilities::default(),
            client,
        ))
    };
    let peer = json!({"name": "handshake", "version": "1.0.0"});

    let stdio = bare_context(handshake());
    assert_eq!(identity(&stdio), peer);

    let mut stateful = bare_context(handshake());
    let session = parts("/mcp", &[("host", b"h"), ("mcp-session-id", b"s-1")]);
    stateful.extensions.insert(session);
    assert_eq!(identity(&stateful), peer);

    let mut stateless = bare_context(handshake());
    stateless
        .extensions
        .insert(parts("/mcp", &[("host", b"h")]));
    assert_eq!(identity(&stateless), Value::Null);

    let modern = json!({"name": "modern", "version": "2.0.0"});
    stateless
        .meta
        .insert(META_CLIENT_INFO_KEY.to_owned(), modern.clone());
    assert_eq!(identity(&stateless), modern);
}

/// The header the HTTP test clients send on every request, and its value (TS's header-capture
/// test sends the same).
const SENTINEL: (&str, &str) = ("x-agentcat-health-run", "hcrun:header-capture");

/// The [`SENTINEL`] header as the HTTP client takes it.
fn sentinel() -> HashMap<HeaderName, HeaderValue> {
    let (name, value) = SENTINEL;
    HashMap::from([(
        HeaderName::from_static(name),
        HeaderValue::from_static(value),
    )])
}

/// D34 over legacy stateful HTTP: `extra` is `{sessionId, requestId, _meta, requestInfo}`;
/// `sessionId` is the `mcp-session-id` header; `requestInfo` has the headers the client sent and
/// the URL it called. The client is the client of the session's `initialize` (B-085).
#[tokio::test]
async fn stateful_http_record_has_session_and_client() {
    let config = StreamableHttpServerConfig::default().with_json_response(true);
    let url = serve_http(ContextEcho, config).await;
    let client = connect_http(&url, "http-client", sentinel()).await;
    let echo = call_echo(&client).await;
    let extra = &echo["extra"];
    assert_eq!(
        keys(extra),
        ["sessionId", "requestId", "_meta", "requestInfo"]
    );
    let info = &extra["requestInfo"];
    let session = info["headers"]["mcp-session-id"].as_str().unwrap();
    assert_ne!(session, "");
    assert_eq!(extra["sessionId"], session);
    assert_eq!(info["headers"][SENTINEL.0], SENTINEL.1);
    assert_eq!(info["url"], url.as_str());
    let expected = json!({"name": "http-client", "version": "1.0.0"});
    assert_eq!(echo["client"], expected);
    client.cancel().await.unwrap();
}

/// C22 over stateless HTTP (`legacy_session_mode(false)`) with a legacy client: no `sessionId`,
/// and no client, although rmcp's peer info names rmcp itself (`03-rmcp.md` §5.2).
#[tokio::test]
async fn stateless_identity_is_unknown() {
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true);
    let url = serve_http(ContextEcho, config).await;
    let client = connect_http(&url, "http-client", sentinel()).await;
    let echo = call_echo(&client).await;
    let extra = &echo["extra"];
    assert_eq!(keys(extra), ["requestId", "_meta", "requestInfo"]);
    let headers = &extra["requestInfo"]["headers"];
    assert_eq!(headers.get("mcp-session-id"), None);
    assert_eq!(headers[SENTINEL.0], SENTINEL.1);
    assert_eq!(echo["peer"]["name"], "rmcp", "rmcp's fabricated peer info");
    assert_eq!(echo["client"], Value::Null);
    client.cancel().await.unwrap();
}

/// A tracked [`Sample`] on a handle whose records go to the returned [`RecordingSink`], served
/// over streamable HTTP with `config`; returns the URL too.
async fn serve_tracked_sample(config: StreamableHttpServerConfig) -> (String, Arc<RecordingSink>) {
    let sink = Arc::new(RecordingSink::default());
    let agentcat = AgentCat::with_sink("", Options::default(), sink.clone());
    let url = serve_http(agentcat.track(Sample), config).await;
    (url, sink)
}

/// Calls the tool `echo` of a [`Sample`] with [`raw_arguments`] and returns the one record the
/// call left in `sink`.
async fn record_of_echo(
    client: &RunningService<RoleClient, InitializeRequestParams>,
    sink: &RecordingSink,
) -> CallRecord {
    let params = CallToolRequestParams::new("echo").with_arguments(raw_arguments());
    client.call_tool(params).await.expect("the call succeeds");
    let mut records = sink.take_records();
    assert_eq!(records.len(), 1, "one record: {records:?}");
    records.remove(0)
}

/// The record-level twin of [`stateful_http_record_has_session_and_client`]: the record of a
/// tracked call over legacy stateful HTTP has the D34 `extra` with the session header, the client's
/// headers and URL, and the session's `initialize` client (B-085).
#[tokio::test]
async fn tracked_stateful_http_record_has_session_and_client() {
    let config = StreamableHttpServerConfig::default().with_json_response(true);
    let (url, sink) = serve_tracked_sample(config).await;
    let client = connect_http(&url, "http-client", sentinel()).await;
    let record = record_of_echo(&client, &sink).await;
    let extra = &record.extra;
    assert_eq!(
        keys(extra),
        ["sessionId", "requestId", "_meta", "requestInfo"]
    );
    let info = &extra["requestInfo"];
    let session = info["headers"]["mcp-session-id"].as_str().unwrap();
    assert_ne!(session, "");
    assert_eq!(extra["sessionId"], session);
    assert_eq!(info["headers"][SENTINEL.0], SENTINEL.1);
    assert_eq!(info["url"], url.as_str());
    let expected = json!({"name": "http-client", "version": "1.0.0"});
    assert_eq!(identity_json(record.client.as_ref()), expected);
    client.cancel().await.unwrap();
}

/// The record-level twin of [`stateless_identity_is_unknown`] (C22): the record of a tracked call
/// over stateless HTTP from a legacy client has no `sessionId` and no client.
#[tokio::test]
async fn tracked_stateless_record_has_no_client() {
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true);
    let (url, sink) = serve_tracked_sample(config).await;
    let client = connect_http(&url, "http-client", sentinel()).await;
    let record = record_of_echo(&client, &sink).await;
    let extra = &record.extra;
    assert_eq!(keys(extra), ["requestId", "_meta", "requestInfo"]);
    let headers = &extra["requestInfo"]["headers"];
    assert_eq!(headers.get("mcp-session-id"), None);
    assert_eq!(headers[SENTINEL.0], SENTINEL.1);
    assert_eq!(record.client, None);
    client.cancel().await.unwrap();
}

/// A record whose every field depends on `n`.
fn record(n: u64) -> CallRecord {
    CallRecord {
        started_at: UNIX_EPOCH + Duration::from_millis(n),
        duration_ms: n,
        resolution: HandleResolution {
            session_id: format!("ses_{n}"),
            session_source: SessionSource::Supplied,
            hook_mode: false,
            agent_id: Some(format!("agt_{n}")),
        },
        resource_name: format!("tool_{n}"),
        request: json!({"n": n}),
        extra: json!({"requestId": n}),
        input_tokens: Some(1),
        output_tokens: None,
        tags: Map::from_iter([("n".to_owned(), json!(n))]),
        user_intent: Some(json!("why")),
        response: None,
        is_error: true,
        error: Some(Map::from_iter([("message".to_owned(), json!("failed"))])),
        server_name: Some("server".to_owned()),
        server_version: None,
        client: Some(ClientIdentity {
            name: Some(format!("client_{n}")),
            version: None,
        }),
        pending: PendingHooks::default(),
    }
}

/// Every field of `record`, as JSON.
fn fields(record: CallRecord) -> Value {
    let CallRecord {
        started_at,
        duration_ms,
        resolution,
        resource_name,
        request,
        extra,
        input_tokens,
        output_tokens,
        tags,
        user_intent,
        response,
        is_error,
        error,
        server_name,
        server_version,
        client,
        pending: _,
    } = record;
    let started_ms = started_at.duration_since(SystemTime::UNIX_EPOCH).unwrap();
    json!({
        "started_at": u64::try_from(started_ms.as_millis()).unwrap(),
        "duration_ms": duration_ms,
        "resolution": format!("{resolution:?}"),
        "resource_name": resource_name,
        "request": request,
        "extra": extra,
        "input_tokens": input_tokens,
        "output_tokens": output_tokens,
        "tags": tags,
        "user_intent": user_intent,
        "response": response,
        "is_error": is_error,
        "error": error,
        "server_name": server_name,
        "server_version": server_version,
        "client": identity_json(client.as_ref()),
    })
}

/// The [`RecordingSink`] keeps the records it gets, in order, and `take_records` drains them.
#[test]
fn recording_sink_takes_records_in_order() {
    let sink = RecordingSink::default();
    sink.record(record(1));
    sink.record(record(2));
    let taken: Vec<Value> = sink.take_records().into_iter().map(fields).collect();
    assert_eq!(taken, [fields(record(1)), fields(record(2))]);
    assert!(sink.take_records().is_empty());
}

/// D06 keeps the credential headers in `extra`, and the record's `Debug` output hides them as the
/// [`ToolCall`](crate::ToolCall)'s does: the values of `authorization`, `proxy-authorization`,
/// `cookie` and `set-cookie` print as `<redacted>` under their names, other headers and the other
/// fields print as they are. Rust-only: TS has no `Debug`.
#[tokio::test]
async fn call_record_debug_hides_credentials() {
    let mut context = bare_context(None);
    context.extensions.insert(credential_parts());
    let mut record = record(7);
    record.extra = extra_value(&context);

    let text = format!("{record:?}");
    assert_credentials_hidden(&text);
    assert!(text.contains("<redacted>"), "{text}");
    assert!(text.contains("tool_7"), "the other fields print: {text}");
    assert!(text.contains("http://localhost:8080/mcp"), "{text}");
    let headers = &record.extra["requestInfo"]["headers"];
    for (name, value) in CREDENTIAL_HEADERS {
        assert_eq!(headers[name], value, "D06: the data keeps {name}");
    }
}

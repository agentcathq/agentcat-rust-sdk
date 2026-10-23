//! `Tracked<H>` forwards: each `ServerHandler` method that a client can reach answers through the
//! wrapper exactly as through the handler alone. The notification methods and `get_tool` (only
//! the streamable HTTP transport calls it) are forwarded too; a client cannot call them. A handle
//! that `AGENTCAT_ENABLED` turned off forwards the methods the SDK otherwise changes (C45).
use std::sync::Arc;
use std::time::Duration;

use rmcp::ServerHandler;
use rmcp::model::{
    ArgumentInfo, CallToolRequest, CallToolRequestParams, CancelTaskParams, CancelTaskRequest,
    ClientCapabilities, ClientRequest, CompleteRequest, CompleteRequestParams, CustomRequest,
    DiscoverRequest, DiscoverRequestParams, GetPromptRequest, GetPromptRequestParams,
    GetTaskParams, GetTaskRequest, Implementation, InitializeRequestParams, InputResponses,
    ListPromptsRequest, ListResourceTemplatesRequest, ListResourcesRequest, ListToolsRequest,
    PingRequest, ProtocolVersion, ReadResourceRequest, ReadResourceRequestParams, Reference,
    Request, RequestMetaObject, SubscribeRequestParams, SubscriptionFilter,
    SubscriptionsListenRequest, SubscriptionsListenRequestParams, UnsubscribeRequestParams,
    UpdateTaskParams, UpdateTaskRequest,
};
use rmcp::model::{ListToolsResult, PaginatedRequestParams, Tool};
use rmcp::service::{RoleClient, RunningService, ServiceError};
use serde_json::{Value, json};

use crate::client::{AgentCat, track};
use crate::config::Options;
use crate::constants::GET_MORE_TOOLS_NAME;
use crate::inject::{InjectConfig, InjectState, build_injected_list, inject_tool};
use crate::test_support::rmcp::{
    ContextEcho, PAGED_CURSOR, PROBE_VERSIONS, Paged, Probe, RecordingSink, Sample, connect,
};

/// How long one request may take: a `listen` that is not forwarded waits for cancellation, and
/// the test fails instead of hanging.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(10);

/// How an answer of the [`Probe`] served alone shows that the request reached the [`Probe`]'s
/// own method and not rmcp's default.
#[derive(Debug, Clone, Copy)]
enum Reached {
    /// The answer has the text `probe <method>`.
    Says,
    /// The answer is a success; rmcp's default answers `-32601`.
    Succeeds,
    /// The answer refuses the protocol version and lists [`PROBE_VERSIONS`] as supported.
    RefusesVersion,
}

impl Reached {
    /// Whether `answer`, the plain Probe's answer to the request for `method`, shows this.
    fn by(self, answer: &Value, method: &str) -> bool {
        match self {
            Self::Says => answer.to_string().contains(&format!("probe {method}")),
            Self::Succeeds => answer.get("result").is_some(),
            Self::RefusesVersion => {
                answer.pointer("/error/data/supported") == Some(&json!(PROBE_VERSIONS))
            }
        }
    }
}

/// One request of the forwarding run.
struct Case {
    /// The `ServerHandler` method the request is for.
    method: &'static str,
    request: ClientRequest,
    reached: Reached,
}

impl Case {
    fn new(method: &'static str, request: ClientRequest, reached: Reached) -> Self {
        Self {
            method,
            request,
            reached,
        }
    }
}

/// A request `_meta` with the 2026-07-28 per-request client context, which `server/discover`
/// and `subscriptions/listen` need.
fn inline_context() -> RequestMetaObject {
    RequestMetaObject::with_client_context(
        ProtocolVersion::V_2026_07_28,
        Implementation::new("forwarding-test", "1.0.0"),
        ClientCapabilities::default(),
    )
}

/// The requests without a `_meta`, one for each method they reach.
#[expect(
    deprecated,
    reason = "set_level takes SEP-2577-deprecated types on every rmcp 3.x"
)]
fn plain_cases() -> Vec<Case> {
    use rmcp::model::{LoggingLevel, SetLevelRequestParams};

    let uri = "file:///greeting.txt";
    vec![
        Case::new(
            "ping",
            ClientRequest::PingRequest(PingRequest::default()),
            Reached::Says,
        ),
        Case::new(
            "complete",
            ClientRequest::CompleteRequest(CompleteRequest::new(CompleteRequestParams::new(
                Reference::for_prompt("greeting"),
                ArgumentInfo::new("name", "A"),
            ))),
            Reached::Says,
        ),
        Case::new(
            "set_level",
            ClientRequest::SetLevelRequest(Request::new(SetLevelRequestParams::new(
                LoggingLevel::Info,
            ))),
            Reached::Succeeds,
        ),
        Case::new(
            "get_prompt",
            ClientRequest::GetPromptRequest(GetPromptRequest::new(GetPromptRequestParams::new(
                "greeting",
            ))),
            Reached::Says,
        ),
        Case::new(
            "list_prompts",
            ClientRequest::ListPromptsRequest(ListPromptsRequest::default()),
            Reached::Says,
        ),
        Case::new(
            "list_resources",
            ClientRequest::ListResourcesRequest(ListResourcesRequest::default()),
            Reached::Says,
        ),
        Case::new(
            "list_resource_templates",
            ClientRequest::ListResourceTemplatesRequest(ListResourceTemplatesRequest::default()),
            Reached::Says,
        ),
        Case::new(
            "read_resource",
            ClientRequest::ReadResourceRequest(ReadResourceRequest::new(
                ReadResourceRequestParams::new(uri),
            )),
            Reached::Says,
        ),
        Case::new(
            "subscribe",
            ClientRequest::SubscribeRequest(Request::new(SubscribeRequestParams::new(uri))),
            Reached::Succeeds,
        ),
        Case::new(
            "unsubscribe",
            ClientRequest::UnsubscribeRequest(Request::new(UnsubscribeRequestParams::new(uri))),
            Reached::Succeeds,
        ),
        Case::new(
            "call_tool",
            ClientRequest::CallToolRequest(CallToolRequest::new(CallToolRequestParams::new(
                "greet",
            ))),
            Reached::Says,
        ),
        Case::new(
            "list_tools",
            ClientRequest::ListToolsRequest(ListToolsRequest::default()),
            Reached::Says,
        ),
        Case::new(
            "on_custom_request",
            ClientRequest::CustomRequest(CustomRequest::new("custom/echo", Some(json!({"n": 1})))),
            Reached::Says,
        ),
    ]
}

/// The requests with a `_meta`, one for each method they reach.
fn meta_cases() -> Vec<Case> {
    // A version rmcp knows and the Probe does not support: refused only when the wrapper asks
    // the Probe's `supported_protocol_versions`.
    let mut old_version = PingRequest::default();
    let mut meta = RequestMetaObject::new();
    meta.set_protocol_version(ProtocolVersion::V_2024_11_05);
    old_version.extensions.insert(meta);

    let mut discover = DiscoverRequest::new(DiscoverRequestParams::default());
    discover.extensions.insert(inline_context());

    let mut listen = SubscriptionsListenRequest::new(SubscriptionsListenRequestParams::new(
        SubscriptionFilter::new(),
    ));
    listen.extensions.insert(inline_context());

    // The tasks extension declared for one request only, so the session keeps a plain client.
    let tasks = || {
        let mut meta = RequestMetaObject::new();
        meta.set_client_capabilities(ClientCapabilities::builder().enable_tasks().build());
        meta
    };
    let mut get_task = GetTaskRequest::new(GetTaskParams::new("task-1"));
    get_task.extensions.insert(tasks());
    let mut update_task =
        UpdateTaskRequest::new(UpdateTaskParams::new("task-1", InputResponses::new()));
    update_task.extensions.insert(tasks());
    let mut cancel_task = CancelTaskRequest::new(CancelTaskParams::new("task-1"));
    cancel_task.extensions.insert(tasks());

    vec![
        Case::new(
            "supported_protocol_versions",
            ClientRequest::PingRequest(old_version),
            Reached::RefusesVersion,
        ),
        Case::new(
            "discover",
            ClientRequest::DiscoverRequest(discover),
            Reached::Says,
        ),
        Case::new(
            "listen",
            ClientRequest::SubscriptionsListenRequest(listen),
            Reached::Says,
        ),
        Case::new(
            "get_task",
            ClientRequest::GetTaskRequest(get_task),
            Reached::Says,
        ),
        Case::new(
            "update_task",
            ClientRequest::UpdateTaskRequest(update_task),
            Reached::Succeeds,
        ),
        Case::new(
            "cancel_task",
            ClientRequest::CancelTaskRequest(cancel_task),
            Reached::Succeeds,
        ),
    ]
}

/// What `client` receives for `request`, as JSON: `{"result": ..}`, `{"error": ..}` (a JSON-RPC
/// error), `{"failure": ..}` (any other client error) or `{"timeout": ..}`.
async fn answer(
    client: &RunningService<RoleClient, InitializeRequestParams>,
    request: ClientRequest,
) -> Value {
    match tokio::time::timeout(ANSWER_TIMEOUT, client.send_request(request)).await {
        Err(_) => json!({"timeout": ANSWER_TIMEOUT.as_secs()}),
        Ok(Ok(result)) => json!({"result": result}),
        Ok(Err(ServiceError::McpError(error))) => json!({"error": error}),
        Ok(Err(other)) => json!({"failure": other.to_string()}),
    }
}

#[tokio::test]
async fn forwards_every_method() {
    let plain = connect(Probe, "forwarding-test").await;
    let tracked = connect(track(Probe, "", Options::default()), "forwarding-test").await;

    let info = plain.peer_info().expect("the plain Probe sent its info");
    assert_eq!(info.instructions.as_deref(), Some("probe"), "get_info");
    let title = info.server_info.as_ref().and_then(|s| s.title.as_deref());
    assert_eq!(title, Some("probe initialize"), "initialize");
    assert_eq!(tracked.peer_info(), plain.peer_info());

    // One request for each method a client can reach; `initialize` and `get_info` are checked
    // through the peer info above, and `listen` needs `accepted_subscription_filter`.
    for case in plain_cases().into_iter().chain(meta_cases()) {
        let expected = answer(&plain, case.request.clone()).await;
        assert!(
            case.reached.by(&expected, case.method),
            "{}: the plain Probe answered {expected}",
            case.method
        );
        let actual = answer(&tracked, case.request).await;
        assert_eq!(actual, expected, "{}", case.method);
    }

    plain.cancel().await.unwrap();
    tracked.cancel().await.unwrap();
}

#[test]
fn tracked_debug_names_the_inner_type() {
    /// A handler without a `Debug` impl.
    struct Opaque;
    impl ServerHandler for Opaque {}

    let text = format!("{:?}", track(Opaque, "", Options::default()));
    assert!(text.contains(std::any::type_name::<Opaque>()), "{text}");
}

/// The warning for a listing with no tools.
const EMPTY_WARNING: &str = "Warning: No tools found in the original list. This is likely due to the tools not being registered before AgentCat.track().";

/// A tracked handle that logs to a [`RecordingSink`].
fn recording_handle() -> (AgentCat, Arc<RecordingSink>) {
    let sink = Arc::new(RecordingSink::default());
    let agentcat = AgentCat::with_sink("", Options::default(), sink.clone());
    (agentcat, sink)
}

/// The tools `client` lists, page by page.
async fn all_tools(client: &RunningService<RoleClient, InitializeRequestParams>) -> Vec<Tool> {
    client.list_all_tools().await.expect("the listing works")
}

/// One listing page, for the cursor `cursor`.
async fn page(
    client: &RunningService<RoleClient, InitializeRequestParams>,
    cursor: Option<&str>,
) -> ListToolsResult {
    let params = PaginatedRequestParams::default().with_cursor(cursor.map(str::to_owned));
    client
        .list_tools(Some(params))
        .await
        .expect("the page lists")
}

#[tokio::test]
async fn listing_equals_the_pure_pipeline() {
    let plain = connect(Sample, "plain").await;
    let own = all_tools(&plain).await;
    let config = InjectConfig::from_options(&Options::default());
    let pure = build_injected_list(&config, own, &mut InjectState::default());
    let expected = pure.tools;

    let (agentcat, sink) = recording_handle();
    let tracked = connect(agentcat.track(Sample), "tracked").await;
    assert_eq!(all_tools(&tracked).await, expected);
    assert_eq!(sink.logs(), pure.logs);
    assert!(expected.iter().any(|t| t.name == GET_MORE_TOOLS_NAME));
}

#[tokio::test]
async fn empty_listing_is_untouched_and_logged() {
    let plain = connect(Probe, "plain").await;
    let expected = page(&plain, None).await;
    assert_eq!(expected.tools, Vec::<Tool>::new());

    let (agentcat, sink) = recording_handle();
    let tracked = connect(agentcat.track(Probe), "tracked").await;
    assert_eq!(page(&tracked, None).await, expected);
    assert_eq!(sink.logs(), [EMPTY_WARNING]);
    let listed = agentcat
        .inner()
        .with_registry::<Probe, _>(|registry| registry.listed());
    assert!(!listed, "the registry stays unset");
}

#[tokio::test]
async fn cursor_and_other_fields_pass_through() {
    let plain = connect(Paged, "plain").await;
    let expected = page(&plain, None).await;
    assert_eq!(expected.next_cursor.as_deref(), Some(PAGED_CURSOR));
    assert!(expected.meta.is_some());

    let tracked = connect(track(Paged, "", Options::default()), "tracked").await;
    let actual = page(&tracked, None).await;
    assert_eq!(actual.next_cursor, expected.next_cursor);
    assert_eq!(actual.meta, expected.meta);
    assert_eq!(actual.tools.len(), expected.tools.len() + 1);
}

#[tokio::test]
async fn get_more_tools_on_every_page() {
    let tracked = connect(track(Paged, "", Options::default()), "tracked").await;
    for (cursor, own) in [(None, "alpha"), (Some(PAGED_CURSOR), "beta")] {
        let names: Vec<String> = page(&tracked, cursor)
            .await
            .tools
            .iter()
            .map(|t| t.name.to_string())
            .collect();
        assert_eq!(names, [own, GET_MORE_TOOLS_NAME], "C20");
    }
}

#[test]
fn get_tool_returns_the_injected_schema() {
    let tracked = track(Sample, "", Options::default());
    let config = InjectConfig::from_options(&Options::default());

    // Before any listing, a customer tool comes through the pure pipeline.
    let own = Sample.get_tool("echo").expect("the macro serves get_tool");
    let (expected, _) = inject_tool(
        &config,
        own.clone(),
        &mut InjectState::default(),
        &mut Vec::new(),
    );
    assert_ne!(expected, own);
    assert_eq!(tracked.get_tool("echo"), Some(expected));

    // C02: the report-missing tool answers by name; an unknown name has none.
    let more = tracked.get_tool(GET_MORE_TOOLS_NAME).expect("injected");
    assert_eq!(more, crate::inject::get_more_tools_tool());
    assert_eq!(tracked.get_tool("nope"), None);

    // With report missing off, the name falls through to the handler, which has none.
    let off = track(Sample, "", Options::default().enable_report_missing(false));
    assert_eq!(off.get_tool(GET_MORE_TOOLS_NAME), None);
}

#[tokio::test]
async fn initialize_keeps_the_client_identity() {
    let tracked = Arc::new(track(Sample, "", Options::default()));
    assert_eq!(tracked.client_identity(), None);
    let _client = connect(tracked.clone(), "identity-test").await;
    assert_eq!(
        tracked.client_identity(),
        Some(Implementation::new("identity-test", "1.0.0"))
    );
    assert_eq!(
        (*tracked).clone().client_identity(),
        None,
        "a clone starts with none"
    );
}

#[tokio::test]
async fn registries_are_per_inner_type() {
    let agentcat = AgentCat::new("", Options::default());
    let sample = connect(agentcat.track(Sample), "sample").await;
    let paged = connect(agentcat.track(Paged), "paged").await;
    all_tools(&sample).await;
    all_tools(&paged).await;

    let has = |present: bool, name: &str, sample_side: bool| {
        let seen = if sample_side {
            agentcat
                .inner()
                .with_registry::<Sample, _>(|r| r.entry(name).is_some())
        } else {
            agentcat
                .inner()
                .with_registry::<Paged, _>(|r| r.entry(name).is_some())
        };
        assert_eq!(
            seen,
            present,
            "{name} in the {} registry",
            if sample_side { "Sample" } else { "Paged" }
        );
    };
    has(true, "echo", true);
    has(false, "alpha", true);
    has(true, "alpha", false);
    has(true, "beta", false);
    has(false, "echo", false);
}

#[tokio::test]
async fn get_tool_leaves_the_listing_logs_intact() {
    // The logs of a first listing on a fresh handle.
    let (fresh, fresh_sink) = recording_handle();
    all_tools(&connect(fresh.track(Sample), "fresh").await).await;
    let expected = fresh_sink.logs();
    assert!(
        expected.iter().any(|line| line.contains("own_session")),
        "the listing logs the session_id conflict"
    );

    // The same listing after lookups of the tool that declares its own session_id.
    let (agentcat, sink) = recording_handle();
    let tracked = agentcat.track(Sample);
    let (_, entry) = tracked
        .lookup_tool("own_session")
        .expect("listed by the macro");
    assert!(entry.declares_session, "C19");
    assert!(tracked.get_tool("own_session").is_some());
    assert!(sink.logs().is_empty(), "a lookup logs nothing");
    all_tools(&connect(tracked, "later").await).await;
    assert_eq!(sink.logs(), expected);
}

/// The request for the tool `name` with the arguments `arguments`, an object.
fn call_params(name: &str, arguments: Value) -> CallToolRequestParams {
    let Value::Object(arguments) = arguments else {
        panic!("{arguments} is not an object");
    };
    CallToolRequestParams::new(name.to_owned()).with_arguments(arguments)
}

/// What `client` receives for a call of `name` with `arguments`, as JSON (see [`answer`]).
async fn call_answer(
    client: &RunningService<RoleClient, InitializeRequestParams>,
    name: &str,
    arguments: Value,
) -> Value {
    let request = CallToolRequest::new(call_params(name, arguments));
    answer(client, ClientRequest::CallToolRequest(request)).await
}

/// C45, C29: a handle that `AGENTCAT_ENABLED` turned off forwards untouched. The listing is the
/// handler's own (no `get_more_tools`, no injected parameters), `get_tool` is the handler's, a
/// call reaches the handler with its raw arguments and a `CallContext` without session or agent,
/// `get_more_tools` is the handler's to answer, `initialize` keeps no client identity, and nothing
/// reaches the sink: no record, no log line.
#[tokio::test]
async fn disabled_handle_forwards_untouched() {
    let options = || Options::default().enable_agent_tracking(true);
    let sink = Arc::new(RecordingSink::default());
    let agentcat = AgentCat::disabled_with_sink("proj_1", options(), sink.clone());

    let tracked = Arc::new(agentcat.track(Sample));
    assert_eq!(tracked.get_tool("echo"), Sample.get_tool("echo"));
    assert_eq!(tracked.get_tool(GET_MORE_TOOLS_NAME), None);
    let plain = connect(Sample, "plain").await;
    let client = connect(tracked.clone(), "disabled").await;
    assert_eq!(all_tools(&client).await, all_tools(&plain).await);
    assert_eq!(tracked.client_identity(), None);

    let raw = json!({"text": "hi", "session_id": "start", "context": "Sent as is."});
    let refused = call_answer(&client, "strict", raw.clone()).await;
    assert_eq!(refused, call_answer(&plain, "strict", raw).await);
    assert!(refused.to_string().contains("unknown field"), "{refused}");
    let whoami = call_answer(&client, "whoami", json!({"session_id": "start"})).await;
    let text = whoami
        .pointer("/result/content/0/text")
        .and_then(Value::as_str);
    let expected = json!({"session_id": null, "agent_id": null}).to_string();
    assert_eq!(text, Some(expected.as_str()), "{whoami}");
    let more = json!({"context": "Looking for more tools."});
    let answered = call_answer(&client, GET_MORE_TOOLS_NAME, more.clone()).await;
    assert_eq!(
        answered,
        call_answer(&plain, GET_MORE_TOOLS_NAME, more).await
    );

    let echo = connect(agentcat.track(ContextEcho), "disabled-echo").await;
    let raw =
        json!({"msg": "hi", "session_id": "start", "agent_id": "opus|t|a1", "context": "why"});
    let echoed = call_answer(&echo, "echo", raw.clone()).await;
    assert_eq!(
        echoed.pointer("/result/structuredContent/request/params/arguments"),
        Some(&raw)
    );

    assert!(sink.logs().is_empty(), "{:?}", sink.logs());
    assert!(sink.take_records().is_empty());
    plain.cancel().await.unwrap();
    client.cancel().await.unwrap();
    echo.cancel().await.unwrap();
}

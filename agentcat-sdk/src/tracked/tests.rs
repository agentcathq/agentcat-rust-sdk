//! `Tracked<H>` forwards: each `ServerHandler` method that a client can reach answers through the
//! wrapper exactly as through the handler alone. The notification methods and `get_tool` (only
//! the streamable HTTP transport calls it) are forwarded too; a client cannot call them.
use std::time::Duration;

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
use rmcp::service::{RoleClient, RunningService, ServiceError};
use serde_json::{Value, json};

use crate::client::track;
use crate::config::Options;
use crate::test_support::rmcp::{PROBE_VERSIONS, Probe, connect};

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
    impl rmcp::ServerHandler for Opaque {}

    let text = format!("{:?}", track(Opaque, "", Options::default()));
    assert!(text.contains(std::any::type_name::<Opaque>()), "{text}");
}

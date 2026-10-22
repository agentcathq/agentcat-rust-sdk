//! rmcp helpers for the crate's tests: an in-memory client connection, the [`Probe`] server, whose
//! answers are sentinels, and the [`RecordingSink`].
use std::borrow::Cow;
use std::future::ready;
use std::sync::{Mutex, PoisonError};

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CancelTaskParams, ClientCapabilities,
    CompleteRequestParams, CompleteResult, CompletionInfo, CustomRequest, CustomResult,
    DiscoverResult, GetPromptRequestParams, GetPromptResponse, GetPromptResult, GetTaskParams,
    GetTaskResult, Implementation, InitializeRequestParams, InitializeResult, ListPromptsResult,
    ListResourceTemplatesResult, ListResourcesResult, ListToolsResult, MetaObject,
    PaginatedRequestParams, Prompt, ProtocolVersion, ReadResourceRequestParams,
    ReadResourceResponse, ReadResourceResult, Resource, ResourceContents, ResourceTemplate,
    ServerCapabilities, SubscribeRequestParams, SubscriptionFilter, UnsubscribeRequestParams,
    UpdateTaskParams,
};
use rmcp::service::{
    MaybeSendFuture, RequestContext, RoleClient, RunningService, SubscriptionContext,
};
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
use serde_json::json;

use crate::sink::Sink;

/// The capacity of the in-memory pipe between client and server, in bytes per direction.
const PIPE_BYTES: usize = 64 * 1024;

/// Serves `server` on one end of an in-memory pipe, on a spawned task, and connects a client to
/// the other end. The client is `client_name` version `1.0.0` and asks for protocol version
/// 2025-11-25, the newest with `initialize` on every rmcp 3.x minor, so the session is a legacy
/// one on each of them (`ping`, `subscribe` and `unsubscribe` reach the handler).
///
/// # Panics
///
/// When the server or the client fails to start.
pub(crate) async fn connect<S: ServerHandler>(
    server: S,
    client_name: &str,
) -> RunningService<RoleClient, InitializeRequestParams> {
    let (server_end, client_end) = tokio::io::duplex(PIPE_BYTES);
    tokio::spawn(async move {
        let running = server.serve(server_end).await.expect("the server starts");
        running.waiting().await.expect("the server stops cleanly");
    });
    InitializeRequestParams::new(
        ClientCapabilities::default(),
        Implementation::new(client_name, "1.0.0"),
    )
    .with_protocol_version(ProtocolVersion::V_2025_11_25)
    .serve(client_end)
    .await
    .expect("the client connects")
}

/// The protocol versions the [`Probe`] supports: fewer than rmcp knows, so a request for
/// another known version shows whether rmcp asked the Probe's `supported_protocol_versions`.
pub(crate) const PROBE_VERSIONS: &[ProtocolVersion] =
    &[ProtocolVersion::V_2025_11_25, ProtocolVersion::V_2026_07_28];

/// A server whose answers no rmcp default gives: each carries the text `probe <method>`, or is a
/// success where rmcp's default answers `-32601`. A test that sends the same requests to the
/// Probe with and without a wrapper can tell a forwarded call from rmcp's default.
///
/// Its `get_info` has the instructions `probe` and declares the tasks extension, so `tasks/*`
/// requests reach the handler. The answers of `call_tool`, `list_tools` and `initialize` stay the
/// same when a wrapper intercepts those methods as TS does: an error passes through unchanged,
/// an empty tool page gets nothing, and the initialize result is forwarded unchanged.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Probe;

/// The error the [`Probe`] answers `method` with.
fn probe_error(method: &str) -> ErrorData {
    ErrorData::internal_error(format!("probe {method}"), None)
}

impl ServerHandler for Probe {
    fn ping(
        &self,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<(), ErrorData>> + MaybeSendFuture + '_ {
        ready(Err(probe_error("ping")))
    }

    fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<InitializeResult, ErrorData>> + MaybeSendFuture + '_ {
        context.peer.set_peer_info(request);
        let mut info = self.get_info();
        info.server_info.title = Some("probe initialize".to_owned());
        ready(Ok(info))
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(PROBE_VERSIONS)
    }

    fn discover(
        &self,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<DiscoverResult, ErrorData>> + MaybeSendFuture + '_ {
        let mut result = DiscoverResult::from_server_info(PROBE_VERSIONS.to_vec(), self.get_info());
        result.instructions = Some("probe discover".to_owned());
        ready(Ok(result))
    }

    fn complete(
        &self,
        _request: CompleteRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CompleteResult, ErrorData>> + MaybeSendFuture + '_ {
        let values = vec!["probe complete".to_owned()];
        ready(
            CompletionInfo::with_all_values(values)
                .map(CompleteResult::new)
                .map_err(|e| ErrorData::internal_error(e, None)),
        )
    }

    #[expect(
        deprecated,
        reason = "SetLevelRequestParams is SEP-2577-deprecated on every rmcp 3.x"
    )]
    fn set_level(
        &self,
        _request: rmcp::model::SetLevelRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<(), ErrorData>> + MaybeSendFuture + '_ {
        ready(Ok(()))
    }

    fn get_prompt(
        &self,
        _request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<GetPromptResponse, ErrorData>> + MaybeSendFuture + '_ {
        let result = GetPromptResult::new(Vec::new()).with_description("probe get_prompt");
        ready(Ok(GetPromptResponse::Complete(result)))
    }

    fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListPromptsResult, ErrorData>> + MaybeSendFuture + '_ {
        let prompt = Prompt::new("greeting", Some("probe list_prompts"), None);
        ready(Ok(ListPromptsResult::with_all_items(vec![prompt])))
    }

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourcesResult, ErrorData>> + MaybeSendFuture + '_ {
        let resource = Resource::new("file:///greeting.txt", "greeting")
            .with_description("probe list_resources");
        ready(Ok(ListResourcesResult::with_all_items(vec![resource])))
    }

    fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourceTemplatesResult, ErrorData>> + MaybeSendFuture + '_
    {
        let template = ResourceTemplate::new("file:///{name}.txt", "greeting")
            .with_description("probe list_resource_templates");
        ready(Ok(ListResourceTemplatesResult::with_all_items(vec![
            template,
        ])))
    }

    fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ReadResourceResponse, ErrorData>> + MaybeSendFuture + '_ {
        let contents = ResourceContents::text("probe read_resource", request.uri);
        let result = ReadResourceResult::new(vec![contents]);
        ready(Ok(ReadResourceResponse::Complete(result)))
    }

    fn accepted_subscription_filter(
        &self,
        requested: &SubscriptionFilter,
    ) -> Option<SubscriptionFilter> {
        Some(requested.clone())
    }

    fn listen(
        &self,
        _context: SubscriptionContext,
    ) -> impl Future<Output = Result<(), ErrorData>> + MaybeSendFuture + '_ {
        ready(Err(probe_error("listen")))
    }

    fn subscribe(
        &self,
        _request: SubscribeRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<(), ErrorData>> + MaybeSendFuture + '_ {
        ready(Ok(()))
    }

    fn unsubscribe(
        &self,
        _request: UnsubscribeRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<(), ErrorData>> + MaybeSendFuture + '_ {
        ready(Ok(()))
    }

    fn call_tool(
        &self,
        _request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResponse, ErrorData>> + MaybeSendFuture + '_ {
        ready(Err(probe_error("call_tool")))
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> + MaybeSendFuture + '_ {
        let mut result = ListToolsResult::with_all_items(Vec::new());
        let mut meta = MetaObject::default();
        meta.0.insert("note".to_owned(), json!("probe list_tools"));
        result.meta = Some(meta);
        ready(Ok(result))
    }

    fn on_custom_request(
        &self,
        request: CustomRequest,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CustomResult, ErrorData>> + MaybeSendFuture + '_ {
        let answer = json!({"method": request.method, "note": "probe on_custom_request"});
        ready(Ok(CustomResult(answer)))
    }

    fn get_info(&self) -> InitializeResult {
        let mut info = InitializeResult::new(ServerCapabilities::builder().enable_tasks().build());
        info.protocol_version = ProtocolVersion::V_2025_11_25;
        info.server_info = Implementation::new("probe", "1.0.0");
        info.instructions = Some("probe".to_owned());
        info
    }

    fn get_task(
        &self,
        _request: GetTaskParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<GetTaskResult, ErrorData>> + MaybeSendFuture + '_ {
        ready(Err(probe_error("get_task")))
    }

    fn update_task(
        &self,
        _request: UpdateTaskParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<(), ErrorData>> + MaybeSendFuture + '_ {
        ready(Ok(()))
    }

    fn cancel_task(
        &self,
        _request: CancelTaskParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<(), ErrorData>> + MaybeSendFuture + '_ {
        ready(Ok(()))
    }
}

/// A [`Sink`] that keeps every log line.
#[derive(Debug, Default)]
pub(crate) struct RecordingSink {
    logs: Mutex<Vec<String>>,
}

impl RecordingSink {
    /// The log lines so far, oldest first.
    pub(crate) fn logs(&self) -> Vec<String> {
        self.logs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Sink for RecordingSink {
    fn log(&self, message: String) {
        self.logs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message);
    }
}

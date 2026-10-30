//! [`Tracked<H>`]: a `ServerHandler` that wraps the customer's handler and forwards every method
//! of rmcp's trait to it, in the order and with the signatures of rmcp 3.0's own wrapper impls
//! (`impl_server_handler_for_wrapper!`). `negotiate_initialize` (rmcp 3.3) is not forwarded:
//! only rmcp's default `initialize` calls it, and `initialize` is forwarded. The four methods the
//! SDK changes (`initialize`, `list_tools`, `get_tool`, `call_tool`) forward untouched when
//! `AGENTCAT_ENABLED` turned the handle off; `call_tool` then still adds a
//! [`CallContext`](crate::CallContext) without session or agent.

use std::any::type_name;
use std::borrow::Cow;
use std::fmt;
use std::sync::{Mutex, PoisonError};
use std::time::SystemTime;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CancelTaskParams, CancelledNotificationParam,
    CompleteRequestParams, CompleteResult, CustomNotification, CustomRequest, CustomResult,
    DiscoverResult, GetPromptRequestParams, GetPromptResponse, GetTaskParams, GetTaskResult,
    Implementation, InitializeRequestParams, InitializeResult, ListPromptsResult,
    ListResourceTemplatesResult, ListResourcesResult, ListToolsResult, PaginatedRequestParams,
    ProgressNotificationParam, ProtocolVersion, ReadResourceRequestParams, ReadResourceResponse,
    SubscribeRequestParams, SubscriptionFilter, Tool, UnsubscribeRequestParams, UpdateTaskParams,
};
use rmcp::service::{MaybeSendFuture, NotificationContext, RequestContext, SubscriptionContext};
use rmcp::{ErrorData, RoleServer, ServerHandler};

use crate::call::run::{run_call, server_info};
use crate::client::AgentCat;
use crate::constants::GET_MORE_TOOLS_NAME;
use crate::custom::{CustomEvent, publish_server};
use crate::error::Error;
use crate::event::ClientIdentity;
use crate::inject::{
    InjectConfig, ToolEntry, build_injected_list, get_more_tools_tool, inject_tool,
};

#[cfg(test)]
mod tests;

/// A server handler wrapped by [`AgentCat::track`] or [`track`](crate::track). It implements
/// rmcp's `ServerHandler`, so it is served like the handler it wraps. When `AGENTCAT_ENABLED`
/// turned the handle off, it forwards every method to the wrapped handler untouched, and a tool
/// still gets a [`CallContext`](crate::CallContext), with no session and no agent.
pub struct Tracked<H> {
    inner: H,
    agentcat: AgentCat,
    /// The client that `initialize` named on this instance. A clone starts with none.
    identity: Mutex<Option<Implementation>>,
}

impl<H: Clone> Clone for Tracked<H> {
    /// Clones the handler and the handle; the clone has seen no `initialize`, so it holds no
    /// client identity.
    fn clone(&self) -> Self {
        Self::new(self.inner.clone(), self.agentcat.clone())
    }
}

/// The warning logged for a listing with no tools.
const EMPTY_LISTING_WARNING: &str = "Warning: No tools found in the original list. This is likely due to the tools not being registered before AgentCat.track().";

const _: () = {
    const fn send_sync_static<T: Send + Sync + 'static>() {}
    const fn tracked<H: Send + Sync + 'static>() {
        send_sync_static::<Tracked<H>>();
    }
    tracked::<()>();
};

impl<H> Tracked<H> {
    /// Wraps `inner` for `agentcat`.
    pub(crate) fn new(inner: H, agentcat: AgentCat) -> Self {
        Self {
            inner,
            agentcat,
            identity: Mutex::new(None),
        }
    }

    /// The wrapped handler.
    pub fn inner(&self) -> &H {
        &self.inner
    }

    /// The wrapped handler, mutably.
    pub fn inner_mut(&mut self) -> &mut H {
        &mut self.inner
    }

    /// Unwraps the handler.
    pub fn into_inner(self) -> H {
        self.inner
    }

    /// The handle this wrapper reports to.
    pub fn agentcat(&self) -> &AgentCat {
        &self.agentcat
    }

    /// The handler's tool `name` as the listing shows it, with the entry the pipeline gives it
    /// (its `declares_session` is the foreign status of `session_id`). The pipeline runs on
    /// a copy of the registry's state and nothing is written back, so a lookup never uses up a
    /// log line that only a listing writes. The lock is held to copy the state only.
    pub(crate) fn lookup_tool(&self, name: &str) -> Option<(Tool, ToolEntry)>
    where
        H: ServerHandler,
    {
        let tool = self.inner.get_tool(name)?;
        let config = InjectConfig::from_options(&self.agentcat.inner().options);
        let mut state = self
            .agentcat
            .inner()
            .with_registry::<H, _>(|registry| registry.state_mut().clone());
        Some(inject_tool(&config, tool, &mut state, &mut Vec::new()))
    }

    /// The client that `initialize` named on this instance, if it has run.
    pub(crate) fn client_identity(&self) -> Option<Implementation> {
        self.identity
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Publishes a custom event with the identity of this instance. It only queues the event, so
    /// it returns at once. The client is the one this instance's `initialize` named (none before
    /// it, and none on a clone, which has seen no `initialize`); the server is what the handler's
    /// `get_info` gives now, and a panic there is logged and leaves the event without server
    /// fields.
    ///
    /// The event goes to the project of the handle, not to `project_id`, which only has to be
    /// non-empty. Nothing is queued while tracing is off, or by a handle that `AGENTCAT_ENABLED`
    /// turned off. The `redact_event` hook of the options runs on the event, on the runtime of the
    /// caller; the string hook does not. The event has no session unless the [`CustomEvent`] names
    /// one: it takes none from anywhere else. Inside a tool, pass
    /// [`CallContext::session_id`](crate::CallContext::session_id) in the [`CustomEvent`] to
    /// attribute the event to the call. A `duration` of zero or none is the time since the event
    /// was built.
    ///
    /// ```no_run
    /// # fn run<H: rmcp::ServerHandler>(
    /// #     tracked: agentcat_sdk::Tracked<H>,
    /// # ) -> Result<(), agentcat_sdk::Error> {
    /// use agentcat_sdk::CustomEvent;
    ///
    /// tracked.publish_custom_event("proj_123", CustomEvent::new().resource_name("startup"))?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// [`Error::ProjectIdRequired`] when `project_id` is empty, except on a handle that
    /// `AGENTCAT_ENABLED` turned off, which returns `Ok(())` and does nothing, not even call the
    /// handler's `get_info`.
    pub fn publish_custom_event(&self, project_id: &str, event: CustomEvent) -> Result<(), Error>
    where
        H: ServerHandler,
    {
        if !self.agentcat.inner().enabled {
            return Ok(());
        }
        if project_id.is_empty() {
            return Err(Error::ProjectIdRequired);
        }
        let created = SystemTime::now();
        let client = self.client_identity().as_ref().map(ClientIdentity::from);
        let server = server_info(self.agentcat.inner(), &self.inner);
        publish_server(
            &self.agentcat,
            project_id,
            event,
            client.as_ref(),
            server.as_ref(),
            created,
            SystemTime::now(),
        )
    }
}

impl<H> fmt::Debug for Tracked<H> {
    /// Names the wrapped handler's type, so `H` needs no `Debug` impl.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tracked")
            .field("inner", &type_name::<H>())
            .field("agentcat", &self.agentcat)
            .finish_non_exhaustive()
    }
}

impl<H: ServerHandler> ServerHandler for Tracked<H> {
    fn ping(
        &self,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<(), ErrorData>> + MaybeSendFuture + '_ {
        self.inner.ping(context)
    }

    #[expect(
        clippy::manual_async_fn,
        reason = "the signature of rmcp's trait method: a returned future with MaybeSendFuture"
    )]
    fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<InitializeResult, ErrorData>> + MaybeSendFuture + '_ {
        async move {
            if !self.agentcat.inner().enabled {
                return self.inner.initialize(request, context).await;
            }
            let client = request.client_info.clone();
            let result = self.inner.initialize(request, context).await?;
            *self.identity.lock().unwrap_or_else(PoisonError::into_inner) = Some(client);
            Ok(result)
        }
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        self.inner.supported_protocol_versions()
    }

    fn discover(
        &self,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<DiscoverResult, ErrorData>> + MaybeSendFuture + '_ {
        self.inner.discover(context)
    }

    fn complete(
        &self,
        request: CompleteRequestParams,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CompleteResult, ErrorData>> + MaybeSendFuture + '_ {
        self.inner.complete(request, context)
    }

    #[expect(
        deprecated,
        reason = "SetLevelRequestParams is SEP-2577-deprecated on every rmcp 3.x; the method is still forwarded"
    )]
    fn set_level(
        &self,
        request: rmcp::model::SetLevelRequestParams,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<(), ErrorData>> + MaybeSendFuture + '_ {
        self.inner.set_level(request, context)
    }

    fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<GetPromptResponse, ErrorData>> + MaybeSendFuture + '_ {
        self.inner.get_prompt(request, context)
    }

    fn list_prompts(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListPromptsResult, ErrorData>> + MaybeSendFuture + '_ {
        self.inner.list_prompts(request, context)
    }

    fn list_resources(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourcesResult, ErrorData>> + MaybeSendFuture + '_ {
        self.inner.list_resources(request, context)
    }

    fn list_resource_templates(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourceTemplatesResult, ErrorData>> + MaybeSendFuture + '_
    {
        self.inner.list_resource_templates(request, context)
    }

    fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ReadResourceResponse, ErrorData>> + MaybeSendFuture + '_ {
        self.inner.read_resource(request, context)
    }

    fn accepted_subscription_filter(
        &self,
        requested: &SubscriptionFilter,
    ) -> Option<SubscriptionFilter> {
        self.inner.accepted_subscription_filter(requested)
    }

    fn listen(
        &self,
        context: SubscriptionContext,
    ) -> impl Future<Output = Result<(), ErrorData>> + MaybeSendFuture + '_ {
        self.inner.listen(context)
    }

    #[expect(
        deprecated,
        reason = "resources/subscribe is legacy-only and deprecated on every rmcp 3.x; it is still forwarded"
    )]
    fn subscribe(
        &self,
        request: SubscribeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<(), ErrorData>> + MaybeSendFuture + '_ {
        self.inner.subscribe(request, context)
    }

    #[expect(
        deprecated,
        reason = "resources/unsubscribe is legacy-only and deprecated on every rmcp 3.x; it is still forwarded"
    )]
    fn unsubscribe(
        &self,
        request: UnsubscribeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<(), ErrorData>> + MaybeSendFuture + '_ {
        self.inner.unsubscribe(request, context)
    }

    /// The tool-call pipeline around the handler's `call_tool`: interception, stripping,
    /// mint-back and the record of the call; on a handle that is off, the handler's `call_tool`
    /// with a [`CallContext`](crate::CallContext) without session or agent.
    fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResponse, ErrorData>> + MaybeSendFuture + '_ {
        run_call(self, request, context)
    }

    #[expect(
        clippy::manual_async_fn,
        reason = "the signature of rmcp's trait method: a returned future with MaybeSendFuture"
    )]
    fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> + MaybeSendFuture + '_ {
        async move {
            let shared = self.agentcat.inner();
            if !shared.enabled {
                return self.inner.list_tools(request, context).await;
            }
            let mut result = self.inner.list_tools(request, context).await?;
            if result.tools.is_empty() {
                shared.sink.log(EMPTY_LISTING_WARNING.to_owned());
                return Ok(result);
            }
            let config = InjectConfig::from_options(&shared.options);
            let tools = std::mem::take(&mut result.tools);
            let list = shared.with_registry::<H, _>(|registry| {
                let list = build_injected_list(&config, tools, registry.state_mut());
                registry.record(&list);
                list
            });
            for line in list.logs {
                shared.sink.log(line);
            }
            result.tools = list.tools;
            Ok(result)
        }
    }

    /// The injected descriptor of `get_more_tools`, or the handler's tool through the pipeline,
    /// so the call path sees what the listing showed. The pipeline's log lines are dropped: the
    /// listing logs them once. On a handle that is off, the handler's tool.
    fn get_tool(&self, name: &str) -> Option<Tool> {
        let shared = self.agentcat.inner();
        if !shared.enabled {
            return self.inner.get_tool(name);
        }
        if shared.options.report_missing() && name == GET_MORE_TOOLS_NAME {
            return Some(get_more_tools_tool());
        }
        self.lookup_tool(name).map(|(tool, _entry)| tool)
    }

    fn on_custom_request(
        &self,
        request: CustomRequest,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CustomResult, ErrorData>> + MaybeSendFuture + '_ {
        self.inner.on_custom_request(request, context)
    }

    fn on_cancelled(
        &self,
        notification: CancelledNotificationParam,
        context: NotificationContext<RoleServer>,
    ) -> impl Future<Output = ()> + MaybeSendFuture + '_ {
        self.inner.on_cancelled(notification, context)
    }

    fn on_progress(
        &self,
        notification: ProgressNotificationParam,
        context: NotificationContext<RoleServer>,
    ) -> impl Future<Output = ()> + MaybeSendFuture + '_ {
        self.inner.on_progress(notification, context)
    }

    fn on_initialized(
        &self,
        context: NotificationContext<RoleServer>,
    ) -> impl Future<Output = ()> + MaybeSendFuture + '_ {
        self.inner.on_initialized(context)
    }

    fn on_roots_list_changed(
        &self,
        context: NotificationContext<RoleServer>,
    ) -> impl Future<Output = ()> + MaybeSendFuture + '_ {
        self.inner.on_roots_list_changed(context)
    }

    fn on_custom_notification(
        &self,
        notification: CustomNotification,
        context: NotificationContext<RoleServer>,
    ) -> impl Future<Output = ()> + MaybeSendFuture + '_ {
        self.inner.on_custom_notification(notification, context)
    }

    fn get_info(&self) -> InitializeResult {
        self.inner.get_info()
    }

    fn get_task(
        &self,
        request: GetTaskParams,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<GetTaskResult, ErrorData>> + MaybeSendFuture + '_ {
        self.inner.get_task(request, context)
    }

    fn update_task(
        &self,
        request: UpdateTaskParams,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<(), ErrorData>> + MaybeSendFuture + '_ {
        self.inner.update_task(request, context)
    }

    fn cancel_task(
        &self,
        request: CancelTaskParams,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<(), ErrorData>> + MaybeSendFuture + '_ {
        self.inner.cancel_task(request, context)
    }
}

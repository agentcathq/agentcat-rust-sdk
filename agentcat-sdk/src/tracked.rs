//! [`Tracked<H>`]: a `ServerHandler` that wraps the customer's handler and forwards every method
//! of rmcp's trait to it, in the order and with the signatures of rmcp 3.0's own wrapper impls
//! (`impl_server_handler_for_wrapper!`). `negotiate_initialize` (rmcp 3.3) is not forwarded
//! (C25): only rmcp's default `initialize` calls it, and `initialize` is forwarded.

use std::any::type_name;
use std::borrow::Cow;
use std::fmt;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CancelTaskParams, CancelledNotificationParam,
    CompleteRequestParams, CompleteResult, CustomNotification, CustomRequest, CustomResult,
    DiscoverResult, GetPromptRequestParams, GetPromptResponse, GetTaskParams, GetTaskResult,
    InitializeRequestParams, InitializeResult, ListPromptsResult, ListResourceTemplatesResult,
    ListResourcesResult, ListToolsResult, PaginatedRequestParams, ProgressNotificationParam,
    ProtocolVersion, ReadResourceRequestParams, ReadResourceResponse, SubscribeRequestParams,
    SubscriptionFilter, Tool, UnsubscribeRequestParams, UpdateTaskParams,
};
use rmcp::service::{MaybeSendFuture, NotificationContext, RequestContext, SubscriptionContext};
use rmcp::{ErrorData, RoleServer, ServerHandler};

use crate::client::AgentCat;

#[cfg(test)]
mod tests;

/// A server handler wrapped by [`AgentCat::track`] or [`track`](crate::track). It implements
/// rmcp's `ServerHandler`, so it is served like the handler it wraps.
#[derive(Clone)]
pub struct Tracked<H> {
    inner: H,
    agentcat: AgentCat,
}

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
        Self { inner, agentcat }
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
}

impl<H> fmt::Debug for Tracked<H> {
    /// Names the wrapped handler's type, so `H` needs no `Debug` impl.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tracked")
            .field("inner", &type_name::<H>())
            .field("agentcat", &self.agentcat)
            .finish()
    }
}

impl<H: ServerHandler> ServerHandler for Tracked<H> {
    fn ping(
        &self,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<(), ErrorData>> + MaybeSendFuture + '_ {
        self.inner.ping(context)
    }

    fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<InitializeResult, ErrorData>> + MaybeSendFuture + '_ {
        self.inner.initialize(request, context)
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

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResponse, ErrorData>> + MaybeSendFuture + '_ {
        self.inner.call_tool(request, context)
    }

    fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> + MaybeSendFuture + '_ {
        self.inner.list_tools(request, context)
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.inner.get_tool(name)
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

//! The call canary (`HARNESS_CANARY=call`, harness/README.md "L3 gate"): [`CallCanary`], an outer
//! handler around the tracked server that changes the SDK's mint-back text of each tool result
//! after the SDK recorded the call. The matrix must catch the change on the wire
//! (`l3-expect.mjs caught-call`) and find no event difference.
use std::borrow::Cow;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, CancelledNotificationParam,
    ContentBlock, DiscoverResult, InitializeRequestParams, InitializeResult, ListToolsResult,
    PaginatedRequestParams, ProgressNotificationParam, ProtocolVersion, Tool,
};
use rmcp::service::{MaybeSendFuture, NotificationContext, RequestContext};
use rmcp::{ErrorData, RoleServer, ServerHandler};

/// The start of the SDK's mint-back block (the first content block of a result that issued a
/// session ID).
const MINT_BACK: &str = "[session_id issued";
/// What the canary makes of [`MINT_BACK`].
const CHANGED: &str = "[session_id lssued";

/// Serves the handler it wraps, but changes the mint-back text of each tool result (see
/// [`change_mint_back`]). Every other request and notification of a tools-only server goes to
/// the wrapped handler as it is.
#[derive(Debug)]
pub(crate) struct CallCanary<H>(pub(crate) H);

impl<H: ServerHandler> ServerHandler for CallCanary<H> {
    fn ping(
        &self,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<(), ErrorData>> + MaybeSendFuture + '_ {
        self.0.ping(context)
    }

    fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<InitializeResult, ErrorData>> + MaybeSendFuture + '_ {
        self.0.initialize(request, context)
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        self.0.supported_protocol_versions()
    }

    fn discover(
        &self,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<DiscoverResult, ErrorData>> + MaybeSendFuture + '_ {
        self.0.discover(context)
    }

    /// The wrapped handler's answer, with the mint-back text of a completed result changed.
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let mut response = self.0.call_tool(request, context).await?;
        if let CallToolResponse::Complete(result) = &mut response {
            change_mint_back(result);
        }
        Ok(response)
    }

    fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> + MaybeSendFuture + '_ {
        self.0.list_tools(request, context)
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.0.get_tool(name)
    }

    fn on_cancelled(
        &self,
        notification: CancelledNotificationParam,
        context: NotificationContext<RoleServer>,
    ) -> impl Future<Output = ()> + MaybeSendFuture + '_ {
        self.0.on_cancelled(notification, context)
    }

    fn on_progress(
        &self,
        notification: ProgressNotificationParam,
        context: NotificationContext<RoleServer>,
    ) -> impl Future<Output = ()> + MaybeSendFuture + '_ {
        self.0.on_progress(notification, context)
    }

    fn on_initialized(
        &self,
        context: NotificationContext<RoleServer>,
    ) -> impl Future<Output = ()> + MaybeSendFuture + '_ {
        self.0.on_initialized(context)
    }

    fn get_info(&self) -> InitializeResult {
        self.0.get_info()
    }
}

/// Changes [`MINT_BACK`] to [`CHANGED`] when the first content block of `result` is text that
/// starts with it; any other result stays as it is.
fn change_mint_back(result: &mut CallToolResult) {
    if let Some(ContentBlock::Text(block)) = result.content.first_mut()
        && let Some(rest) = block.text.strip_prefix(MINT_BACK)
    {
        block.text = format!("{CHANGED}{rest}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(result: &CallToolResult) -> Vec<&str> {
        result
            .content
            .iter()
            .filter_map(|block| block.as_text().map(|text| text.text.as_str()))
            .collect()
    }

    #[test]
    fn only_a_leading_mint_back_block_changes() {
        let mint =
            "[session_id issued — see this tool's session_id parameter description]\nsession_id: s";
        let mut result = CallToolResult::success(vec![
            ContentBlock::text(mint),
            ContentBlock::text("[session_id issued again"),
        ]);
        change_mint_back(&mut result);
        assert_eq!(
            texts(&result),
            [
                "[session_id lssued — see this tool's session_id parameter description]\nsession_id: s",
                "[session_id issued again",
            ]
        );
        for kept in [
            vec![ContentBlock::text("hello"), ContentBlock::text(mint)],
            vec![ContentBlock::text(" [session_id issued")],
            vec![],
        ] {
            let mut result = CallToolResult::success(kept.clone());
            change_mint_back(&mut result);
            assert_eq!(result.content, kept);
        }
    }
}

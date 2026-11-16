//! The lists of a server info heartbeat, read from the handler's own list methods with the
//! context of the request that started the check: `tools/list` always, and `resources/list`,
//! `resources/templates/list` and `prompts/list` when the server declares the capability.
//!
//! Each list is read page by page through `next_cursor`, under `catch_unwind` and within
//! [`LIST_TIMEOUT`] in total. A list fails when its handler fails or panics, when a cursor comes
//! twice, when it has more than [`MAX_PAGES`] pages or when the time is over. The failure of the
//! tool list fails the whole read; another list that fails is left out.
use std::collections::HashSet;
use std::panic::AssertUnwindSafe;
use std::time::Duration;

use futures::FutureExt;
use rmcp::model::{
    Cursor, ListPromptsResult, ListResourceTemplatesResult, ListResourcesResult, ListToolsResult,
    PaginatedRequestParams, Prompt, Resource, ResourceTemplate, ServerCapabilities, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler};

use crate::capture::panic_message;

#[cfg(test)]
mod tests;

/// How long one list may take in total, all its pages together.
pub(crate) const LIST_TIMEOUT: Duration = Duration::from_secs(5);

/// The most pages one list may have.
pub(crate) const MAX_PAGES: usize = 100;

/// The lists that a read gave.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Lists {
    /// The pages of the tool list, as the handler gave them.
    pub(crate) tool_pages: Vec<Vec<Tool>>,
    /// The resources; `None` when the server declares no resources, or the list failed.
    pub(crate) resources: Option<Vec<Resource>>,
    /// The resource templates; `None` as for `resources`.
    pub(crate) resource_templates: Option<Vec<ResourceTemplate>>,
    /// The prompts; `None` when the server declares no prompts, or the list failed.
    pub(crate) prompts: Option<Vec<Prompt>>,
    /// The key of each list that failed and was left out, in the order of the reads.
    pub(crate) left_out: Vec<&'static str>,
}

/// One page of a list, as an rmcp list method gives it.
trait Page {
    /// What the list holds.
    type Item;

    /// The items of the page and the cursor of the next one.
    fn into_parts(self) -> (Vec<Self::Item>, Option<Cursor>);
}

impl Page for ListToolsResult {
    type Item = Tool;

    fn into_parts(self) -> (Vec<Tool>, Option<Cursor>) {
        (self.tools, self.next_cursor)
    }
}

impl Page for ListResourcesResult {
    type Item = Resource;

    fn into_parts(self) -> (Vec<Resource>, Option<Cursor>) {
        (self.resources, self.next_cursor)
    }
}

impl Page for ListResourceTemplatesResult {
    type Item = ResourceTemplate;

    fn into_parts(self) -> (Vec<ResourceTemplate>, Option<Cursor>) {
        (self.resource_templates, self.next_cursor)
    }
}

impl Page for ListPromptsResult {
    type Item = Prompt;

    fn into_parts(self) -> (Vec<Prompt>, Option<Cursor>) {
        (self.prompts, self.next_cursor)
    }
}

/// Reads the lists of `inner` with `context` (see the module docs). `Err` holds why the tool list
/// failed.
pub(crate) async fn read<H: ServerHandler>(
    inner: &H,
    context: &RequestContext<RoleServer>,
    capabilities: &ServerCapabilities,
) -> Result<Lists, String> {
    let tool_pages = pages(|params| inner.list_tools(Some(params), context.clone()))
        .await
        .map_err(|why| format!("the tools list {why}"))?;
    let mut lists = Lists {
        tool_pages,
        ..Lists::default()
    };
    if capabilities.resources.is_some() {
        let resources = pages(|params| inner.list_resources(Some(params), context.clone())).await;
        lists.resources = kept(resources, "resources", &mut lists.left_out);
        let templates =
            pages(|params| inner.list_resource_templates(Some(params), context.clone())).await;
        lists.resource_templates = kept(templates, "resourceTemplates", &mut lists.left_out);
    }
    if capabilities.prompts.is_some() {
        let prompts = pages(|params| inner.list_prompts(Some(params), context.clone())).await;
        lists.prompts = kept(prompts, "prompts", &mut lists.left_out);
    }
    Ok(lists)
}

/// The items of a list that worked, all its pages in order; a list that failed gives `None` and
/// its `key` joins `left_out`.
fn kept<T>(
    list: Result<Vec<Vec<T>>, String>,
    key: &'static str,
    left_out: &mut Vec<&'static str>,
) -> Option<Vec<T>> {
    match list {
        Ok(pages) => Some(pages.into_iter().flatten().collect()),
        Err(_why) => {
            left_out.push(key);
            None
        }
    }
}

/// Every page of the list that `fetch` reads, within [`LIST_TIMEOUT`]; a panic of the handler is
/// contained. `Err` holds what went wrong, worded to follow the name of the list.
async fn pages<P, F>(
    fetch: impl FnMut(PaginatedRequestParams) -> F,
) -> Result<Vec<Vec<P::Item>>, String>
where
    P: Page,
    F: Future<Output = Result<P, ErrorData>>,
{
    let all = AssertUnwindSafe(all_pages(fetch)).catch_unwind();
    match tokio::time::timeout(LIST_TIMEOUT, all).await {
        Ok(Ok(pages)) => pages,
        Ok(Err(payload)) => Err(format!("panicked: {}", panic_message(&*payload))),
        Err(_elapsed) => Err(format!(
            "did not respond within {}ms",
            LIST_TIMEOUT.as_millis()
        )),
    }
}

/// The pages that `fetch` gives, from the first one (no cursor) through `next_cursor`, until a
/// page has no cursor.
async fn all_pages<P, F>(
    mut fetch: impl FnMut(PaginatedRequestParams) -> F,
) -> Result<Vec<Vec<P::Item>>, String>
where
    P: Page,
    F: Future<Output = Result<P, ErrorData>>,
{
    let mut pages = Vec::new();
    let mut seen = HashSet::new();
    let mut params = PaginatedRequestParams::default();
    loop {
        if pages.len() >= MAX_PAGES {
            return Err(format!("has more than {MAX_PAGES} pages"));
        }
        let page = fetch(params)
            .await
            .map_err(|error| format!("gave an error: {}", error.message))?;
        let (items, next) = page.into_parts();
        pages.push(items);
        let Some(cursor) = next else {
            return Ok(pages);
        };
        if !seen.insert(cursor.clone()) {
            return Err("gave the same cursor twice".to_owned());
        }
        params = PaginatedRequestParams::default().with_cursor(Some(cursor));
    }
}

//! The body of a server info heartbeat: [`body`] turns what a read gave ([`Snapshot`]) into
//! `agentcat-api`'s `PublishServerInfoRequest`, and [`without_schemas`] makes the smaller body
//! that follows a `413`.
use std::time::Instant;

use agentcat_api::models::PublishServerInfoRequest;
use rmcp::model::{Implementation, Tool};
use serde_json::{Map, Value};
use tokio::runtime::Handle;

use super::lists::Lists;
use crate::constants::GET_MORE_TOOLS_NAME;
use crate::inject::{InjectConfig, InjectState, build_injected_list};

#[cfg(test)]
mod tests;

/// The keys that [`without_schemas`] removes from each tool.
const SCHEMA_KEYS: [&str; 2] = ["inputSchema", "outputSchema"];

/// What one read of a server gave: owned data, which the delivery worker turns into the body.
#[derive(Debug)]
pub(crate) struct Snapshot {
    /// The server's implementation info: the name and the version that its events carry, and the
    /// title.
    pub(crate) server: Implementation,
    /// The server's instructions.
    pub(crate) instructions: Option<String>,
    /// The lists.
    pub(crate) lists: Lists,
    /// What the tool-list pipeline injects on this handle.
    pub(crate) inject: InjectConfig,
    /// A copy of the pipeline's state for the handler type, as a listing would find it.
    pub(crate) state: InjectState,
    /// The runtime of the request that started the read, for the customer's redaction hook.
    pub(crate) runtime: Option<Handle>,
    /// When the read started.
    pub(crate) started: Instant,
}

/// The reason of a body whose lists cannot be written as JSON.
const UNWRITABLE: &str = "a list is not valid JSON";

/// The body for the project `project_id` from the parts of a [`Snapshot`]:
///
/// - `server_name` and `server_version` are those of `server`; `server_title` is its title and
///   `instructions` the server's instructions, each `""` without one.
/// - `features.tools` is what a client gets from `tools/list` ([`listed_tools`]).
/// - `features.resources`, `features.resourceTemplates` and `features.prompts` are the lists
///   that were read; a list that was not read has no key.
/// - Each item is the JSON object that rmcp writes for it.
///
/// `Err` holds a fixed reason when an item cannot be written as JSON. It never holds the text of
/// the error, which can quote the item.
pub(crate) fn body(
    project_id: &str,
    server: Implementation,
    instructions: Option<String>,
    lists: Lists,
    inject: &InjectConfig,
    state: &mut InjectState,
) -> Result<PublishServerInfoRequest, String> {
    let unwritable = |_error: serde_json::Error| UNWRITABLE.to_owned();
    let mut features = Map::new();
    let tools = listed_tools(lists.tool_pages, inject, state);
    features.insert(
        "tools".to_owned(),
        serde_json::to_value(tools).map_err(unwritable)?,
    );
    if let Some(resources) = lists.resources {
        features.insert(
            "resources".to_owned(),
            serde_json::to_value(resources).map_err(unwritable)?,
        );
    }
    if let Some(templates) = lists.resource_templates {
        features.insert(
            "resourceTemplates".to_owned(),
            serde_json::to_value(templates).map_err(unwritable)?,
        );
    }
    if let Some(prompts) = lists.prompts {
        features.insert(
            "prompts".to_owned(),
            serde_json::to_value(prompts).map_err(unwritable)?,
        );
    }
    Ok(PublishServerInfoRequest {
        project_id: project_id.to_owned(),
        server_name: Some(server.name),
        server_version: Some(server.version),
        server_title: Some(server.title.unwrap_or_default()),
        instructions: Some(instructions.unwrap_or_default()),
        features: Some(Value::Object(features)),
    })
}

/// The tools of `pages` as `tools/list` shows them: each page that has tools goes through the
/// tool-list pipeline, as its listing does, and an empty page adds nothing. When the pipeline
/// adds `get_more_tools`, it does so on each page; the tool is kept one time, where it first
/// comes.
fn listed_tools(
    pages: Vec<Vec<Tool>>,
    inject: &InjectConfig,
    state: &mut InjectState,
) -> Vec<Tool> {
    let mut tools = Vec::new();
    let mut reported_missing = false;
    for page in pages {
        if page.is_empty() {
            continue;
        }
        for tool in build_injected_list(inject, page, state).tools {
            if inject.report_missing && tool.name == GET_MORE_TOOLS_NAME {
                if reported_missing {
                    continue;
                }
                reported_missing = true;
            }
            tools.push(tool);
        }
    }
    tools
}

/// Removes `inputSchema` and `outputSchema` from each tool of `body`; every other key and its
/// place stay.
pub(crate) fn without_schemas(body: &mut PublishServerInfoRequest) {
    for tool in tools_mut(body) {
        if let Value::Object(tool) = tool {
            tool.retain(|key, _| !SCHEMA_KEYS.contains(&key.as_str()));
        }
    }
}

/// How many tools `body` holds.
pub(crate) fn tool_count(body: &PublishServerInfoRequest) -> usize {
    body.features
        .as_ref()
        .and_then(|features| features.get("tools"))
        .and_then(Value::as_array)
        .map_or(0, Vec::len)
}

/// The tools of `body`.
fn tools_mut(body: &mut PublishServerInfoRequest) -> impl Iterator<Item = &mut Value> {
    body.features
        .as_mut()
        .and_then(|features| features.get_mut("tools"))
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
}

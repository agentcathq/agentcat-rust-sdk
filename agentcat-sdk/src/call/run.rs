//! The tool-call pipeline of the TS SDK (`installCallWrap`, B-022) on [`Tracked<H>`]: the
//! report-missing intercept, the tracing-off and degraded paths, the schema of the called tool
//! (C03), the session handles (B-052 to B-069), the stripped call with its [`CallContext`], the
//! mint-back and its mirror, and the [`CallRecord`] handed to the [`Sink`](crate::sink::Sink).
//! `call/run/tests.rs` drives it over in-memory connections; `event/tests.rs` checks its records
//! over HTTP.
use std::collections::HashSet;
use std::time::{Duration, SystemTime};

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, PaginatedRequestParams, ServerResult,
    Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde_json::{Map, Value};
use tokio::time::Instant;

use super::{
    error_data_error, mirror_mint_back, prepend_mint_back, report_missing, result_error,
    strip_arguments,
};
use crate::client::Inner;
use crate::config::Options;
use crate::constants::{AGENTCAT_TAG_MRTR, GET_MORE_TOOLS_NAME};
use crate::context::CallContext;
use crate::event::{CallRecord, client_identity, extra_value, protocol_version, request_value};
use crate::handles::{
    build_handle_tags, build_mint_back_text, build_structured_mint_back, resolve_handles,
};
use crate::inject::{CONTEXT_PARAM, InjectConfig, ToolEntry, build_injected_list};
use crate::tokens::{estimate_input_tokens, estimate_output_tokens};
use crate::tracked::Tracked;

#[cfg(test)]
mod tests;

/// How long a registry rebuild may take in total, all listing pages together (B-027, C03).
pub(crate) const REBUILD_TIMEOUT: Duration = Duration::from_secs(5);

/// The resource name of a call whose tool name is empty (B-077).
const UNKNOWN_TOOL: &str = "Unknown Tool";

/// The log line of a registry rebuild that worked (B-027).
const REBUILT: &str =
    "Rebuilt injection registries on demand (tools/call before tools/list on this instance)";

/// The `agentcat_mrtr` tag of a call that carries the client's input responses (B-025).
const MRTR_CONTINUATION: &str = "continuation";

/// The `agentcat_mrtr` tag of a call that answers an `InputRequiredResult` (B-025).
const MRTR_INPUT_REQUIRED: &str = "input_required";

/// What the SDK learned about the called tool before the call (C03).
struct Schema {
    /// The tool's entry: from `inner.get_tool`, else from the registry.
    entry: Option<ToolEntry>,
    /// Whether the tool declares its own `session_id` (B-040, C19).
    foreign: bool,
    /// Whether a structured result gets the `mcp_session` mirror (B-058, B-069).
    mirror: bool,
}

/// TS's `tools/call` wrapper (B-022) around the inner handler's `call_tool`, in TS's order:
///
/// 1. `get_more_tools` is intercepted when report-missing is on.
/// 2. With tracing off, the intercept is answered, and any other call goes to the handler with
///    its raw arguments; nothing is recorded (B-023, K-05).
/// 3. The call starts (`started_at`).
/// 4. The tool's schema ([`learn_schema`]).
/// 5. The session handles. When minting fails, the call takes the degraded path: it is answered
///    as in step 2, after a log line.
/// 6. The record of the call (B-077, B-083, B-096).
/// 7. The intercept is answered with the `context` argument as the intent; any other call goes
///    to the handler with the injected arguments stripped (B-050, B-051) and a [`CallContext`]
///    in its request context.
/// 8. A result gets the mint-back text and its mirror and is recorded (B-056 to B-059, B-097);
///    an `InputRequiredResult` is tagged, not decorated (B-025).
/// 9. A JSON-RPC error is recorded and returned unchanged (RD-05, D16).
pub(crate) async fn run_call<H: ServerHandler>(
    tracked: &Tracked<H>,
    request: CallToolRequestParams,
    mut context: RequestContext<RoleServer>,
) -> Result<CallToolResponse, ErrorData> {
    let shared = tracked.agentcat().inner();
    let options = &shared.options;
    let intercept = options.report_missing() && request.name == GET_MORE_TOOLS_NAME;
    if !options.tracing() {
        return untraced(tracked, request, context, intercept).await;
    }

    let started_at = SystemTime::now();
    let started = Instant::now();

    let schema = learn_schema(tracked, &request.name, &context).await;

    let Some(resolution) = resolve_handles(
        request.arguments.as_ref(),
        options.hook_mode(),
        options.agent_tracking(),
        !schema.foreign,
    ) else {
        shared.sink.log(format!(
            "Warning: AgentCat tracing failed for tool {}, falling back to original handler - Error: could not generate a session ID",
            request.name
        ));
        return untraced(tracked, request, context, intercept).await;
    };

    let request_json = request_value(&request, &context);
    let input_tokens = estimate_input_tokens(request_json.pointer("/params/arguments"));
    let mut tags = Map::new();
    if request.input_responses.is_some() {
        tags.insert(AGENTCAT_TAG_MRTR.to_owned(), MRTR_CONTINUATION.into());
    }
    tags.extend(build_handle_tags(
        &resolution,
        protocol_version(&context.meta).as_deref(),
    ));
    let server = tracked.inner().get_info().server_info;
    let mut record = CallRecord {
        started_at,
        duration_ms: 0,
        resolution,
        resource_name: resource_name(&request.name),
        request: request_json,
        extra: extra_value(&context),
        input_tokens,
        output_tokens: None,
        tags,
        user_intent: user_intent(options, &request),
        response: None,
        is_error: false,
        error: None,
        server_name: Some(server.name),
        server_version: Some(server.version),
        client: client_identity(&context),
    };

    let name = request.name.to_string();
    let outcome = if intercept {
        record.user_intent = context_argument(&request).cloned();
        Ok(answer_report_missing(shared, &request).into())
    } else {
        let mut forwarded = request;
        forwarded.arguments = forwarded
            .arguments
            .map(|arguments| strip_arguments(&arguments, schema.entry.as_ref(), &name));
        context.extensions.insert(CallContext::new(
            record.resolution.session_id.clone(),
            record.resolution.agent_id.clone(),
            tracked.agentcat().clone(),
        ));
        tracked.inner().call_tool(forwarded, context).await
    };

    match outcome {
        Ok(response) => Ok(finish(
            shared,
            record,
            &name,
            schema.mirror,
            started,
            response,
        )),
        Err(error) => {
            record.is_error = true;
            record.error = Some(error_data_error(&error));
            record.duration_ms = elapsed_ms(started);
            shared.sink.record(record);
            Err(error)
        }
    }
}

/// A call that is not traced (tracing off, or the degraded path): the intercept is answered, and
/// any other call goes to the handler with its raw arguments (B-022 steps 2 and 9).
async fn untraced<H: ServerHandler>(
    tracked: &Tracked<H>,
    request: CallToolRequestParams,
    context: RequestContext<RoleServer>,
    intercept: bool,
) -> Result<CallToolResponse, ErrorData> {
    if intercept {
        let shared = tracked.agentcat().inner();
        return Ok(answer_report_missing(shared, &request).into());
    }
    tracked.inner().call_tool(request, context).await
}

/// The answer to a `get_more_tools` call, after its log line goes to the sink (B-046). TS logs it
/// inside the tool, so every answered call logs it, traced or not.
fn answer_report_missing(shared: &Inner, request: &CallToolRequestParams) -> CallToolResult {
    let (result, line) = report_missing(context_argument(request));
    shared.sink.log(line);
    result
}

/// The `context` argument of `request`, as sent.
fn context_argument(request: &CallToolRequestParams) -> Option<&Value> {
    request.arguments.as_ref()?.get(CONTEXT_PARAM)
}

/// The tool name as the record names it: `Unknown Tool` for an empty one (B-077).
fn resource_name(name: &str) -> String {
    if name.is_empty() {
        UNKNOWN_TOOL.to_owned()
    } else {
        name.to_owned()
    }
}

/// The intent of a call that is not intercepted (B-083, K-08): with the tool-call context on and
/// a tool other than `get_more_tools`, the `context` argument as sent, whoever owns the parameter
/// and whatever its type.
fn user_intent(options: &Options, request: &CallToolRequestParams) -> Option<Value> {
    if !options.tool_call_context() || request.name == GET_MORE_TOOLS_NAME {
        return None;
    }
    context_argument(request).cloned()
}

/// The schema of the tool `name` (C03): `inner.get_tool(name)` through the tool-list pipeline
/// ([`Tracked::lookup_tool`]), else the registry's entry, else, while the registry has never
/// recorded a listing, the entry after a [`rebuild`]. Without an entry the call gets TS's
/// heuristic strip (B-051).
///
/// The tool is foreign when the registry's set of tools that declared their own `session_id`
/// holds it, or when its `get_tool` schema declares one (C19). The mirror gate is the entry's
/// `output_injected`; without an entry it is open only while no listing was ever recorded (B-069).
async fn learn_schema<H: ServerHandler>(
    tracked: &Tracked<H>,
    name: &str,
    context: &RequestContext<RoleServer>,
) -> Schema {
    let shared = tracked.agentcat().inner();
    let looked_up = tracked.lookup_tool(name).map(|(_tool, entry)| entry);
    let mut known = registry_view::<H>(shared, name);
    if looked_up.is_none() && known.entry.is_none() && !known.listed {
        rebuild(tracked, context).await;
        known = registry_view::<H>(shared, name);
    }
    let foreign = known.declares_session
        || looked_up
            .as_ref()
            .is_some_and(|entry| entry.declares_session);
    let entry = looked_up.or(known.entry);
    let mirror = entry
        .as_ref()
        .map_or(!known.listed, |entry| entry.output_injected);
    Schema {
        entry,
        foreign,
        mirror,
    }
}

/// What the registry of the handler type `H` knows about one tool.
struct RegistryView {
    entry: Option<ToolEntry>,
    listed: bool,
    declares_session: bool,
}

/// The registry's entry for `name`, whether it recorded a listing, and whether `name` ever
/// declared its own `session_id`, read under one lock.
fn registry_view<H: 'static>(shared: &Inner, name: &str) -> RegistryView {
    shared.with_registry::<H, _>(|registry| RegistryView {
        entry: registry.entry(name).cloned(),
        listed: registry.listed(),
        declares_session: registry.declares_session(name),
    })
}

/// TS's registry rebuild on demand (B-027), across all pages (C03, RD-12): the inner handler's
/// listing, page by page through `next_cursor`, within [`REBUILD_TIMEOUT`] in total. Each page
/// then goes through the tool-list pipeline and into the registry, as a listing would, and the
/// pipeline's log lines go to the sink. A failure or a timeout logs a warning and records
/// nothing, so the next call tries again.
async fn rebuild<H: ServerHandler>(tracked: &Tracked<H>, context: &RequestContext<RoleServer>) {
    let shared = tracked.agentcat().inner();
    let listing = tokio::time::timeout(REBUILD_TIMEOUT, list_pages(tracked.inner(), context));
    let pages = match listing.await {
        Ok(Ok(pages)) => pages,
        Ok(Err(error)) => return rebuild_failed(shared, &error.message),
        Err(_elapsed) => {
            let millis = REBUILD_TIMEOUT.as_millis();
            let message = format!("tools/list did not respond within {millis}ms");
            return rebuild_failed(shared, &message);
        }
    };
    let config = InjectConfig::from_options(&shared.options);
    let logs = shared.with_registry::<H, _>(|registry| {
        let mut logs = Vec::new();
        for tools in pages {
            let list = build_injected_list(&config, tools, registry.state_mut());
            registry.record(&list);
            logs.extend(list.logs);
        }
        logs
    });
    for line in logs {
        shared.sink.log(line);
    }
    shared.sink.log(REBUILT.to_owned());
}

/// The log line of a rebuild that failed with `message` (B-027).
fn rebuild_failed(shared: &Inner, message: &str) {
    shared.sink.log(format!(
        "Warning: registry rebuild-on-demand failed - Error: {message}"
    ));
}

/// The tools of every listing page of `inner`, from the first page (TS's `params: {}`) through
/// `next_cursor`. A cursor seen before ends the listing, and the task yields before each further
/// page, so a handler that pages forever cannot hold the runtime past the rebuild's timeout.
async fn list_pages<H: ServerHandler>(
    inner: &H,
    context: &RequestContext<RoleServer>,
) -> Result<Vec<Vec<Tool>>, ErrorData> {
    let mut pages = Vec::new();
    let mut seen = HashSet::new();
    let mut params = PaginatedRequestParams::default();
    loop {
        let page = inner.list_tools(Some(params), context.clone()).await?;
        pages.push(page.tools);
        match page.next_cursor {
            Some(cursor) if seen.insert(cursor.clone()) => {
                params = PaginatedRequestParams::default().with_cursor(Some(cursor));
                tokio::task::yield_now().await;
            }
            _ => return Ok(pages),
        }
    }
}

/// B-022 step 12 on the handler's `response`: the record gets the response as it came (B-059)
/// and its output tokens; a complete result gets the mint-back text, the mirror when `mirror` is
/// open, and, when it is an error result, the record's error (B-097). An `InputRequiredResult`
/// is tagged and not decorated (B-025); any other response is not decorated. When the response
/// cannot be written as JSON, it is returned untouched and not recorded, after a log line.
fn finish(
    shared: &Inner,
    mut record: CallRecord,
    name: &str,
    mirror: bool,
    started: Instant,
    response: CallToolResponse,
) -> CallToolResponse {
    let original = match response_json(&response) {
        Ok(original) => original,
        Err(error) => {
            shared.sink.log(format!(
                "Warning: AgentCat post-handler processing failed for tool {name}; returning the customer's result untouched - {error}"
            ));
            return response;
        }
    };
    let response = match response {
        CallToolResponse::Complete(mut result) => {
            if result.is_error == Some(true) {
                record.is_error = true;
                record.error = Some(result_error(&result));
            }
            if let Some(text) = build_mint_back_text(&record.resolution) {
                prepend_mint_back(&mut result, text);
            }
            if mirror && let Some(mint) = build_structured_mint_back(&record.resolution) {
                mirror_mint_back(&mut result.structured_content, mint);
            }
            CallToolResponse::Complete(result)
        }
        CallToolResponse::InputRequired(result) => {
            record
                .tags
                .insert(AGENTCAT_TAG_MRTR.to_owned(), MRTR_INPUT_REQUIRED.into());
            CallToolResponse::InputRequired(result)
        }
        other => other,
    };
    record.output_tokens = estimate_output_tokens(Some(&original));
    record.response = Some(original);
    record.duration_ms = elapsed_ms(started);
    shared.sink.record(record);
    response
}

/// `response` as JSON, as the client would receive it before any decoration.
fn response_json(response: &CallToolResponse) -> serde_json::Result<Value> {
    match response {
        CallToolResponse::Complete(result) => serde_json::to_value(result),
        CallToolResponse::InputRequired(result) => serde_json::to_value(result),
        other => serde_json::to_value(ServerResult::from(other.clone())),
    }
}

/// The milliseconds since `started`, at most `u64::MAX`.
fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

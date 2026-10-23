//! The tool-call pipeline of the TS SDK (`installCallWrap`, B-022) on [`Tracked<H>`]: the
//! report-missing intercept, the tracing-off and degraded paths, the schema of the called tool
//! (C03), the session handles (B-052 to B-069), the stripped call with its [`CallContext`], the
//! mint-back and its mirror, the panics and captured errors of the call (C23), and the
//! [`CallRecord`] handed to the [`Sink`](crate::sink::Sink). `call/run/tests.rs` drives it over
//! in-memory connections, `capture/tests.rs` its panics and captured errors; `event/tests.rs`
//! checks its records over HTTP.
use std::collections::HashSet;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::time::{Duration, SystemTime};

use futures::FutureExt;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, Implementation,
    PaginatedRequestParams, ServerResult, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde_json::{Map, Value};
use tokio::runtime::Handle;
use tokio::time::Instant;

use super::{
    error_data_error, mirror_mint_back, prepend_mint_back, report_missing, result_error,
    strip_arguments,
};
use crate::capture::{CaptureSlot, Captured, panic_error, panic_message, scope};
use crate::client::Inner;
use crate::config::Options;
use crate::constants::{AGENTCAT_TAG_MRTR, GET_MORE_TOOLS_NAME};
use crate::context::CallContext;
use crate::event::{
    CallRecord, ClientIdentity, client_identity, extra_value, protocol_version, request_value,
};
use crate::handles::{
    HandleResolution, build_handle_tags, build_mint_back_text, build_structured_mint_back,
    resolve_handles,
};
use crate::hooks::{PendingHooks, ToolCall, fire};
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
///    its raw arguments and a [`CallContext`] without a session; nothing is recorded (B-023,
///    K-05).
/// 3. The call starts (`started_at`).
/// 4. The tool's schema ([`learn_schema`]).
/// 5. The session handles. When minting fails, the call takes the degraded path: it is answered
///    as in step 2, after a log line.
/// 6. The record of the call (B-077, B-083, B-096).
/// 7. The intercept is answered with the `context` argument as the intent; any other call goes
///    to the handler with the injected arguments stripped (B-050, B-051) and a [`CallContext`]
///    in its request context. The handler's `call_tool` runs inside the call's
///    [`scope`], so [`capture_error`](crate::capture_error) reaches this record, and under
///    `catch_unwind`: a panic is recorded with `type: "panic"` (C23) and then resumed with its
///    own payload, so the client gets no response, as without the SDK (design §6.4).
/// 8. A result gets the mint-back text and its mirror and is recorded (B-056 to B-059, B-097);
///    an `InputRequiredResult` is tagged, not decorated (B-025).
/// 9. A JSON-RPC error is recorded and returned unchanged (RD-05, D16).
///
/// For a failed call, an error captured during the call replaces the error that the result
/// (B-097) or the JSON-RPC code name (R09) would give (C23). The other customer calls this path
/// makes (`get_tool`, the rebuild's `list_tools`, `get_info`) are contained when they panic: each
/// panic is logged with its message (C30), and the call goes on without what they would have
/// given.
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
    let runtime = Handle::try_current().ok();

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

    let client = client_identity(&context);
    let pending = start_hooks(shared, &request, &context, client.as_ref()).await;

    let request_json = request_value(&request, &context);
    let input_tokens = estimate_input_tokens(request_json.pointer("/params/arguments"));
    let tags = call_tags(&request, &resolution, &context);
    let server = server_info(shared, tracked.inner());
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
        server_name: server.as_ref().map(|server| server.name.clone()),
        server_version: server.as_ref().map(|server| server.version.clone()),
        client: client.clone(),
        pending,
        runtime,
        backtrace: None,
    };

    let name = request.name.to_string();
    let slot = CaptureSlot::default();
    let outcome = if intercept {
        record.user_intent = context_argument(&request).cloned();
        Ok(answer_report_missing(shared, &request).into())
    } else {
        let mut forwarded = request;
        forwarded.arguments = forwarded
            .arguments
            .map(|arguments| strip_arguments(&arguments, schema.entry.as_ref(), &name));
        context.extensions.insert(
            CallContext::new(
                record.resolution.session_id.clone(),
                record.resolution.agent_id.clone(),
                tracked.agentcat().clone(),
            )
            .with_identity(client, server),
        );
        // The handler's `call_tool` itself runs inside the future, so the scope and the panic
        // boundary also cover what a hand-written handler does before it returns its future.
        let call = async move { tracked.inner().call_tool(forwarded, context).await };
        match scope(slot.clone(), AssertUnwindSafe(call).catch_unwind()).await {
            Ok(outcome) => outcome,
            Err(payload) => {
                record.is_error = true;
                record.error = Some(panic_error(&*payload));
                record.duration_ms = elapsed_ms(started);
                shared.sink.record(record);
                resume_unwind(payload);
            }
        }
    };

    let captured = slot.take();
    match outcome {
        Ok(response) => Ok(finish(
            shared,
            record,
            &name,
            schema.mirror,
            started,
            response,
            captured,
        )),
        Err(error) => {
            set_error(&mut record, captured, || error_data_error(&error));
            record.duration_ms = elapsed_ms(started);
            shared.sink.record(record);
            Err(error)
        }
    }
}

/// The SDK's tags for a call (B-025, B-096): the MRTR continuation, then the handle tags.
fn call_tags(
    request: &CallToolRequestParams,
    resolution: &HandleResolution,
    context: &RequestContext<RoleServer>,
) -> Map<String, Value> {
    let mut tags = Map::new();
    if request.input_responses.is_some() {
        tags.insert(AGENTCAT_TAG_MRTR.to_owned(), MRTR_CONTINUATION.into());
    }
    tags.extend(build_handle_tags(
        resolution,
        protocol_version(&context.meta).as_deref(),
    ));
    tags
}

/// B-022 step 7: starts the customer's hooks, before the tool runs, and never awaits them (B-026).
/// The task yields once when a hook started, so that each hook has made its first poll before the
/// tool runs, as TS calls them first. The hooks see the request as sent, before the
/// [`CallContext`] joins the context.
async fn start_hooks(
    shared: &Inner,
    request: &CallToolRequestParams,
    context: &RequestContext<RoleServer>,
    client: Option<&ClientIdentity>,
) -> PendingHooks {
    let options = &shared.options;
    let call = ToolCall::new(request, context, client);
    let pending = fire(options, &call, options.hook_mode(), shared.sink.as_ref());
    if pending.any() {
        tokio::task::yield_now().await;
    }
    pending
}

/// A call that is not traced (tracing off, or the degraded path): the intercept is answered, and
/// any other call goes to the handler with its raw arguments (B-022 steps 2 and 9) and a
/// [`CallContext`] without a session or an agent, so a tool that reads one keeps working.
async fn untraced<H: ServerHandler>(
    tracked: &Tracked<H>,
    request: CallToolRequestParams,
    mut context: RequestContext<RoleServer>,
    intercept: bool,
) -> Result<CallToolResponse, ErrorData> {
    if intercept {
        let shared = tracked.agentcat().inner();
        return Ok(answer_report_missing(shared, &request).into());
    }
    let call = CallContext::new(String::new(), None, tracked.agentcat().clone());
    context.extensions.insert(call);
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

/// The `server_info` of `inner` (B-086), or none when its `get_info` panics: the panic is
/// contained, logged with its message (C30), and the record gets no server name or version.
pub(crate) fn server_info<H: ServerHandler>(shared: &Inner, inner: &H) -> Option<Implementation> {
    match catch_unwind(AssertUnwindSafe(|| inner.get_info().server_info)) {
        Ok(server) => Some(server),
        Err(payload) => {
            let message = panic_message(&*payload);
            shared
                .sink
                .log(format!("Warning: get_info panicked: {message}"));
            None
        }
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
/// heuristic strip (B-051). A tool that got no parameters (a composed input schema, for example)
/// has an empty entry, as TS gives every listed tool one (`listWrap.ts`), so its arguments stay as
/// sent.
///
/// A `get_tool` that panics gives no schema: the panic is contained and logged with its message
/// (C30), and the registry, the rebuild and the heuristic follow as for a tool `get_tool` does not
/// know.
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
    let looked_up = match catch_unwind(AssertUnwindSafe(|| tracked.lookup_tool(name))) {
        Ok(found) => found.map(|(_tool, entry)| entry),
        Err(payload) => {
            let message = panic_message(&*payload);
            shared.sink.log(format!(
                "Warning: Tool lookup for {name} panicked: {message}"
            ));
            None
        }
    };
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
/// pipeline's log lines go to the sink. A failure, a panic of the handler's `list_tools` (with
/// the panic's message) or a timeout logs a warning and records nothing, so the next call tries
/// again.
async fn rebuild<H: ServerHandler>(tracked: &Tracked<H>, context: &RequestContext<RoleServer>) {
    let shared = tracked.agentcat().inner();
    let pages = AssertUnwindSafe(list_pages(tracked.inner(), context)).catch_unwind();
    let pages = match tokio::time::timeout(REBUILD_TIMEOUT, pages).await {
        Ok(Ok(Ok(pages))) => pages,
        Ok(Ok(Err(error))) => return rebuild_failed(shared, &error.message),
        Ok(Err(payload)) => return rebuild_failed(shared, &panic_message(&*payload)),
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
/// open, and, when it is an error result, the record's error: the `captured` one, else the
/// result's (B-097, C23). An `InputRequiredResult` is tagged and not decorated (B-025); any other
/// response is not decorated. When the response cannot be written as JSON, it is returned
/// untouched and not recorded, after a log line.
fn finish(
    shared: &Inner,
    mut record: CallRecord,
    name: &str,
    mirror: bool,
    started: Instant,
    response: CallToolResponse,
    captured: Option<Captured>,
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
                set_error(&mut record, captured, || result_error(&result));
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

/// Marks `record` as failed with the `captured` error and its backtrace, else the error of
/// `fallback` (C23).
fn set_error(
    record: &mut CallRecord,
    captured: Option<Captured>,
    fallback: impl FnOnce() -> Map<String, Value>,
) {
    record.is_error = true;
    let error = match captured {
        Some((error, backtrace)) => {
            record.backtrace = backtrace;
            error
        }
        None => fallback(),
    };
    record.error = Some(error);
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

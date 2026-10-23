//! The tool-call pipeline of the TS SDK (`installCallWrap`, B-022) on [`Tracked<H>`]: the
//! pass-through of a handle that is off (C45), the report-missing intercept, the tracing-off and
//! degraded paths, the schema of the called tool (C03), the session handles (B-052 to B-069), the
//! stripped call with its [`CallContext`], the mint-back and its mirror, the panics and captured
//! errors of the call (C23), and the [`CallRecord`] handed to the [`Sink`], also for a call whose
//! future is dropped (C41). `call/run/tests.rs` drives it over in-memory connections,
//! `capture/tests.rs` its panics and captured errors; `event/tests.rs` checks its records over
//! HTTP.
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
    cancelled_error, error_data_error, mirror_mint_back, prepend_mint_back, report_missing,
    result_error, strip_arguments,
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
use crate::sink::Sink;
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

/// Whether learning a tool's schema ([`learn_schema`]) writes its log lines: the lookup panic, the
/// rebuild's pipeline lines, the rebuild line and a failed rebuild's line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SchemaLogs {
    /// A traced call writes them to the sink.
    Written,
    /// A call with tracing off writes none of them, as TS's tracing-off branch returns before
    /// `ensureRegistries` and logs nothing (`callWrap.ts:159-165`, P16). So a tracing-off handle
    /// starts no delivery thread and writes no log file on this path.
    Silent,
}

impl SchemaLogs {
    /// Hands the line that `line` builds to the sink of `shared` when the lines are written.
    fn log(self, shared: &Inner, line: impl FnOnce() -> String) {
        if self == Self::Written {
            shared.sink.log(line());
        }
    }
}

/// TS's `tools/call` wrapper (B-022) around the inner handler's `call_tool`, in TS's order:
///
/// 1. A handle that `AGENTCAT_ENABLED` turned off hands the call to the handler untouched, with a
///    [`CallContext`] without a session or an agent (C45, C29).
/// 2. `get_more_tools` is intercepted when report-missing is on.
/// 3. With tracing off, the intercept is answered, and any other call learns the tool's schema as
///    in step 5, without its log lines (P16), and goes to the handler with the injected arguments
///    stripped as in step 8 (R27, RD-29) and a [`CallContext`] without a session; nothing is
///    recorded (B-023).
/// 4. The call starts (`started_at`).
/// 5. The tool's schema ([`learn_schema`]).
/// 6. The session handles. When minting fails, the call takes the degraded path: after a log
///    line, the intercept is answered, and any other call goes to the handler with its raw
///    arguments (TS, K-05) and a [`CallContext`] without a session; nothing is recorded.
/// 7. The record of the call (B-077, B-083, B-096), in a [`RecordGuard`] until it is sent: when
///    the call future is dropped before that (C41, RD-34), the record goes to the sink as
///    cancelled.
/// 8. The intercept is answered with the `context` argument as the intent; any other call goes
///    to the handler with the injected arguments stripped (B-050, B-051) and a [`CallContext`]
///    in its request context. The handler's `call_tool` runs inside the call's
///    [`scope`], so [`capture_error`](crate::capture_error) reaches this record, and under
///    `catch_unwind`: a panic is recorded with `type: "panic"` (C23) and then resumed with its
///    own payload, so the client gets no response, as without the SDK (design §6.4).
/// 9. A result gets the mint-back text and its mirror and is recorded (B-056 to B-059, B-097);
///    an `InputRequiredResult` is tagged, not decorated (B-025).
/// 10. A JSON-RPC error is recorded and returned unchanged (RD-05, D16).
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
    if !shared.enabled {
        return forward(tracked, request, context).await;
    }
    let options = &shared.options;
    let intercept = options.report_missing() && request.name == GET_MORE_TOOLS_NAME;
    if !options.tracing() {
        return untraced(tracked, request, context, intercept).await;
    }

    let started_at = SystemTime::now();
    let started = Instant::now();
    let runtime = Handle::try_current().ok();

    let schema = learn_schema(tracked, &request.name, &context, SchemaLogs::Written).await;

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
        return degraded(tracked, request, context, intercept).await;
    };

    let client = client_identity(&context);
    let pending = start_hooks(shared, &request, &context, client.as_ref()).await;

    let request_json = request_value(&request, &context);
    let input_tokens = estimate_input_tokens(request_json.pointer("/params/arguments"));
    let tags = call_tags(&request, &resolution, &context);
    let server = server_info(shared, tracked.inner());
    let mint_back = MintBack::new(&resolution, schema.mirror);
    let call_context = CallContext::new(
        resolution.session_id.clone(),
        resolution.agent_id.clone(),
        tracked.agentcat().clone(),
    );
    let intent = if intercept {
        context_argument(&request).cloned()
    } else {
        user_intent(options, &request)
    };
    let record = CallRecord {
        started_at,
        duration_ms: 0,
        resolution,
        resource_name: resource_name(&request.name),
        request: request_json,
        extra: extra_value(&context),
        input_tokens,
        output_tokens: None,
        tags,
        user_intent: intent,
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
    let guard = RecordGuard::new(record, shared.sink.as_ref(), started);

    let name = request.name.to_string();
    if intercept {
        let answer = answer_report_missing(shared, &request);
        return Ok(finish(guard, &name, mint_back, answer.into(), None));
    }
    let forwarded = stripped(request, schema.entry.as_ref());
    context
        .extensions
        .insert(call_context.with_identity(client, server));
    // The handler's `call_tool` itself runs inside the future, so the scope and the panic
    // boundary also cover what a hand-written handler does before it returns its future.
    let slot = CaptureSlot::default();
    let call = async move { tracked.inner().call_tool(forwarded, context).await };
    let outcome = match scope(slot.clone(), AssertUnwindSafe(call).catch_unwind()).await {
        Ok(outcome) => outcome,
        Err(payload) => {
            guard.send(|record| {
                record.is_error = true;
                record.error = Some(panic_error(&*payload));
            });
            resume_unwind(payload);
        }
    };

    let captured = slot.take();
    match outcome {
        Ok(response) => Ok(finish(guard, &name, mint_back, response, captured)),
        Err(error) => {
            guard.send(|record| set_error(record, captured, || error_data_error(&error)));
            Err(error)
        }
    }
}

/// The record of a traced call from its creation until it goes to the sink (C41, RD-34). The
/// call's own outcomes send it with [`send`](Self::send) or drop it with
/// [`discard`](Self::discard). When the call future is dropped before that (a customer timeout, a
/// `select!`, a server that stops), the guard's drop sends the record as cancelled: no response,
/// `is_error`, the [`cancelled_error`], and the duration until the drop. So each record goes to
/// the sink at most once.
struct RecordGuard<'a> {
    /// The record; `None` once it was sent or discarded.
    record: Option<CallRecord>,
    sink: &'a dyn Sink,
    /// When the call started, for the duration.
    started: Instant,
}

impl<'a> RecordGuard<'a> {
    /// Holds `record` until it goes to `sink`.
    fn new(record: CallRecord, sink: &'a dyn Sink, started: Instant) -> Self {
        Self {
            record: Some(record),
            sink,
            started,
        }
    }

    /// Completes the record with `complete` and the duration of the call, and sends it.
    fn send(mut self, complete: impl FnOnce(&mut CallRecord)) {
        if let Some(mut record) = self.record.take() {
            complete(&mut record);
            record.duration_ms = elapsed_ms(self.started);
            self.sink.record(record);
        }
    }

    /// Drops the record without sending it, after the log line `line`.
    fn discard(mut self, line: String) {
        self.record = None;
        self.sink.log(line);
    }
}

impl Drop for RecordGuard<'_> {
    fn drop(&mut self) {
        if let Some(mut record) = self.record.take() {
            record.response = None;
            record.output_tokens = None;
            record.is_error = true;
            record.error = Some(cancelled_error());
            record.duration_ms = elapsed_ms(self.started);
            self.sink.record(record);
        }
    }
}

/// What a complete result gets before it reaches the client (B-056, B-058): the mint-back text,
/// and the structured mirror when the gate of [`learn_schema`] is open. It is built from the
/// handles before the handler runs, while the record waits in its [`RecordGuard`].
struct MintBack {
    text: Option<String>,
    mirror: Option<Map<String, Value>>,
}

impl MintBack {
    /// The mint-back of `resolution`, with the mirror when `mirror` is open.
    fn new(resolution: &HandleResolution, mirror: bool) -> Self {
        Self {
            text: build_mint_back_text(resolution),
            mirror: mirror
                .then(|| build_structured_mint_back(resolution))
                .flatten(),
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
/// [`CallContext`] joins the context; that snapshot is built only when a hook starts.
async fn start_hooks(
    shared: &Inner,
    request: &CallToolRequestParams,
    context: &RequestContext<RoleServer>,
    client: Option<&ClientIdentity>,
) -> PendingHooks {
    let options = &shared.options;
    let pending = fire(options, options.hook_mode(), shared.sink.as_ref(), || {
        ToolCall::new(request, context, client)
    });
    if pending.any() {
        tokio::task::yield_now().await;
    }
    pending
}

/// A call with tracing off (B-022 step 2, R27, RD-29): the intercept is answered; any other call
/// learns the tool's schema as a traced call does ([`learn_schema`]: `get_tool`, the registry, a
/// rebuild, else TS's heuristic) and goes to the handler without the injected arguments, so a
/// tool's own `context` stays, and with a [`CallContext`] without a session or an agent. Nothing
/// is recorded, and learning the schema writes no log line ([`SchemaLogs::Silent`], P16).
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
    let schema = learn_schema(tracked, &request.name, &context, SchemaLogs::Silent).await;
    let request = stripped(request, schema.entry.as_ref());
    forward(tracked, request, context).await
}

/// The degraded path, when no session ID could be issued (B-022 step 9): the intercept is
/// answered, and any other call goes to the handler with its raw arguments, as in TS (K-05), and a
/// [`CallContext`] without a session or an agent. Nothing is recorded.
async fn degraded<H: ServerHandler>(
    tracked: &Tracked<H>,
    request: CallToolRequestParams,
    context: RequestContext<RoleServer>,
    intercept: bool,
) -> Result<CallToolResponse, ErrorData> {
    if intercept {
        let shared = tracked.agentcat().inner();
        return Ok(answer_report_missing(shared, &request).into());
    }
    forward(tracked, request, context).await
}

/// Hands `request` to the handler as it is, with a [`CallContext`] without a session or an agent
/// in its context, so a tool that reads one keeps working on a call the SDK does not trace (C29).
async fn forward<H: ServerHandler>(
    tracked: &Tracked<H>,
    request: CallToolRequestParams,
    mut context: RequestContext<RoleServer>,
) -> Result<CallToolResponse, ErrorData> {
    let call = CallContext::new(String::new(), None, tracked.agentcat().clone());
    context.extensions.insert(call);
    tracked.inner().call_tool(request, context).await
}

/// `request` without the arguments the SDK injected into its tool ([`strip_arguments`]), by the
/// tool's registry `entry`, else by TS's heuristic.
fn stripped(
    mut request: CallToolRequestParams,
    entry: Option<&ToolEntry>,
) -> CallToolRequestParams {
    let arguments = request
        .arguments
        .take()
        .map(|arguments| strip_arguments(&arguments, entry, &request.name));
    request.arguments = arguments;
    request
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
/// know. With [`SchemaLogs::Silent`] none of these steps writes a log line.
///
/// The tool is foreign when the registry's set of tools that declared their own `session_id`
/// holds it, or when its `get_tool` schema declares one (C19). The mirror gate is the entry's
/// `output_injected`; without an entry it is open only while no listing was ever recorded (B-069).
async fn learn_schema<H: ServerHandler>(
    tracked: &Tracked<H>,
    name: &str,
    context: &RequestContext<RoleServer>,
    logs: SchemaLogs,
) -> Schema {
    let shared = tracked.agentcat().inner();
    let looked_up = match catch_unwind(AssertUnwindSafe(|| tracked.lookup_tool(name))) {
        Ok(found) => found.map(|(_tool, entry)| entry),
        Err(payload) => {
            let message = panic_message(&*payload);
            logs.log(shared, || {
                format!("Warning: Tool lookup for {name} panicked: {message}")
            });
            None
        }
    };
    let mut known = registry_view::<H>(shared, name);
    if looked_up.is_none() && known.entry.is_none() && !known.listed {
        rebuild(tracked, context, logs).await;
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
/// again. With [`SchemaLogs::Silent`] the rebuild is the same but writes no log line.
async fn rebuild<H: ServerHandler>(
    tracked: &Tracked<H>,
    context: &RequestContext<RoleServer>,
    logs: SchemaLogs,
) {
    let shared = tracked.agentcat().inner();
    let pages = AssertUnwindSafe(list_pages(tracked.inner(), context)).catch_unwind();
    let failure = match tokio::time::timeout(REBUILD_TIMEOUT, pages).await {
        Ok(Ok(Ok(pages))) => return record_pages::<H>(shared, pages, logs),
        Ok(Ok(Err(error))) => error.message.into_owned(),
        Ok(Err(payload)) => panic_message(&*payload),
        Err(_elapsed) => {
            let millis = REBUILD_TIMEOUT.as_millis();
            format!("tools/list did not respond within {millis}ms")
        }
    };
    logs.log(shared, || {
        format!("Warning: registry rebuild-on-demand failed - Error: {failure}")
    });
}

/// The pages of a rebuild that worked, through the tool-list pipeline into the registry of the
/// handler type `H` (B-027); the pipeline's lines, then the rebuild line, go to the sink when
/// `logs` writes them.
fn record_pages<H: 'static>(shared: &Inner, pages: Vec<Vec<Tool>>, logs: SchemaLogs) {
    let config = InjectConfig::from_options(&shared.options);
    let lines = shared.with_registry::<H, _>(|registry| {
        let mut lines = Vec::new();
        for tools in pages {
            let list = build_injected_list(&config, tools, registry.state_mut());
            registry.record(&list);
            lines.extend(list.logs);
        }
        lines
    });
    for line in lines {
        logs.log(shared, || line);
    }
    logs.log(shared, || REBUILT.to_owned());
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
/// and its output tokens; a complete result gets the [`MintBack`] and, when it is an error result,
/// the record gets its error: the `captured` one, else the result's (B-097, C23). An
/// `InputRequiredResult` is tagged and not decorated (B-025); any other response is not decorated.
/// Then the guard sends the record. When the response cannot be written as JSON, it is returned
/// untouched and the record is discarded, after a log line.
fn finish(
    guard: RecordGuard<'_>,
    name: &str,
    mint_back: MintBack,
    response: CallToolResponse,
    captured: Option<Captured>,
) -> CallToolResponse {
    let original = match response_json(&response) {
        Ok(original) => original,
        Err(error) => {
            guard.discard(format!(
                "Warning: AgentCat post-handler processing failed for tool {name}; returning the customer's result untouched - {error}"
            ));
            return response;
        }
    };
    let mut result_failure = None;
    let mut input_required = false;
    let response = match response {
        CallToolResponse::Complete(mut result) => {
            if result.is_error == Some(true) {
                result_failure = Some(result_error(&result));
            }
            if let Some(text) = mint_back.text {
                prepend_mint_back(&mut result, text);
            }
            if let Some(mint) = mint_back.mirror {
                mirror_mint_back(&mut result.structured_content, mint);
            }
            CallToolResponse::Complete(result)
        }
        CallToolResponse::InputRequired(result) => {
            input_required = true;
            CallToolResponse::InputRequired(result)
        }
        other => other,
    };
    guard.send(|record| {
        if let Some(failure) = result_failure {
            set_error(record, captured, || failure);
        }
        if input_required {
            record
                .tags
                .insert(AGENTCAT_TAG_MRTR.to_owned(), MRTR_INPUT_REQUIRED.into());
        }
        record.output_tokens = estimate_output_tokens(Some(&original));
        record.response = Some(original);
    });
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

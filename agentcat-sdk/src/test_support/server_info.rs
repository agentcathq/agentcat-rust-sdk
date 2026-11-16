//! Fixtures of the server info heartbeat tests: the [`Demo`] server, whose lists a test sets and
//! breaks; the [`Api`], a [`Publisher`] that records every heartbeat and answers as told; and
//! the [`Harness`], a handle whose delivery posts to an [`Api`] and whose heartbeat reports and
//! log lines a test can count.
use std::collections::VecDeque;
use std::future::{pending, ready};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

use agentcat_api::models::{PublishEventRequest, PublishServerInfoRequest};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, Implementation, InitializeResult,
    JsonObject, ListPromptsResult, ListResourceTemplatesResult, ListResourcesResult,
    ListToolsResult, PaginatedRequestParams, Prompt, PromptArgument, Resource, ResourceTemplate,
    ServerCapabilities, Tool, ToolAnnotations,
};
use rmcp::service::{MaybeSendFuture, RequestContext};
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde_json::{Value, json};
use tokio::runtime::Handle;

use crate::client::AgentCat;
use crate::config::Options;
use crate::delivery::{Delivery, DeliveryConfig, LineWriter, Publisher, ServerInfoAnswer};
use crate::event::CallRecord;
use crate::event::assemble::EventJob;
use crate::inject::{InjectConfig, InjectState};
use crate::server_info::lists::Lists;
use crate::server_info::{LINE_PREFIX, Report, Snapshot};
use crate::sink::{BoxFuture, Sink};

/// A project ID of the real form: `proj_` and 27 more characters.
pub(crate) const PROJECT: &str = "proj_0123456789abcdefghijklmnopq";

/// The length of the description of the `beta` tool.
pub(crate) const LONG: usize = 2000;

/// How long a test waits for something that comes at once when the code is right.
const WAIT: Duration = Duration::from_secs(10);

/// How one list handler of a [`Demo`] behaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// It gives its items.
    Works,
    /// It gives a JSON-RPC error.
    Fails,
    /// It never returns.
    Hangs,
    /// It panics.
    Panics,
    /// It gives its first page with the same cursor each time.
    SameCursor,
    /// It gives its first page with a new cursor each time.
    Endless,
}

/// How often each list handler of a [`Demo`] was called. Clones of the server share the counts.
#[derive(Debug, Default)]
pub(crate) struct Listed {
    pub(crate) tools: AtomicUsize,
    pub(crate) resources: AtomicUsize,
    pub(crate) templates: AtomicUsize,
    pub(crate) prompts: AtomicUsize,
}

/// A hand-written server for the heartbeat tests. [`Demo::new`] is `demo-server` 1.2.3 with the
/// title `Demo Title`, the instructions `Use the demo tools.`, the tools `alpha`, `beta` and
/// `gamma` on one page, two resources, one resource template and two prompts. Every field is
/// public, so a test changes what it needs.
///
/// A tool call answers `{"tool": <name>}` as structured content, after `call_takes` when that is
/// not zero. `get_tool` knows every tool of `tool_pages`, so a call never rebuilds the registry.
#[derive(Debug, Clone)]
pub(crate) struct Demo {
    pub(crate) name: &'static str,
    pub(crate) instructions: Option<&'static str>,
    /// The pages of the tool list; the cursor of page `n` (from 0) is `page-<n + 1>`.
    pub(crate) tool_pages: Vec<Vec<Tool>>,
    pub(crate) tools: Mode,
    /// `None` declares no resources capability.
    pub(crate) resources: Option<Mode>,
    pub(crate) resource_items: Vec<Resource>,
    /// `None` declares no prompts capability.
    pub(crate) prompts: Option<Mode>,
    /// How long a tool call takes.
    pub(crate) call_takes: Duration,
    pub(crate) listed: Arc<Listed>,
}

/// The input schema of a demo tool: one string property `text` with `description`.
fn schema(description: &str) -> JsonObject {
    let schema = json!({
        "type": "object",
        "properties": {"text": {"type": "string", "description": description}},
    });
    schema.as_object().cloned().unwrap_or_default()
}

/// The tool `name` with a short description.
pub(crate) fn tool(name: &'static str) -> Tool {
    Tool::new(name, format!("The {name} tool"), schema("Some text"))
}

/// The `beta` tool: a description of [`LONG`] characters, an input schema, an output schema, a
/// title and annotations.
pub(crate) fn beta() -> Tool {
    let output = json!({"type": "object", "properties": {"tool": {"type": "string"}}});
    Tool::new("beta", "b".repeat(LONG), schema("Some text"))
        .with_title("Beta Tool")
        .with_raw_output_schema(Arc::new(output.as_object().cloned().unwrap_or_default()))
        .with_annotations(ToolAnnotations::with_title("Beta").read_only(true))
}

impl Demo {
    /// The fixture server (see the type).
    pub(crate) fn new() -> Self {
        Self {
            name: "demo-server",
            instructions: Some("Use the demo tools."),
            tool_pages: vec![vec![tool("alpha"), beta(), tool("gamma")]],
            tools: Mode::Works,
            resources: Some(Mode::Works),
            resource_items: vec![
                Resource::new("file:///one.txt", "one").with_description("The first file"),
                Resource::new("file:///two.txt", "two"),
            ],
            prompts: Some(Mode::Works),
            call_takes: Duration::ZERO,
            listed: Arc::default(),
        }
    }

    /// [`Demo::new`] with tools only: it declares no resources and no prompts.
    pub(crate) fn tools_only() -> Self {
        Self {
            resources: None,
            prompts: None,
            ..Self::new()
        }
    }

    /// [`Demo::new`] with 5 tools on 3 pages: `alpha` and `beta`, `gamma` and `delta`,
    /// `epsilon`.
    pub(crate) fn paged() -> Self {
        Self {
            tool_pages: vec![
                vec![tool("alpha"), beta()],
                vec![tool("gamma"), tool("delta")],
                vec![tool("epsilon")],
            ],
            ..Self::new()
        }
    }

    /// The page of `items` that `mode` gives for `request`, with the cursor that follows it.
    fn page<T: Clone>(
        mode: Mode,
        pages: &[Vec<T>],
        request: Option<PaginatedRequestParams>,
    ) -> Result<(Vec<T>, Option<String>), ErrorData> {
        let cursor = request.and_then(|request| request.cursor);
        let index = cursor
            .as_deref()
            .and_then(|cursor| cursor.strip_prefix("page-"))
            .and_then(|index| index.parse::<usize>().ok())
            .unwrap_or(0);
        let first = pages.first().cloned().unwrap_or_default();
        match mode {
            Mode::Fails => Err(ErrorData::internal_error("demo list failure", None)),
            Mode::Panics => panic!("demo list panic"),
            Mode::SameCursor => Ok((first, Some("again".to_owned()))),
            Mode::Endless => Ok((first, Some(format!("page-{}", index + 1)))),
            Mode::Works | Mode::Hangs => {
                let items = pages.get(index).cloned().unwrap_or_default();
                let next = (index + 1 < pages.len()).then(|| format!("page-{}", index + 1));
                Ok((items, next))
            }
        }
    }

    /// The future of a list handler in `mode` whose answer is `answer`: one that never ends for
    /// [`Mode::Hangs`].
    async fn answered<T>(mode: Mode, answer: Result<T, ErrorData>) -> Result<T, ErrorData> {
        if mode == Mode::Hangs {
            pending::<()>().await;
        }
        answer
    }
}

impl ServerHandler for Demo {
    fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> + MaybeSendFuture + '_ {
        self.listed.tools.fetch_add(1, Ordering::SeqCst);
        let answer = Self::page(self.tools, &self.tool_pages, request).map(|(tools, next)| {
            let mut result = ListToolsResult::with_all_items(tools);
            result.next_cursor = next;
            result
        });
        Self::answered(self.tools, answer)
    }

    fn list_resources(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourcesResult, ErrorData>> + MaybeSendFuture + '_ {
        self.listed.resources.fetch_add(1, Ordering::SeqCst);
        let mode = self.resources.unwrap_or(Mode::Works);
        let pages = [self.resource_items.clone()];
        let answer = Self::page(mode, &pages, request).map(|(resources, next)| {
            let mut result = ListResourcesResult::with_all_items(resources);
            result.next_cursor = next;
            result
        });
        Self::answered(mode, answer)
    }

    fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourceTemplatesResult, ErrorData>> + MaybeSendFuture + '_
    {
        self.listed.templates.fetch_add(1, Ordering::SeqCst);
        let template = ResourceTemplate::new("file:///{name}.txt", "files")
            .with_description("Any file by name");
        ready(Ok(ListResourceTemplatesResult::with_all_items(vec![
            template,
        ])))
    }

    fn list_prompts(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListPromptsResult, ErrorData>> + MaybeSendFuture + '_ {
        self.listed.prompts.fetch_add(1, Ordering::SeqCst);
        let mode = self.prompts.unwrap_or(Mode::Works);
        let argument = PromptArgument::new("topic")
            .with_description("What to write about")
            .with_required(true);
        let pages = [vec![
            Prompt::new("greet", Some("Greets the user"), None),
            Prompt::new("write", Some("Writes a text"), Some(vec![argument])),
        ]];
        let answer = Self::page(mode, &pages, request).map(|(prompts, next)| {
            let mut result = ListPromptsResult::with_all_items(prompts);
            result.next_cursor = next;
            result
        });
        Self::answered(mode, answer)
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tool_pages
            .iter()
            .flatten()
            .find(|tool| tool.name == name)
            .cloned()
    }

    #[expect(
        clippy::manual_async_fn,
        reason = "the signature of rmcp's trait method: a returned future with MaybeSendFuture"
    )]
    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResponse, ErrorData>> + MaybeSendFuture + '_ {
        async move {
            if !self.call_takes.is_zero() {
                tokio::time::sleep(self.call_takes).await;
            }
            Ok(CallToolResult::structured(json!({"tool": request.name})).into())
        }
    }

    fn get_info(&self) -> InitializeResult {
        let capabilities = match (self.resources.is_some(), self.prompts.is_some()) {
            (true, true) => ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .enable_prompts()
                .build(),
            (true, false) => ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
            (false, true) => ServerCapabilities::builder()
                .enable_tools()
                .enable_prompts()
                .build(),
            (false, false) => ServerCapabilities::builder().enable_tools().build(),
        };
        let mut info = InitializeResult::new(capabilities);
        info.server_info = Implementation::new(self.name, "1.2.3").with_title("Demo Title");
        info.instructions = self.instructions.map(str::to_owned);
        info
    }
}

/// The snapshot of a read of [`Demo::new`] for a handle with the default options, started now on
/// the current runtime: its tools, its resources, and no other list.
pub(crate) fn snapshot() -> Snapshot {
    let demo = Demo::new();
    Snapshot {
        server: Implementation::new(demo.name, "1.2.3").with_title("Demo Title"),
        instructions: demo.instructions.map(str::to_owned),
        lists: Lists {
            tool_pages: demo.tool_pages.clone(),
            resources: Some(demo.resource_items.clone()),
            resource_templates: None,
            prompts: None,
            left_out: Vec::new(),
        },
        inject: InjectConfig::from_options(&Options::default()),
        state: InjectState::default(),
        runtime: Handle::try_current().ok(),
        started: std::time::Instant::now(),
    }
}

/// How an [`Api`] answers one heartbeat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Answer {
    /// `202`.
    Accepted,
    /// `413`.
    TooLarge,
    /// `500`.
    ServerError,
    /// No answer, ever.
    Never,
}

/// What the clones of one [`Api`] share.
#[derive(Default)]
struct Calls {
    /// The answers of the next heartbeats, in order; after them every heartbeat is accepted.
    answers: Mutex<VecDeque<Answer>>,
    /// The body of every heartbeat, in the order they came.
    heartbeats: Mutex<Vec<PublishServerInfoRequest>>,
    /// The request of every event, in the order they came.
    events: Mutex<Vec<PublishEventRequest>>,
}

/// A [`Publisher`] that records every heartbeat and every event and sends nothing. Clones share
/// the record.
#[derive(Clone, Default)]
pub(crate) struct Api(Arc<Calls>);

impl Api {
    /// An API that answers the next heartbeats with `answers`, in order, and accepts the rest.
    pub(crate) fn answering(answers: &[Answer]) -> Self {
        let api = Self::default();
        api.0
            .answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend(answers);
        api
    }

    /// The body of every heartbeat so far, as the JSON the API would get.
    pub(crate) fn heartbeats(&self) -> Vec<Value> {
        self.0
            .heartbeats
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|body| serde_json::to_value(body).expect("a body is JSON"))
            .collect()
    }

    /// The request of every event so far.
    pub(crate) fn events(&self) -> Vec<PublishEventRequest> {
        self.0
            .events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Publisher for Api {
    fn publish(&self, request: PublishEventRequest) -> BoxFuture<'static, Result<(), String>> {
        self.0
            .events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        Box::pin(ready(Ok(())))
    }

    fn publish_server_info(
        &self,
        request: PublishServerInfoRequest,
    ) -> BoxFuture<'static, Result<ServerInfoAnswer, String>> {
        self.0
            .heartbeats
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request);
        let answer = self
            .0
            .answers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .unwrap_or(Answer::Accepted);
        match answer {
            Answer::Accepted => Box::pin(ready(Ok(ServerInfoAnswer::Accepted))),
            Answer::TooLarge => Box::pin(ready(Ok(ServerInfoAnswer::TooLarge))),
            Answer::ServerError => Box::pin(ready(Err("the API answered 500".to_owned()))),
            Answer::Never => Box::pin(pending()),
        }
    }
}

/// The heartbeat lines a delivery worker wrote, and the signal of each new one.
#[derive(Clone, Default)]
struct HeartbeatLines(Arc<(Mutex<Vec<String>>, Condvar)>);

impl HeartbeatLines {
    /// A writer that keeps the lines of the heartbeat and drops every other line.
    fn writer(&self) -> LineWriter {
        let lines = self.clone();
        Arc::new(move |line: &str| {
            if line.starts_with(LINE_PREFIX) {
                let (kept, added) = &*lines.0;
                kept.lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(line.to_owned());
                added.notify_all();
            }
        })
    }

    /// Blocks until there are `count` lines, for at most [`WAIT`], and gives the lines.
    fn wait_for(&self, count: usize) -> Vec<String> {
        let (kept, added) = &*self.0;
        let kept = kept.lock().unwrap_or_else(PoisonError::into_inner);
        let (kept, _) = added
            .wait_timeout_while(kept, WAIT, |kept| kept.len() < count)
            .unwrap_or_else(PoisonError::into_inner);
        kept.clone()
    }
}

/// A [`Delivery`] that counts the heartbeat reports it takes.
struct Counted {
    delivery: Delivery,
    reports: Arc<AtomicUsize>,
}

impl Sink for Counted {
    fn log(&self, message: String) {
        self.delivery.log(message);
    }

    fn record(&self, record: CallRecord) {
        self.delivery.record(record);
    }

    fn event(&self, job: EventJob) {
        self.delivery.event(job);
    }

    fn server_info(&self, report: Report) {
        self.reports.fetch_add(1, Ordering::SeqCst);
        self.delivery.server_info(report);
    }

    fn flush(&self, timeout: Duration) -> BoxFuture<'static, bool> {
        self.delivery.flush(timeout)
    }

    fn shutdown(&self, timeout: Duration) -> BoxFuture<'static, bool> {
        self.delivery.shutdown(timeout)
    }
}

/// A handle whose delivery posts to an [`Api`], with what a heartbeat test reads.
pub(crate) struct Harness {
    pub(crate) agentcat: AgentCat,
    pub(crate) api: Api,
    lines: HeartbeatLines,
    reports: Arc<AtomicUsize>,
}

impl Harness {
    /// A handle for [`PROJECT`] with the default options, whose API accepts every heartbeat.
    pub(crate) fn new() -> Self {
        Self::with(PROJECT, Options::default(), Api::default())
    }

    /// A handle for `project` with `options` that posts to `api`.
    pub(crate) fn with(project: &str, options: Options, api: Api) -> Self {
        let lines = HeartbeatLines::default();
        let reports = Arc::new(AtomicUsize::new(0));
        let delivery = Delivery::with_writer(
            project.to_owned(),
            &options,
            Arc::new(api.clone()),
            None,
            DeliveryConfig::default(),
            lines.writer(),
        );
        let sink = Counted {
            delivery,
            reports: Arc::clone(&reports),
        };
        Self {
            agentcat: AgentCat::with_sink(project, options, Arc::new(sink)),
            api,
            lines,
            reports,
        }
    }

    /// How many checks found a heartbeat due so far: each one hands the sink one report, when
    /// its read ends or is dropped.
    pub(crate) fn reports(&self) -> usize {
        self.reports.load(Ordering::SeqCst)
    }

    /// Waits until the worker wrote `count` heartbeat lines, and gives the lines. It waits on a
    /// blocking thread, so the runtime of the test keeps running and a paused clock stays where
    /// it is.
    ///
    /// # Panics
    ///
    /// When fewer lines come in time.
    pub(crate) async fn lines(&self, count: usize) -> Vec<String> {
        let lines = self.lines.clone();
        let lines = tokio::task::spawn_blocking(move || lines.wait_for(count))
            .await
            .expect("the wait ends");
        assert!(lines.len() >= count, "{count} heartbeat lines: {lines:?}");
        lines
    }

    /// Waits until every report so far has its line, and gives the heartbeats the API got.
    pub(crate) async fn heartbeats(&self) -> Vec<Value> {
        self.lines(self.reports()).await;
        self.api.heartbeats()
    }
}

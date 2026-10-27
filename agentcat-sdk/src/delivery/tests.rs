//! The delivery queue and its worker thread (C36, B-100 to B-105): what the worker posts and logs
//! (TS's lines, retries after the configured delays, telemetry-only events never posted), the
//! queue's limits (drop the oldest pending, at most `concurrency` in flight, a push that never
//! blocks), `flush` and `shutdown` with their deadlines and the blocking twins, their edge cases
//! (before the first event, twice, cancelled, after a timed-out shutdown, the stop's wait for the
//! aborted tasks), and the drain on the last drop (`Queue::close`), off and on the worker thread.
//! Every test shrinks the budgets through [`DeliveryConfig`] and writes the worker's lines to a
//! [`CapturedLines`], except `log_lines_are_written_by_the_worker`, which uses the process logger.
//! The TS texts come from `queue.json` (`success_line_matches_ts` here; the retry cases run in
//! `pipeline::process::tests::queue_retry_vectors_match_ts`).
use std::future::pending;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use agentcat_api::models::PublishEventRequest;
use agentcat_api::models::publish_event_request::EventType;
use rmcp::ServerHandler;
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ClientCapabilities, Implementation};
use rmcp::model::{InitializeRequestParams, ProtocolVersion};
use serde_json::{Map, Value, json};
use tokio::runtime::{Builder, Handle, Runtime};
use tokio::sync::{Semaphore, oneshot};

use super::queue::{Kind, Queue, Reply, Worker};
use super::{Delivery, DeliveryConfig, Observer, Publisher};
use crate::client::AgentCat;
use crate::config::{HookError, LogDestination, Options, UserIdentity};
use crate::event::assemble::EventJob;
use crate::hooks::PendingHooks;
use crate::logging;
use crate::pipeline::TsEvent;
use crate::sink::{BoxFuture, Sink};
use crate::test_support::queue::{
    self, CapturedLines, DropSignal, MOCK_FAILURE, RecordingPublisher,
};
use crate::test_support::rmcp::{Sample, connect};
use crate::test_support::vectors;

/// The project of the test events.
const PROJECT: &str = "proj_delivery";

/// How long a test waits for something that comes at once when the code is right.
const WAIT: Duration = Duration::from_secs(2);

/// The name of the worker thread.
const WORKER_THREAD: &str = "agentcat-delivery";

/// TS's line for a successful attempt starts with this.
const SUCCESS: &str = "Successfully sent event ";

/// `n` milliseconds.
const fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// TS's queue limits with short retry delays, hook budget and drain.
fn fast() -> DeliveryConfig {
    DeliveryConfig {
        max_pending: 10_000,
        concurrency: 5,
        retry_delays: [ms(10), ms(20), ms(40)],
        hook_budget: ms(50),
        drop_drain: ms(1_000),
    }
}

/// The JSON object `value`.
fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(object) => object,
        other => panic!("{other} is not an object"),
    }
}

/// The job of a tool-call event for `project` whose `resourceName` is `name`, with no hooks and
/// no redaction.
fn job(project: &str, name: &str) -> EventJob {
    EventJob {
        event: object(json!({
            "id": "",
            "sessionId": "ses_delivery000000000000000000",
            "projectId": project,
            "eventType": "mcp:tools/call",
            "timestamp": "2026-10-23T12:00:00.000Z",
            "duration": 5,
            "resourceName": name,
            "tags": {},
        })),
        hooks: PendingHooks::default(),
        hook_mode: false,
        runtime: None,
        string_redaction: false,
        event_redaction: false,
        backtrace: None,
    }
}

/// A handle for `project` whose delivery posts through `publisher`, tells `observer` and writes its
/// lines to the returned capture.
fn handle(
    project: &str,
    options: Options,
    publisher: &RecordingPublisher,
    observer: Option<Arc<dyn Observer>>,
    config: DeliveryConfig,
) -> (AgentCat, CapturedLines) {
    let lines = CapturedLines::default();
    let delivery = Delivery::with_writer(
        project.to_owned(),
        &options,
        Arc::new(publisher.clone()),
        observer,
        config,
        lines.writer(),
    );
    (
        AgentCat::with_sink(project, options, Arc::new(delivery)),
        lines,
    )
}

/// Pushes `job` as the call path does.
fn push(agentcat: &AgentCat, job: EventJob) {
    agentcat.inner().sink.event(job);
}

/// Waits until `count` more attempts started.
fn wait_starts(starts: &mpsc::Receiver<()>, count: usize) {
    for _ in 0..count {
        starts.recv_timeout(WAIT).expect("an attempt starts");
    }
}

/// A runtime with one worker thread for the customer's hooks.
fn hooks_runtime() -> Runtime {
    Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap()
}

/// A hook task on `runtime` that never settles.
fn hanging<T: Send + 'static>(
    runtime: &Runtime,
    signal: Option<DropSignal>,
) -> tokio::task::JoinHandle<Result<Option<T>, HookError>> {
    runtime.spawn(async move {
        let _signal = signal;
        pending().await
    })
}

/// An observer that keeps every event it is told about.
#[derive(Debug, Default)]
struct RecordingObserver(Mutex<Vec<TsEvent>>);

impl Observer for RecordingObserver {
    fn finished(&self, event: &TsEvent) {
        self.0.lock().unwrap().push(event.clone());
    }
}

/// A server with no tools: its listing gives the call path a warning line.
struct Empty;

impl ServerHandler for Empty {}

// ------------------------------------------------------------------------------- defaults

/// B-100 to B-102, C37, R06: the default budgets are TS's queue values (10 000 pending, 5 in
/// flight, retries after 1, 2 and 4 s, 30 s per hook) and the 5 s drain of the last drop.
#[test]
fn default_config_is_ts_queue() {
    let config = DeliveryConfig::default();
    assert_eq!(config.max_pending, 10_000);
    assert_eq!(config.concurrency, 5);
    assert_eq!(config.retry_delays, [ms(1_000), ms(2_000), ms(4_000)]);
    assert_eq!(config.hook_budget, Duration::from_secs(30));
    assert_eq!(config.drop_drain, Duration::from_secs(5));
}

// ------------------------------------------------------------------------------- publishing

/// B-077, B-087: the record of a tool call served by rmcp is assembled on the worker and posted
/// once, with the handle's project, the tool, the server and client of the call, a minted session
/// and an `evt_` ID (TS's "should process events end-to-end through publishEvent" and "should
/// publish event with server tracking data and session info").
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn call_record_is_published() {
    let publisher = RecordingPublisher::new();
    let (agentcat, lines) = handle(PROJECT, Options::default(), &publisher, None, fast());
    let client = connect(agentcat.track(Sample), "delivery-client").await;
    let arguments = object(json!({"msg": "hi"}));
    client
        .call_tool(CallToolRequestParams::new("echo").with_arguments(arguments))
        .await
        .unwrap();
    client.cancel().await.unwrap();
    assert!(agentcat.flush(WAIT).await);

    let requests = publisher.requests();
    assert_eq!(requests.len(), 1, "{requests:?}");
    let request = &requests[0];
    assert_eq!(request.project_id, PROJECT);
    assert_eq!(request.event_type, Some(EventType::McpColonToolsSlashCall));
    assert_eq!(request.resource_name.as_deref(), Some("echo"));
    assert_eq!(request.client_name.as_deref(), Some("delivery-client"));
    assert!(request.server_name.is_some(), "{request:?}");
    assert!(matches!(&request.session_id, Some(Some(session)) if session.starts_with("ses_")));
    let id = request.id.clone().unwrap();
    assert!(id.starts_with("evt_"), "{id}");
    let lines = lines.lines();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].starts_with(&format!("{SUCCESS}{id} | mcp:tools/call | session ses_")));
}

/// B-003, B-104, C43: an event without a project (telemetry only) finishes the pipeline and goes
/// to the observer, but is never posted and logs no success line; an event with a project goes to
/// both.
#[test]
fn telemetry_only_is_not_posted_but_observed() {
    let publisher = RecordingPublisher::new();
    let observer = Arc::new(RecordingObserver::default());
    let (agentcat, lines) = handle(
        "",
        Options::default(),
        &publisher,
        Some(observer.clone()),
        fast(),
    );
    push(&agentcat, job("", "telemetry"));
    push(&agentcat, job(PROJECT, "posted"));
    assert!(agentcat.flush_blocking(WAIT));

    assert_eq!(publisher.resource_names(), ["posted"]);
    let seen = observer.0.lock().unwrap().clone();
    let mut names: Vec<&str> = seen
        .iter()
        .map(|event| event["resourceName"].as_str().unwrap())
        .collect();
    names.sort_unstable();
    assert_eq!(names, ["posted", "telemetry"]);
    for event in &seen {
        assert!(
            event["id"].as_str().unwrap().starts_with("evt_"),
            "{event:?}"
        );
    }
    let lines = lines.lines();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].starts_with(SUCCESS) && lines[0].contains(PROJECT));
}

/// B-102: a failing publish is tried 4 times: after each failure TS's `Failed to send event <id>,
/// retrying... [Error: <text>]`, then the next configured delay, and after the fourth `Failed to
/// deliver AgentCat event after retries: <text>`; `flush` then returns `true`, the event settled
/// (TS's "settles fail-open when `AgentCat` ingestion exhausts retries"). The worker thread writes
/// every line.
#[test]
fn retries_three_times_then_logs_failure() {
    let publisher = RecordingPublisher::failing(usize::MAX);
    let config = DeliveryConfig {
        retry_delays: [ms(30), ms(60), ms(120)],
        ..fast()
    };
    let (agentcat, lines) = handle(PROJECT, Options::default(), &publisher, None, config);
    push(&agentcat, job(PROJECT, "failing"));
    assert!(agentcat.flush_blocking(WAIT));

    let attempts = publisher.attempts();
    assert_eq!(attempts.len(), 4, "{attempts:?}");
    for (pair, delay) in attempts.windows(2).zip(config.retry_delays) {
        let gap = pair[1].at - pair[0].at;
        assert!(gap >= delay, "{gap:?} < {delay:?}");
    }
    let id = attempts[0].request.id.clone().unwrap();
    let mut expected =
        vec![format!("Failed to send event {id}, retrying... [Error: {MOCK_FAILURE}]"); 4];
    expected.push(format!(
        "Failed to deliver AgentCat event after retries: {MOCK_FAILURE}"
    ));
    assert_eq!(lines.lines(), expected);
    for (line, thread) in lines.entries() {
        assert_eq!(thread.as_deref(), Some(WORKER_THREAD), "{line}");
    }
}

/// A publisher that can never send (C44, a client that failed to build).
#[derive(Debug)]
struct Unusable(Arc<std::sync::atomic::AtomicUsize>);

impl Publisher for Unusable {
    fn publish(&self, _request: PublishEventRequest) -> BoxFuture<'static, Result<(), String>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Box::pin(std::future::ready(Ok(())))
    }

    fn prepare(&self, _log: &mut Vec<String>) -> Result<(), String> {
        Err("the HTTP client could not be built".to_owned())
    }
}

/// C44: an unusable publisher gets no attempt and no retry. The final failure line is logged at
/// once, well under one retry delay, and `flush` returns `true`.
#[test]
fn unusable_publisher_drops_the_event_at_once() {
    let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let lines = CapturedLines::default();
    let delivery = Delivery::with_writer(
        PROJECT.to_owned(),
        &Options::default(),
        Arc::new(Unusable(Arc::clone(&attempts))),
        None,
        DeliveryConfig {
            retry_delays: [ms(1_000), ms(2_000), ms(4_000)],
            ..fast()
        },
        lines.writer(),
    );
    let agentcat = AgentCat::with_sink(PROJECT, Options::default(), Arc::new(delivery));
    push(&agentcat, job(PROJECT, "unsendable"));
    let started = Instant::now();
    assert!(agentcat.flush_blocking(WAIT));
    assert!(started.elapsed() < ms(500), "{:?}", started.elapsed());
    assert_eq!(attempts.load(Ordering::SeqCst), 0);
    assert_eq!(
        lines.lines(),
        ["Failed to deliver AgentCat event after retries: the HTTP client could not be built"]
    );
}

/// Design §6.7, C44: the real publisher builds its client on the worker thread, at the first
/// event, not when the handle is built: the failing build here names the thread it ran on. A
/// build that fails is logged once, and each event is then dropped at once with TS's final
/// failure line (here with retry delays of 1 s and more, which a retry would show).
#[test]
fn failed_client_build_is_logged_once_by_the_worker() {
    let publisher = super::http::ApiPublisher::with_build("http://127.0.0.1:9".to_owned(), |_| {
        let thread = std::thread::current();
        Err(format!(
            "no TLS backend on {}",
            thread.name().unwrap_or("?")
        ))
    });
    let lines = CapturedLines::default();
    let delivery = Delivery::with_writer(
        PROJECT.to_owned(),
        &Options::default(),
        Arc::new(publisher),
        None,
        DeliveryConfig {
            retry_delays: [ms(1_000), ms(2_000), ms(4_000)],
            ..fast()
        },
        lines.writer(),
    );
    let agentcat = AgentCat::with_sink(PROJECT, Options::default(), Arc::new(delivery));
    assert!(agentcat.flush_blocking(WAIT));
    assert!(lines.lines().is_empty(), "{:?}", lines.lines());

    push(&agentcat, job(PROJECT, "first"));
    push(&agentcat, job(PROJECT, "second"));
    let started = Instant::now();
    assert!(agentcat.flush_blocking(WAIT));
    assert!(started.elapsed() < ms(500), "{:?}", started.elapsed());
    let failed =
        "Failed to deliver AgentCat event after retries: the HTTP client could not be built";
    assert_eq!(
        lines.lines(),
        [
            "Failed to build the HTTP client, events will not be sent: no TLS backend on \
             agentcat-delivery",
            failed,
            failed,
        ]
    );
    for (line, thread) in lines.entries() {
        assert_eq!(thread.as_deref(), Some(WORKER_THREAD), "{line}");
    }
}

/// TS sets an event's ID once, before `sendEvent`, so every attempt sends the same `evt_` ID and
/// every line names it. `queue.json` masks the IDs as `<evt>`, so only this test pins it.
#[test]
fn retry_attempts_share_one_event_id() {
    let publisher = RecordingPublisher::failing(2);
    let (agentcat, lines) = handle(PROJECT, Options::default(), &publisher, None, fast());
    push(&agentcat, job(PROJECT, "retried"));
    assert!(agentcat.flush_blocking(WAIT));

    let ids: Vec<String> = publisher
        .requests()
        .into_iter()
        .map(|request| request.id.unwrap())
        .collect();
    assert_eq!(ids.len(), 3, "{ids:?}");
    assert!(ids[0].starts_with("evt_"), "{ids:?}");
    assert!(ids.iter().all(|id| *id == ids[0]), "{ids:?}");
    let lines = lines.lines();
    assert_eq!(lines.len(), 3, "{lines:?}");
    for line in &lines {
        assert!(line.contains(&ids[0]), "{line}");
        assert_eq!(
            queue::mask_event_ids(line).matches("<evt>").count(),
            1,
            "{line}"
        );
    }
}

/// Each non-retry `queue.json` case through the worker with a publisher that works: every line the
/// worker writes must be TS's after [`queue::worker_logs`], the success line of a posted event
/// included (`session ` with an empty session, `undefined ms` without a duration, the identify
/// hook's user or `anonymous`), with `<evt>` for each `evt_` ID and `<minted>` for a session the
/// hooks minted. A telemetry-only case logs no success line.
#[test]
fn success_line_matches_ts() {
    let runtime = hooks_runtime();
    let mut ran = 0;
    for case in queue::cases() {
        let name = vectors::name(&case).to_owned();
        if name.starts_with("retry:") {
            continue;
        }
        let publisher = RecordingPublisher::new();
        let lines = CapturedLines::default();
        let delivery = Delivery::with_writer(
            String::new(),
            &queue::options_of(&case),
            Arc::new(publisher.clone()),
            None,
            fast(),
            lines.writer(),
        );
        delivery.event(queue::job_of(&case, Some(runtime.handle())));
        assert!(delivery.flush_blocking(WAIT), "{name}");
        drop(delivery);

        let minted = publisher.requests().first().and_then(minted_session);
        let logs: Vec<String> = lines
            .lines()
            .iter()
            .map(|line| {
                let line = queue::mask_event_ids(line);
                match &minted {
                    Some(session) => line.replace(session.as_str(), "<minted>"),
                    None => line,
                }
            })
            .collect();
        assert_eq!(logs, queue::worker_logs(&case), "{name}");
        ran += 1;
    }
    assert_eq!(ran, 56, "every non-retry case ran");
}

/// The session of `request` when its `agentcat_session_id_source` tag says the SDK minted it.
fn minted_session(request: &PublishEventRequest) -> Option<String> {
    let source = request.tags.as_ref()?.get("agentcat_session_id_source")?;
    if source != "minted" {
        return None;
    }
    request.session_id.clone().flatten()
}

// -------------------------------------------------------------------------------- the queue

/// B-100: a push onto `max_pending` pending events drops the oldest pending one, not the one in
/// flight, with TS's `Event queue full, dropping oldest event` (TS's "settles an event when queue
/// overflow drops it": `flush` does not wait for the dropped event).
#[test]
fn overflow_drops_oldest_pending() {
    let (publisher, gate) = RecordingPublisher::gated();
    let starts = publisher.starts();
    let config = DeliveryConfig {
        max_pending: 3,
        concurrency: 1,
        ..fast()
    };
    let (agentcat, lines) = handle(PROJECT, Options::default(), &publisher, None, config);
    push(&agentcat, job(PROJECT, "a"));
    wait_starts(&starts, 1);
    for name in ["b", "c", "d", "e"] {
        push(&agentcat, job(PROJECT, name));
    }
    gate.open();
    assert!(agentcat.flush_blocking(WAIT));

    assert_eq!(publisher.resource_names(), ["a", "c", "d", "e"]);
    let other: Vec<String> = lines
        .lines()
        .into_iter()
        .filter(|line| !line.starts_with(SUCCESS))
        .collect();
    assert_eq!(other, ["Event queue full, dropping oldest event"]);
}

/// Ruling P10 (RD-35's reason): the job an overflow drops aborts its hook tasks on the customer's
/// runtime instead of leaving them running detached.
#[test]
fn overflow_drop_aborts_hooks() {
    let runtime = hooks_runtime();
    let (publisher, gate) = RecordingPublisher::gated();
    let starts = publisher.starts();
    let config = DeliveryConfig {
        max_pending: 1,
        concurrency: 1,
        ..fast()
    };
    let (agentcat, _lines) = handle(PROJECT, Options::default(), &publisher, None, config);
    push(&agentcat, job(PROJECT, "a"));
    wait_starts(&starts, 1);
    let (signal, dropped) = DropSignal::new();
    let mut hooked = job(PROJECT, "b");
    hooked.hooks.identify = Some(hanging::<UserIdentity>(&runtime, Some(signal)));
    push(&agentcat, hooked);
    push(&agentcat, job(PROJECT, "c"));
    dropped
        .recv_timeout(WAIT)
        .expect("the dropped job's hook is aborted");

    gate.open();
    assert!(agentcat.flush_blocking(WAIT));
    assert_eq!(publisher.resource_names(), ["a", "c"]);
}

/// B-100, design §7.3: with every slot held, 10 000 pushes take under a second: a push only
/// appends under a short lock and wakes the worker.
#[test]
fn enqueue_never_blocks() {
    let (publisher, _gate) = RecordingPublisher::gated();
    let starts = publisher.starts();
    let (agentcat, _lines) = handle(PROJECT, Options::default(), &publisher, None, fast());
    for _ in 0..5 {
        push(&agentcat, job(PROJECT, "held"));
    }
    wait_starts(&starts, 5);

    let jobs: Vec<EventJob> = (0..10_000).map(|_| job(PROJECT, "queued")).collect();
    let started = Instant::now();
    for job in jobs {
        push(&agentcat, job);
    }
    let elapsed = started.elapsed();
    assert!(elapsed < Duration::from_secs(1), "{elapsed:?}");
    assert!(!agentcat.shutdown_blocking(ms(50)));
}

/// B-101: at most 5 events are in flight at once, each holding its slot through its attempts; the
/// others wait in the queue, and all of them are posted in the end (TS's "should handle multiple
/// events").
#[test]
fn at_most_five_in_flight() {
    let (publisher, gate) = RecordingPublisher::gated();
    let starts = publisher.starts();
    let (agentcat, _lines) = handle(PROJECT, Options::default(), &publisher, None, fast());
    for index in 0..12 {
        push(&agentcat, job(PROJECT, &format!("event-{index}")));
    }
    wait_starts(&starts, 5);
    // The deadline of this flush gives the worker the time to start a sixth attempt if it would.
    assert!(!agentcat.flush_blocking(ms(100)));
    assert_eq!(publisher.attempts().len(), 5);

    gate.open();
    assert!(agentcat.flush_blocking(WAIT));
    assert_eq!(publisher.attempts().len(), 12);
    assert_eq!(publisher.most_running(), 5);
}

// ------------------------------------------------------------------------- flush, shutdown

/// R06, C36: `flush` answers once the queue and the in-flight events are empty: it waits for an
/// attempt in flight and returns `true` after it ends (TS's "keeps the registered task pending
/// until `AgentCat` ingestion completes").
#[test]
fn flush_waits_for_in_flight() {
    let (publisher, gate) = RecordingPublisher::gated();
    let starts = publisher.starts();
    let (agentcat, _lines) = handle(PROJECT, Options::default(), &publisher, None, fast());
    push(&agentcat, job(PROJECT, "held"));
    wait_starts(&starts, 1);

    let (answer_tx, answer) = mpsc::channel();
    let flusher = agentcat.clone();
    let thread = std::thread::spawn(move || {
        answer_tx.send(flusher.flush_blocking(WAIT)).unwrap();
    });
    assert_eq!(
        answer.recv_timeout(ms(100)),
        Err(mpsc::RecvTimeoutError::Timeout),
        "flush waits while the attempt is held"
    );
    gate.open();
    assert_eq!(answer.recv_timeout(WAIT), Ok(true));
    assert_eq!(publisher.finished(), 1);
    thread.join().unwrap();
}

/// R06, C36, review focus 1: with an event held by a hook that hangs, `flush` returns `false` at
/// its deadline, in its blocking form and in its async form on a runtime without a time driver:
/// the worker owns the deadline, so the customer's runtime needs no timer.
#[test]
fn flush_returns_false_at_deadline() {
    let hooks = hooks_runtime();
    let publisher = RecordingPublisher::new();
    let config = DeliveryConfig {
        hook_budget: Duration::from_secs(10),
        ..fast()
    };
    let (agentcat, _lines) = handle(PROJECT, Options::default(), &publisher, None, config);
    let mut held = job(PROJECT, "held");
    held.hooks.identify = Some(hanging::<UserIdentity>(&hooks, None));
    push(&agentcat, held);

    let started = Instant::now();
    assert!(!agentcat.flush_blocking(ms(150)));
    let elapsed = started.elapsed();
    assert!(
        ms(150) <= elapsed && elapsed < Duration::from_secs(1),
        "{elapsed:?}"
    );

    let no_timer = Builder::new_current_thread().build().unwrap();
    let started = Instant::now();
    assert!(!no_timer.block_on(agentcat.flush(ms(150))));
    let elapsed = started.elapsed();
    assert!(
        ms(150) <= elapsed && elapsed < Duration::from_secs(1),
        "{elapsed:?}"
    );

    assert!(publisher.attempts().is_empty());
    assert!(!agentcat.shutdown_blocking(ms(50)));
}

/// C12, B-105: `shutdown` drains, then a push is dropped with TS's `Queue is shutting down, event
/// dropped`, which the worker writes (TS's "should prevent adding events after destroy").
#[tokio::test]
async fn shutdown_rejects_later_events() {
    let publisher = RecordingPublisher::new();
    let (agentcat, lines) = handle(PROJECT, Options::default(), &publisher, None, fast());
    push(&agentcat, job(PROJECT, "before"));
    assert!(agentcat.shutdown(WAIT).await);
    push(&agentcat, job(PROJECT, "after"));
    assert!(agentcat.flush(WAIT).await);

    assert_eq!(publisher.resource_names(), ["before"]);
    let other: Vec<(String, Option<String>)> = lines
        .entries()
        .into_iter()
        .filter(|(line, _)| !line.starts_with(SUCCESS))
        .collect();
    assert_eq!(
        other,
        [(
            "Queue is shutting down, event dropped".to_owned(),
            Some(WORKER_THREAD.to_owned())
        )]
    );
}

/// C36, C12, B-105: a `shutdown` that reaches its deadline abandons what is left after TS's
/// `Shutting down with 2 events still in queue` (the pending queue only, not the event in flight)
/// and returns `false`.
#[test]
fn shutdown_logs_events_left() {
    let (publisher, _gate) = RecordingPublisher::gated();
    let starts = publisher.starts();
    let config = DeliveryConfig {
        concurrency: 1,
        ..fast()
    };
    let (agentcat, lines) = handle(PROJECT, Options::default(), &publisher, None, config);
    for name in ["a", "b", "c"] {
        push(&agentcat, job(PROJECT, name));
    }
    wait_starts(&starts, 1);

    let started = Instant::now();
    assert!(!agentcat.shutdown_blocking(ms(100)));
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(
        lines.lines(),
        ["Shutting down with 2 events still in queue"]
    );
    assert_eq!(publisher.resource_names(), ["a"]);
    assert_eq!(
        publisher.finished(),
        0,
        "the attempt in flight is abandoned"
    );
}

/// R06: the blocking twins work on a thread without a tokio runtime: `flush_blocking` waits for
/// the queue to empty, and `shutdown_blocking` drains before it stops (TS's "should handle
/// destroy method without errors").
#[test]
fn blocking_twins_work_outside_a_runtime() {
    assert!(Handle::try_current().is_err());
    let publisher = RecordingPublisher::slow(ms(20));
    let (agentcat, _lines) = handle(PROJECT, Options::default(), &publisher, None, fast());
    push(&agentcat, job(PROJECT, "a"));
    assert!(agentcat.flush_blocking(WAIT));
    assert_eq!(publisher.finished(), 1);
    push(&agentcat, job(PROJECT, "b"));
    assert!(agentcat.shutdown_blocking(WAIT));
    assert_eq!(publisher.finished(), 2);
}

// --------------------------------------------------------------------- lifecycle edge cases

/// A deadline that no test reaches.
const LONG: Duration = Duration::from_secs(60);

/// A delivery for [`PROJECT`] with no handle around it, so a test can read its queue, that posts
/// through `publisher` and writes its lines to the returned capture.
fn bare_delivery(
    publisher: &RecordingPublisher,
    config: DeliveryConfig,
) -> (Delivery, CapturedLines) {
    let lines = CapturedLines::default();
    let delivery = Delivery::with_writer(
        PROJECT.to_owned(),
        &Options::default(),
        Arc::new(publisher.clone()),
        None,
        config,
        lines.writer(),
    );
    (delivery, lines)
}

/// The state of the worker of `delivery`.
fn worker_of(delivery: &Delivery) -> Worker {
    delivery.shared.lock().worker
}

/// Awaits `request` (the future of a `flush` or a `shutdown`) on a customer's runtime with a
/// timer and drops it after 20 ms, as a customer's timeout does; `true` when it was cancelled.
fn cancel_after_20_ms(request: impl Future<Output = bool>) -> bool {
    let runtime = Builder::new_current_thread().enable_time().build().unwrap();
    runtime.block_on(async { tokio::time::timeout(ms(20), request).await.is_err() })
}

/// The lines of `lines` that are not success lines.
fn other_lines(lines: &CapturedLines) -> Vec<String> {
    lines
        .lines()
        .into_iter()
        .filter(|line| !line.starts_with(SUCCESS))
        .collect()
}

/// C36, C12: a `shutdown` that reaches its deadline stops the worker for good and answers
/// `false`. A `flush` after it answers `true` at once, in both forms: the stop abandoned what was
/// left, so nothing is left to wait for.
#[test]
fn flush_after_timed_out_shutdown_returns_true() {
    let (publisher, _gate) = RecordingPublisher::gated();
    let starts = publisher.starts();
    let (agentcat, _lines) = handle(PROJECT, Options::default(), &publisher, None, fast());
    push(&agentcat, job(PROJECT, "held"));
    wait_starts(&starts, 1);
    assert!(!agentcat.shutdown_blocking(ms(50)));

    let started = Instant::now();
    assert!(agentcat.flush_blocking(WAIT));
    let no_timer = Builder::new_current_thread().build().unwrap();
    assert!(no_timer.block_on(agentcat.flush(WAIT)));
    assert!(started.elapsed() < ms(500), "{:?}", started.elapsed());
    assert_eq!(
        publisher.finished(),
        0,
        "the attempt in flight was abandoned"
    );
}

/// C36: the worker cannot tell a caller that stopped waiting from one that waits. A cancelled
/// async `flush` (its future dropped after its first poll, here by the customer's timeout)
/// leaves its request in the worker's waiting list until the queue and the in-flight events are
/// empty (or its deadline passes); then the worker sends an answer that nobody reads and lets the
/// request go. The test sees that moment through a probe: a request like the cancelled one, whose
/// answer the test reads. The cancelled flush stops nothing: the queue takes the next event.
#[test]
fn cancelled_flush_leaves_waiting_list_when_worker_idles() {
    let (publisher, gate) = RecordingPublisher::gated();
    let starts = publisher.starts();
    let (delivery, lines) = bare_delivery(&publisher, fast());
    let delivery = Arc::new(delivery);
    let agentcat = AgentCat::with_sink(PROJECT, Options::default(), delivery.clone());
    push(&agentcat, job(PROJECT, "a"));
    wait_starts(&starts, 1);

    assert!(
        cancel_after_20_ms(agentcat.flush(LONG)),
        "the customer's timeout cancels the flush"
    );
    let (probe, answer) = mpsc::channel();
    assert_eq!(
        delivery.request(Kind::Flush, LONG, Reply::Blocking(probe)),
        None
    );
    push(&agentcat, job(PROJECT, "b"));
    assert_eq!(
        answer.recv_timeout(ms(100)),
        Err(mpsc::RecvTimeoutError::Timeout),
        "the request waits while the events are in flight"
    );

    gate.open();
    assert_eq!(answer.recv_timeout(WAIT), Ok(true), "the worker is idle");
    assert_eq!(
        answer.recv_timeout(WAIT),
        Err(mpsc::RecvTimeoutError::Disconnected),
        "the worker let the request go"
    );
    let mut names = publisher.resource_names();
    names.sort();
    assert_eq!(names, ["a", "b"]);
    assert_eq!(other_lines(&lines), Vec::<String>::new());
}

/// A delivery for [`PROJECT`] whose attempts wait for the returned gate, with a handle around it
/// that shares it, after its first event, `a`, started its attempt.
fn handle_with_held_attempt() -> (
    AgentCat,
    Arc<Delivery>,
    RecordingPublisher,
    queue::Gate,
    CapturedLines,
) {
    let (publisher, gate) = RecordingPublisher::gated();
    let starts = publisher.starts();
    let (delivery, lines) = bare_delivery(&publisher, fast());
    let delivery = Arc::new(delivery);
    let agentcat = AgentCat::with_sink(PROJECT, Options::default(), delivery.clone());
    push(&agentcat, job(PROJECT, "a"));
    wait_starts(&starts, 1);
    (agentcat, delivery, publisher, gate, lines)
}

/// C36, C12: a cancelled async `shutdown` still shuts the handle down. Only the cancelled
/// shutdown can mark the queue or stop the worker here: the probes are flushes.
///
/// - Its first poll marks the queue shutting down, so the next event, `b`, is dropped with TS's
///   line before any probe exists.
/// - Its request stays in the worker's waiting list. With a deadline no test reaches, the worker
///   drains the event in flight, stops once it is idle with nothing left to abandon (no `Shutting
///   down with` line), and answers the probe `true`; a later `flush` answers `true` at once.
/// - With a short deadline, the worker stops at that deadline and abandons the event in flight:
///   the probe, whose own deadline no test reaches, gets `false` while the attempt is still held,
///   which only the cancelled request's deadline can give.
#[test]
fn cancelled_shutdown_leaves_waiting_list_when_worker_idles() {
    let (agentcat, delivery, publisher, gate, lines) = handle_with_held_attempt();
    assert!(
        cancel_after_20_ms(agentcat.shutdown(LONG)),
        "the customer's timeout cancels the shutdown"
    );
    push(&agentcat, job(PROJECT, "b"));
    {
        let queue = delivery.shared.lock();
        assert!(
            queue.shutting_down,
            "the cancelled shutdown marked the queue"
        );
        assert_eq!(
            queue.pending.len() + queue.in_flight,
            1,
            "only `a` is queued: `b` was dropped"
        );
    }
    let (probe, answer) = mpsc::channel();
    assert_eq!(
        delivery.request(Kind::Flush, LONG, Reply::Blocking(probe)),
        None
    );
    assert_eq!(
        answer.recv_timeout(ms(100)),
        Err(mpsc::RecvTimeoutError::Timeout),
        "the probe waits while the event is in flight"
    );
    gate.open();
    assert_eq!(answer.recv_timeout(WAIT), Ok(true), "the worker drained");
    let started = Instant::now();
    assert!(agentcat.flush_blocking(WAIT));
    assert!(started.elapsed() < ms(500), "{:?}", started.elapsed());
    assert_eq!(publisher.resource_names(), ["a"]);
    assert_eq!(publisher.finished(), 1, "the event in flight was delivered");
    assert_eq!(
        other_lines(&lines),
        ["Queue is shutting down, event dropped"]
    );

    let (agentcat, delivery, publisher, gate, lines) = handle_with_held_attempt();
    assert!(
        cancel_after_20_ms(agentcat.shutdown(ms(200))),
        "the customer's timeout cancels the shutdown"
    );
    let (probe, answer) = mpsc::channel();
    assert_eq!(
        delivery.request(Kind::Flush, LONG, Reply::Blocking(probe)),
        None
    );
    assert_eq!(
        answer.recv_timeout(WAIT),
        Ok(false),
        "the worker stopped at the cancelled shutdown's deadline"
    );
    gate.open();
    assert!(agentcat.flush_blocking(WAIT));
    assert_eq!(publisher.finished(), 0, "the event in flight was abandoned");
    assert_eq!(other_lines(&lines), Vec::<String>::new());
}

/// C12, C36: `shutdown` is terminal, and a second call answers `true` at once in both forms and
/// logs nothing: after a first one that drained, and after a first one that timed out and
/// abandoned the event in flight.
#[test]
fn shutdown_twice_second_returns_true() {
    let no_timer = Builder::new_current_thread().build().unwrap();

    let publisher = RecordingPublisher::slow(ms(20));
    let (agentcat, lines) = handle(PROJECT, Options::default(), &publisher, None, fast());
    push(&agentcat, job(PROJECT, "drained"));
    assert!(agentcat.shutdown_blocking(WAIT));
    assert_eq!(publisher.finished(), 1);
    let started = Instant::now();
    assert!(agentcat.shutdown_blocking(WAIT));
    assert!(no_timer.block_on(agentcat.shutdown(WAIT)));
    assert!(started.elapsed() < ms(500), "{:?}", started.elapsed());
    let lines = lines.lines();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].starts_with(SUCCESS), "{lines:?}");

    let (publisher, _gate) = RecordingPublisher::gated();
    let starts = publisher.starts();
    let (agentcat, lines) = handle(PROJECT, Options::default(), &publisher, None, fast());
    push(&agentcat, job(PROJECT, "held"));
    wait_starts(&starts, 1);
    assert!(!agentcat.shutdown_blocking(ms(50)));
    let started = Instant::now();
    assert!(agentcat.shutdown_blocking(WAIT));
    assert!(no_timer.block_on(agentcat.shutdown(WAIT)));
    assert!(started.elapsed() < ms(500), "{:?}", started.elapsed());
    assert_eq!(publisher.finished(), 0);
    assert_eq!(lines.lines(), Vec::<String>::new());
}

/// C36, C42: `flush` on a handle that has taken no event and no log line answers `true` at once
/// in both forms and starts no thread: nothing was taken. The first event then starts the worker.
#[test]
fn flush_before_first_event_returns_true_without_thread() {
    let publisher = RecordingPublisher::new();
    let (delivery, lines) = bare_delivery(&publisher, fast());
    let started = Instant::now();
    assert!(delivery.flush_blocking(WAIT));
    let no_timer = Builder::new_current_thread().build().unwrap();
    assert!(no_timer.block_on(delivery.flush(WAIT)));
    assert!(started.elapsed() < ms(500), "{:?}", started.elapsed());
    assert_eq!(worker_of(&delivery), Worker::Idle);
    assert_eq!(lines.lines(), Vec::<String>::new());

    delivery.event(job(PROJECT, "first"));
    assert!(delivery.flush_blocking(WAIT));
    assert!(matches!(worker_of(&delivery), Worker::Running(_)));
    assert_eq!(publisher.resource_names(), ["first"]);
}

/// C36, C12: `shutdown` on a handle that has taken no event answers `true` at once in both forms
/// and starts no thread. It is still terminal: a later event is dropped and starts no thread
/// either, and a `flush` after it answers `true` at once.
#[test]
fn shutdown_before_first_event_returns_true_without_thread() {
    let publisher = RecordingPublisher::new();
    let (delivery, _lines) = bare_delivery(&publisher, fast());
    let started = Instant::now();
    assert!(delivery.shutdown_blocking(WAIT));
    let no_timer = Builder::new_current_thread().build().unwrap();
    assert!(no_timer.block_on(delivery.shutdown(WAIT)));
    assert_eq!(worker_of(&delivery), Worker::Idle);

    delivery.event(job(PROJECT, "after"));
    assert!(delivery.flush_blocking(WAIT));
    assert!(started.elapsed() < ms(500), "{:?}", started.elapsed());
    assert_eq!(worker_of(&delivery), Worker::Idle);
    assert!(publisher.attempts().is_empty());
}

/// C12, C42: TS logs `Queue is shutting down, event dropped` for an event after `shutdown`. On a
/// handle whose worker never started, Rust drops that line with the event, and a log line after
/// the shutdown too (`Queue::takes_lines`): a handle whose worker never starts writes nothing, and
/// no thread starts to write a line. With a worker, the line is written
/// (`shutdown_rejects_later_events`).
#[test]
fn shutting_down_line_dropped_when_worker_never_started() {
    let publisher = RecordingPublisher::new();
    let (delivery, lines) = bare_delivery(&publisher, fast());
    assert!(delivery.shutdown_blocking(WAIT));
    delivery.event(job(PROJECT, "after"));
    delivery.log("a call-path line".to_owned());
    {
        let queue = delivery.shared.lock();
        assert_eq!(queue.lines, Vec::<String>::new());
        assert!(queue.pending.is_empty());
        assert_eq!(queue.worker, Worker::Idle);
    }
    drop(delivery);
    assert_eq!(lines.lines(), Vec::<String>::new());
    assert!(publisher.attempts().is_empty());
}

/// Blocks its drop until the test lets it go, then sets its flag.
#[derive(Debug)]
struct HeldDrop {
    release: mpsc::Receiver<()>,
    dropped: Arc<AtomicBool>,
}

impl Drop for HeldDrop {
    fn drop(&mut self) {
        // A test that fails drops the sender, which lets the drop go too.
        let _ = self.release.recv();
        self.dropped.store(true, Ordering::SeqCst);
    }
}

/// A publisher whose one attempt never ends and holds a [`HeldDrop`].
#[derive(Debug)]
struct HangsHolding {
    held: Mutex<Option<HeldDrop>>,
    starts: mpsc::Sender<()>,
}

impl Publisher for HangsHolding {
    fn publish(&self, _request: PublishEventRequest) -> BoxFuture<'static, Result<(), String>> {
        let held = self.held.lock().unwrap().take();
        let starts = self.starts.clone();
        Box::pin(async move {
            let _held = held;
            let _ = starts.send(());
            pending().await
        })
    }
}

/// C36, C37: the stop of a `shutdown` at its deadline aborts the event in flight and awaits its
/// task before the worker answers, so the abandoned event's future, and what it holds (here an
/// attempt; hooks and redaction tasks alike), is gone when the caller hears `false`. The test holds
/// that future's drop: while it is held no answer comes, and the answer comes after the drop.
/// Deterministic both ways: on the worker's current-thread runtime an aborted task's future is
/// dropped only when the worker yields, so a stop that did not await would answer first.
#[test]
fn stop_awaits_aborted_tasks_before_answering() {
    let dropped = Arc::new(AtomicBool::new(false));
    let (release, released) = mpsc::channel();
    let (starts_tx, starts) = mpsc::channel();
    let publisher = HangsHolding {
        held: Mutex::new(Some(HeldDrop {
            release: released,
            dropped: Arc::clone(&dropped),
        })),
        starts: starts_tx,
    };
    let delivery = Delivery::with_writer(
        PROJECT.to_owned(),
        &Options::default(),
        Arc::new(publisher),
        None,
        fast(),
        CapturedLines::default().writer(),
    );
    delivery.event(job(PROJECT, "held"));
    wait_starts(&starts, 1);

    let (reply, answer) = mpsc::channel();
    assert_eq!(
        delivery.request(Kind::Shutdown, ms(50), Reply::Blocking(reply)),
        None
    );
    assert_eq!(
        answer.recv_timeout(ms(300)),
        Err(mpsc::RecvTimeoutError::Timeout),
        "no answer while the aborted attempt's drop is held"
    );
    assert!(!dropped.load(Ordering::SeqCst));
    release.send(()).unwrap();
    assert_eq!(answer.recv_timeout(WAIT), Ok(false));
    assert!(
        dropped.load(Ordering::SeqCst),
        "the aborted attempt was dropped before the answer"
    );
}

// ------------------------------------------------------------------------------- last drop

/// C36: `Queue::close`, the queue side of the last drop, marks the queue closed and shutting
/// down in every state of the worker. A worker that runs gets a shutdown with the drain, and the
/// caller waits for its answer only when that worker runs on another thread: on the worker thread
/// nobody can wait, so nobody gets the answer. Without a running worker nothing was taken, so
/// nothing is queued and nobody waits.
#[test]
fn close_waits_only_for_a_worker_on_another_thread() {
    let here = std::thread::current().id();
    let other = std::thread::spawn(|| std::thread::current().id())
        .join()
        .unwrap();
    let cases = [
        (Worker::Idle, None),
        (Worker::Gone, None),
        (Worker::Running(other), Some(true)),
        (Worker::Running(here), Some(false)),
    ];
    for (worker, queued) in cases {
        let mut queue = Queue::new();
        queue.worker = worker;
        let (answer, _receiver) = mpsc::channel();
        let before = Instant::now();
        let waits = queue.close(ms(300), answer);
        assert_eq!(waits, queued == Some(true), "{worker:?}");
        assert!(queue.closed && queue.shutting_down, "{worker:?}");
        match (queued, queue.requests.as_slice()) {
            (None, []) => {}
            (Some(waits), [request]) => {
                assert_eq!(request.kind, Kind::Shutdown, "{worker:?}");
                assert!(request.deadline >= before + ms(300), "{worker:?}");
                assert_eq!(
                    matches!(request.reply, Reply::Blocking(_)),
                    waits,
                    "{worker:?}"
                );
                assert_eq!(matches!(request.reply, Reply::Nobody), !waits, "{worker:?}");
            }
            (_, requests) => panic!("{worker:?}: {requests:?}"),
        }
    }
}

/// R06, C36: when the last handle drops, `Drop` drains the queue within `drop_drain`: the events
/// pushed before it are posted, and their lines written, when the drop returns.
#[test]
fn last_drop_drains_queue() {
    let publisher = RecordingPublisher::slow(ms(20));
    let (agentcat, lines) = handle(PROJECT, Options::default(), &publisher, None, fast());
    for name in ["a", "b", "c"] {
        push(&agentcat, job(PROJECT, name));
    }
    drop(agentcat);

    let mut names = publisher.resource_names();
    names.sort();
    assert_eq!(names, ["a", "b", "c"]);
    assert_eq!(publisher.finished(), 3);
    assert_eq!(lines.lines().len(), 3);
}

/// Releases a hook's sender when the hook's future is dropped, finished or aborted.
struct OnDrop(Option<oneshot::Sender<()>>);

impl Drop for OnDrop {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}

/// Design §13, review focus 1: on a current-thread customer runtime, the last handle drops inside
/// `block_on` while the call's `identify` hook still needs that runtime. The hook cannot run while
/// `Drop` blocks the only thread, so the drain waits for `drop_drain` (300 ms) and returns in under
/// a second; the abandoned event is not posted, and its hook is aborted, not left to finish.
#[test]
fn last_drop_on_current_thread_runtime_returns_within_budget() {
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    let publisher = RecordingPublisher::new();
    let completed = Arc::new(AtomicBool::new(false));
    let (release, released) = oneshot::channel::<()>();
    let (dropped_tx, dropped) = oneshot::channel::<()>();
    let hook_state = Arc::new(Mutex::new(Some((released, dropped_tx))));
    let done = Arc::clone(&completed);
    let options = Options::default().identify(move |_call| {
        let state = hook_state.lock().unwrap().take();
        let done = Arc::clone(&done);
        async move {
            let (released, dropped) = state.expect("one call");
            let _on_drop = OnDrop(Some(dropped));
            let _ = released.await;
            done.store(true, Ordering::SeqCst);
            Ok(Some(UserIdentity::new("user-1")))
        }
    });
    let config = DeliveryConfig {
        hook_budget: Duration::from_secs(10),
        drop_drain: ms(300),
        ..fast()
    };

    let recording = publisher.clone();
    let elapsed = runtime.block_on(async move {
        let (agentcat, _lines) = handle(PROJECT, options, &recording, None, config);
        let (server_end, client_end) = tokio::io::duplex(64 * 1024);
        let client = InitializeRequestParams::new(
            ClientCapabilities::default(),
            Implementation::new("current-thread", "1.0.0"),
        )
        .with_protocol_version(ProtocolVersion::V_2025_11_25);
        let (server, client) = tokio::join!(
            agentcat.track(Sample).serve(server_end),
            client.serve(client_end)
        );
        let (server, client) = (server.unwrap(), client.unwrap());
        let arguments = object(json!({"msg": "hi"}));
        client
            .call_tool(CallToolRequestParams::new("echo").with_arguments(arguments))
            .await
            .unwrap();
        client.cancel().await.unwrap();
        // The server's `Tracked` holds a clone of the handle; it goes with the server.
        server.cancel().await.unwrap();
        release.send(()).unwrap();

        let started = Instant::now();
        drop(agentcat);
        let elapsed = started.elapsed();
        tokio::time::timeout(WAIT, dropped)
            .await
            .expect("the hook's future is dropped")
            .unwrap();
        elapsed
    });
    assert!(
        ms(300) <= elapsed && elapsed < Duration::from_secs(1),
        "{elapsed:?}"
    );
    assert!(
        !completed.load(Ordering::SeqCst),
        "the abandoned event's hook is aborted"
    );
    assert!(publisher.attempts().is_empty());
}

/// A publisher whose attempt, once released, takes the last handle out of its slot and drops it
/// on the worker thread, and reports how long that drop took and on which thread.
#[derive(Debug)]
struct DropsLastHandle {
    slot: Arc<Mutex<Option<AgentCat>>>,
    release: Arc<Semaphore>,
    report: mpsc::Sender<(Duration, Option<String>)>,
    /// Fires when the worker drops its publisher, as it exits.
    _exit: DropSignal,
}

impl Publisher for DropsLastHandle {
    fn publish(&self, _request: PublishEventRequest) -> BoxFuture<'static, Result<(), String>> {
        let slot = Arc::clone(&self.slot);
        let release = Arc::clone(&self.release);
        let report = self.report.clone();
        Box::pin(async move {
            // The release opens by closing: `acquire` then fails at once.
            let _ = release.acquire().await;
            let last = slot.lock().unwrap().take();
            let started = Instant::now();
            drop(last);
            let thread = std::thread::current().name().map(str::to_owned);
            let _ = report.send((started.elapsed(), thread));
            Ok(())
        })
    }
}

/// C36: when the last handle drops on the worker thread itself (here inside an attempt, as a
/// publisher or a hook output holding the last clone can), `Drop` does not wait for the drain,
/// which would be a wait for its own thread; the worker drains and exits on its own.
#[test]
fn drop_on_worker_thread_does_not_wait() {
    let slot = Arc::new(Mutex::new(None));
    let release = Arc::new(Semaphore::new(0));
    let (report, reports) = mpsc::channel();
    let (exit, exited) = DropSignal::new();
    let publisher = DropsLastHandle {
        slot: Arc::clone(&slot),
        release: Arc::clone(&release),
        report,
        _exit: exit,
    };
    let delivery = Delivery::with_writer(
        PROJECT.to_owned(),
        &Options::default(),
        Arc::new(publisher),
        None,
        fast(),
        CapturedLines::default().writer(),
    );
    let agentcat = AgentCat::with_sink(PROJECT, Options::default(), Arc::new(delivery));
    push(&agentcat, job(PROJECT, "a"));
    *slot.lock().unwrap() = Some(agentcat);
    release.close();

    let (elapsed, thread) = reports
        .recv_timeout(WAIT)
        .expect("the attempt drops the last handle");
    assert_eq!(thread.as_deref(), Some(WORKER_THREAD));
    assert!(
        elapsed < ms(200),
        "{elapsed:?}: a drain wait takes drop_drain (1 s)"
    );
    exited
        .recv_timeout(WAIT)
        .expect("the worker exits and drops its publisher");
}

// ---------------------------------------------------------------------------------- logging

/// C42, B-117: a call-path line, the warning of an empty tool listing, reaches the file of the
/// process logger in a temp home through the worker. The first `logging::configure` of a process
/// wins, so this test relies on nextest's process per test (ruling P4).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn log_lines_are_written_by_the_worker() {
    let home = std::env::temp_dir().join(format!("agentcat-delivery-log-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    logging::configure(LogDestination::File, Some(home.clone()));

    let publisher: Arc<dyn Publisher> = Arc::new(RecordingPublisher::new());
    let agentcat = AgentCat::with_delivery(PROJECT, Options::default(), publisher, fast());
    let client = connect(agentcat.track(Empty), "delivery-client").await;
    client.list_all_tools().await.unwrap();
    client.cancel().await.unwrap();
    assert!(agentcat.flush(WAIT).await);

    let text = std::fs::read_to_string(home.join("agentcat.log")).unwrap();
    let warning = "Warning: No tools found in the original list. This is likely due to the tools not being registered before AgentCat.track().";
    let expected = format!("] {} {warning}", logging::prefix());
    assert!(text.lines().any(|line| line.ends_with(&expected)), "{text}");
    let _ = std::fs::remove_dir_all(&home);
}

/// Task 11 (R22): the workspace turns neither `arbitrary_precision` nor `float_roundtrip` on, so
/// the self-check finds nothing and the worker writes no warning.
#[test]
fn json_features_empty_in_this_build() {
    assert_eq!(super::worker::json_features(), Vec::<&str>::new());
}

/// The self-check line names the feature, with the text the plan fixes (R22).
#[test]
fn feature_warning_has_the_fixed_text() {
    assert_eq!(
        super::worker::feature_warning("float_roundtrip"),
        "Warning: serde_json feature \"float_roundtrip\" is enabled in this build; AgentCat token estimates and size checks may differ from the TypeScript SDK"
    );
}

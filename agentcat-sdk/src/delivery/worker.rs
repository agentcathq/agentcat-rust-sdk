//! The worker of a handle: one OS thread, `agentcat-delivery`, with a
//! current-thread tokio runtime that has the time driver. Its loop writes the queued log lines,
//! takes pending jobs while fewer than `concurrency` run, answers `flush` and `shutdown` and owns
//! their deadlines. Each job runs as a task on the worker's runtime and holds its slot through
//! `process` and every attempt ([`run_job`]).
use std::mem;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::Instant;

use serde_json::Value;
use tokio::runtime::Builder;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use super::queue::{Job, Kind, Queue, Request, Worker};
use super::request::publish_request;
use super::{DeliveryConfig, LineWriter, Observer, Publisher};
use crate::event::assemble::from_call;
use crate::pipeline::EventObject;
use crate::pipeline::process::{Redactors, process};
use crate::wire;

/// The name of the worker thread.
pub(super) const THREAD_NAME: &str = "agentcat-delivery";

/// The line after the fourth failed attempt of an event, before the error.
const DELIVERY_FAILED: &str = "Failed to deliver AgentCat event after retries: ";

/// The line when the worker's thread or its runtime cannot start, before the error.
const START_FAILED: &str = "Failed to start the AgentCat delivery thread: ";

/// What a handle's [`Delivery`](super::Delivery) and its worker share.
pub(super) struct Shared {
    /// The project of the handle: the `projectId` of its tool-call events.
    pub(super) project_id: String,
    /// The customer's redaction hooks.
    pub(super) redactors: Redactors,
    /// Makes one attempt to post an event.
    pub(super) publisher: Arc<dyn Publisher>,
    /// Told about each event that finishes the pipeline.
    pub(super) observer: Option<Arc<dyn Observer>>,
    /// The limits and budgets.
    pub(super) config: DeliveryConfig,
    /// Writes one log line.
    pub(super) write: LineWriter,
    /// What the call path and the worker share.
    queue: Mutex<Queue>,
    /// Wakes the worker's loop.
    wake: Notify,
}

impl Shared {
    /// The state of a handle whose worker has not started.
    pub(super) fn new(
        project_id: String,
        redactors: Redactors,
        publisher: Arc<dyn Publisher>,
        observer: Option<Arc<dyn Observer>>,
        config: DeliveryConfig,
        write: LineWriter,
    ) -> Self {
        Self {
            project_id,
            redactors,
            publisher,
            observer,
            config,
            write,
            queue: Mutex::new(Queue::new()),
            wake: Notify::new(),
        }
    }

    /// The queue; a poisoned lock is taken over.
    pub(super) fn lock(&self) -> MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Wakes the worker's loop; a wake while the loop runs is kept for its next wait.
    pub(super) fn wake(&self) {
        self.wake.notify_one();
    }

    /// Writes `line` through the handle's writer.
    fn write(&self, line: &str) {
        (self.write)(line);
    }

    /// Marks the worker gone, drops the jobs it held and answers its requests `false`.
    fn gone(&self) {
        let (jobs, requests) = self.lock().take_all();
        drop(jobs);
        for request in requests {
            request.reply.send(false);
        }
    }
}

/// Spawns the worker thread of `shared`, whose state a push set to starting. When the thread
/// cannot be spawned, one line says so, written on the calling thread (the one line not written
/// by a worker, as there is none), and the worker is gone: what it held is dropped and later jobs
/// are dropped.
pub(super) fn start(shared: &Arc<Shared>) {
    let worker = Arc::clone(shared);
    let spawned = thread::Builder::new()
        .name(THREAD_NAME.to_owned())
        .spawn(move || run_thread(&worker));
    match spawned {
        Ok(thread) => {
            let id = thread.thread().id();
            let mut queue = shared.lock();
            // The thread may already have failed and marked itself gone.
            if queue.worker == Worker::Starting {
                queue.worker = Worker::Running(id);
            }
        }
        Err(error) => {
            shared.write(&format!("{START_FAILED}{error}"));
            shared.gone();
        }
    }
}

/// Marks the worker gone when its thread ends, also by a panic.
struct Exit<'a>(&'a Shared);

impl Drop for Exit<'_> {
    fn drop(&mut self) {
        self.0.gone();
    }
}

/// The body of the worker thread: the loop on a current-thread runtime with the time and IO
/// drivers (reqwest needs the IO driver). When the runtime cannot be built, one line says so and
/// the worker is gone.
fn run_thread(shared: &Arc<Shared>) {
    let _exit = Exit(shared);
    for feature in json_features() {
        shared.write(&feature_warning(feature));
    }
    let runtime = match Builder::new_current_thread()
        .enable_time()
        .enable_io()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            shared.write(&format!("{START_FAILED}{error}"));
            return;
        }
    };
    runtime.block_on(run(shared));
    // The runtime drops here, before `_exit`, with any task left on it.
}

/// The worker's loop. Each turn it writes the queued lines, starts pending jobs while
/// fewer than `concurrency` run, and answers the requests:
///
/// - A flush gets `true` once nothing is pending or in flight, `false` at its deadline.
/// - A shutdown, once nothing is pending or in flight or at its deadline, stops the worker
///   ([`stop`]) and answers every waiting request whether the queue had emptied.
/// - After the stop, every request gets `true`: nothing is left. The worker then only writes log
///   lines, and it exits once the last handle dropped.
///
/// Between turns it waits for a wake (a push, a line, a request, a job's end) or the next
/// deadline. A request whose caller stopped waiting (a cancelled async `flush` or `shutdown`)
/// stays in the waiting list like any other until it is answered.
async fn run(shared: &Arc<Shared>) {
    let mut tasks: Vec<JoinHandle<()>> = Vec::new();
    let mut waiting: Vec<Request> = Vec::new();
    let mut stopped = false;
    loop {
        let (lines, jobs, idle, closed) = {
            let mut queue = shared.lock();
            waiting.append(&mut queue.requests);
            let mut jobs = Vec::new();
            while !stopped && queue.in_flight < shared.config.concurrency {
                let Some(job) = queue.pending.pop_front() else {
                    break;
                };
                queue.in_flight += 1;
                jobs.push(job);
            }
            let idle = queue.pending.is_empty() && queue.in_flight == 0;
            (mem::take(&mut queue.lines), jobs, idle, queue.closed)
        };
        for line in &lines {
            shared.write(line);
        }
        tasks.retain(|task| !task.is_finished());
        for job in jobs {
            let slot = Slot(Arc::clone(shared));
            tasks.push(tokio::spawn(run_job(Arc::clone(shared), job, slot)));
        }

        let now = Instant::now();
        let shutdown_due = waiting
            .iter()
            .any(|request| request.kind == Kind::Shutdown && (idle || request.deadline <= now));
        if !stopped && shutdown_due {
            stop(shared, &mut tasks).await;
            stopped = true;
            for request in waiting.drain(..) {
                request.reply.send(idle);
            }
        }
        if stopped {
            for request in waiting.drain(..) {
                request.reply.send(true);
            }
            if closed {
                return;
            }
        } else {
            let (due, open): (Vec<Request>, Vec<Request>) =
                waiting.drain(..).partition(|request| {
                    request.kind == Kind::Flush && (idle || request.deadline <= now)
                });
            waiting = open;
            for request in due {
                request.reply.send(idle);
            }
        }

        let woken = shared.wake.notified();
        match waiting.iter().map(|request| request.deadline).min() {
            Some(deadline) => {
                let _ = tokio::time::timeout_at(deadline.into(), woken).await;
            }
            None => woken.await,
        }
    }
}

/// Stops the worker for good: drops the pending jobs, after the line `Shutting down with <n>
/// events still in queue` when there are any (`<n>` counts the pending queue only), and
/// aborts the jobs in flight. It returns once each aborted job is dropped, so the hooks and the
/// redaction tasks of the abandoned events are aborted too before any caller hears the answer.
async fn stop(shared: &Shared, tasks: &mut Vec<JoinHandle<()>>) {
    let abandoned = mem::take(&mut shared.lock().pending);
    if !abandoned.is_empty() {
        shared.write(&format!(
            "Shutting down with {} events still in queue",
            abandoned.len()
        ));
    }
    drop(abandoned);
    for task in tasks.iter() {
        task.abort();
    }
    for task in tasks.drain(..) {
        // An aborted task's handle resolves after its future is dropped.
        let _ = task.await;
    }
}

/// A job's slot: dropping it, when the job ends or is aborted, frees the slot and wakes the loop.
struct Slot(Arc<Shared>);

impl Drop for Slot {
    fn drop(&mut self) {
        let mut queue = self.0.lock();
        queue.in_flight = queue.in_flight.saturating_sub(1);
        drop(queue);
        self.0.wake();
    }
}

/// One job, holding `_slot` until it ends: a tool call is assembled into its event
/// ([`from_call`]), the event goes through [`process`] (whose lines are written when it ends),
/// the observer is told about the finished event, and an event with a project is posted with
/// retries ([`post`]); a telemetry-only event (`projectId` `""`) is never posted.
async fn run_job(shared: Arc<Shared>, job: Job, _slot: Slot) {
    let job = match job {
        Job::Call(record) => from_call(*record, &shared.project_id),
        Job::Event(job) => job,
    };
    let mut log = Vec::new();
    let event = process(job, &shared.redactors, shared.config.hook_budget, &mut log).await;
    for line in &log {
        shared.write(line);
    }
    let Some(event) = event else {
        return;
    };
    if let Some(observer) = &shared.observer {
        observer.finished(&event);
    }
    let has_project = event
        .get("projectId")
        .and_then(Value::as_str)
        .is_some_and(|project| !project.is_empty());
    if has_project {
        post(&shared, &event).await;
    }
}

/// Posts `event`: up to 4 attempts with one request, the same
/// `evt_` ID each time. After each failure `Failed to send event <id>, retrying... [Error:
/// <text>]`, then the next of `retry_delays`; after the fourth `Failed to deliver AgentCat event
/// after retries: <text>`; on success the metadata line ([`success_line`]). The publisher is
/// prepared first ([`Publisher::prepare`]), with its lines written; one that can never send gets
/// no attempt, only the final line.
async fn post(shared: &Shared, event: &EventObject) {
    let mut log = Vec::new();
    let prepared = shared.publisher.prepare(&mut log);
    for line in &log {
        shared.write(line);
    }
    if let Err(reason) = prepared {
        shared.write(&format!("{DELIVERY_FAILED}{reason}"));
        return;
    }
    let request = publish_request(event);
    let id = wire::coerce_to_string(event.get("id"));
    let mut delays = shared.config.retry_delays.iter();
    loop {
        match shared.publisher.publish(request.clone()).await {
            Ok(()) => {
                shared.write(&success_line(event));
                return;
            }
            Err(error) => {
                shared.write(&format!(
                    "Failed to send event {id}, retrying... [Error: {error}]"
                ));
                let Some(delay) = delays.next() else {
                    shared.write(&format!("{DELIVERY_FAILED}{error}"));
                    return;
                };
                tokio::time::sleep(*delay).await;
            }
        }
    }
}

/// The line after a successful attempt, the event's metadata and never its payload: `Successfully
/// sent event <id> | <eventType> | session <sessionId> | <projectId> | <duration> ms | <actor>`,
/// each field in its string form ([`wire::coerce_to_string`], `undefined` for an absent one), the
/// actor the `identifyActorGivenId` when it is truthy ([`wire::truthy`]), else `anonymous`.
fn success_line(event: &EventObject) -> String {
    let field = |key: &str| wire::coerce_to_string(event.get(key));
    let actor = match event.get("identifyActorGivenId") {
        Some(actor) if wire::truthy(actor) => wire::coerce_to_string(Some(actor)),
        _ => "anonymous".to_owned(),
    };
    format!(
        "Successfully sent event {} | {} | session {} | {} | {} ms | {actor}",
        field("id"),
        field("eventType"),
        field("sessionId"),
        field("projectId"),
        field("duration"),
    )
}

/// The `serde_json` features a customer build unified on that change the SDK's JSON, found
/// at run time by parsing a probe: `arbitrary_precision` lets a `Number` read `1e400`;
/// `float_roundtrip` makes `f64` parsing exact, where the default parser rounds
/// `9007199254740993.0`, `0.9999999999999999` and `7.038531e-26` to a neighbor.
pub(super) fn json_features() -> Vec<&'static str> {
    const ROUNDTRIP_PROBES: [&str; 3] =
        ["9007199254740993.0", "0.9999999999999999", "7.038531e-26"];
    let mut features = Vec::new();
    if "1e400".parse::<serde_json::Number>().is_ok() {
        features.push("arbitrary_precision");
    }
    if ROUNDTRIP_PROBES
        .iter()
        .all(|text| serde_json::from_str::<f64>(text).ok() == text.parse::<f64>().ok())
    {
        features.push("float_roundtrip");
    }
    features
}

/// The log line for a unified `serde_json` `feature`.
pub(super) fn feature_warning(feature: &str) -> String {
    format!(
        "Warning: serde_json feature \"{feature}\" is enabled in this build; AgentCat token \
         estimates and size checks may differ from the TypeScript SDK"
    )
}

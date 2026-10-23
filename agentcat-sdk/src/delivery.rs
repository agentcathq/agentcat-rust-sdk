//! The delivery of finished events to the `AgentCat` API (design §6.7, C36).
//!
//! [`Delivery`] is the [`Sink`] of a handle that publishes. The call path hands it each tool-call
//! record, each assembled event and each log line, and it only appends them to the [`queue`]
//! under a short lock and wakes the worker: it never blocks and never waits (design §7.3). The
//! [`worker`] is one OS thread per handle, `agentcat-delivery`, started by the first push or log
//! line, with a current-thread tokio runtime that has the time driver. It writes the log lines
//! (C42), runs at most `concurrency` events at once through the pipeline and posts each one
//! through the [`Publisher`] with TS's retries, and it owns every deadline. [`wire`] builds the
//! request of one event.
//!
//! The lifecycle (C36, C12, R06):
//!
//! - `flush` and `shutdown` send their deadline to the worker, which answers `true` once the queue
//!   and the in-flight events are empty and `false` when the deadline comes first. The async forms
//!   await a `oneshot` answer, with no timer on the caller's runtime; the blocking twins wait on a
//!   `std::sync::mpsc` channel for the timeout and 100 ms more. Both give `false` when the worker
//!   is gone, and `true` at once when it never started (nothing was taken).
//! - `shutdown` is terminal: from the call on, a push is dropped with TS's `Queue is shutting
//!   down, event dropped`. Once the queue is empty or the deadline passed, the worker abandons what
//!   is left (after `Shutting down with <n> events still in queue` when events are pending) and
//!   aborts the events in flight with their hooks; from then on it only writes log lines.
//! - The last drop of the handle runs that shutdown with `drop_drain` (5 s) and waits for the
//!   worker's answer on a `std::sync::mpsc` channel, for at most `drop_drain` and 100 ms more, so it
//!   never waits on the caller's runtime; the worker then exits. A drop on the worker thread itself
//!   does not wait. `std::process::exit` and `panic = "abort"` skip the drain.
use std::fmt;
use std::future::ready;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use agentcat_api::models::PublishEventRequest;
use tokio::sync::oneshot;

use crate::config::Options;
use crate::event::CallRecord;
use crate::event::assemble::EventJob;
use crate::logging;
use crate::pipeline::TsEvent;
use crate::pipeline::process::Redactors;
use crate::sink::{BoxFuture, Sink};
use queue::{Job, Kind, Reply, Request, Worker};
use worker::Shared;

mod queue;
pub(crate) mod wire;
mod worker;

#[cfg(test)]
mod tests;

/// How much longer than its timeout a blocking caller waits for the worker's answer, which the
/// worker sends at the deadline.
const ANSWER_GRACE: Duration = Duration::from_millis(100);

/// Makes one attempt to post one event.
pub(crate) trait Publisher: Send + Sync + fmt::Debug + 'static {
    /// Posts `request` once: `Err` holds the text of the failure for TS's log lines.
    fn publish(&self, request: PublishEventRequest) -> BoxFuture<'static, Result<(), String>>;
}

/// Told about each event that finishes the pipeline, telemetry-only events included (the test
/// fake's record, C43).
pub(crate) trait Observer: Send + Sync + fmt::Debug + 'static {
    /// Takes the finished event, before it is posted.
    fn finished(&self, event: &TsEvent);
}

/// Writes one log line (the process logger, or a test's capture).
pub(crate) type LineWriter = Arc<dyn Fn(&str) + Send + Sync>;

/// The limits and budgets of a handle's delivery. [`Default`] gives TS's queue values (B-100 to
/// B-102), TS's hook budget (C37) and the 5 s drain of the last drop (R06); tests shrink them.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DeliveryConfig {
    /// The most jobs that wait; a push onto that many drops the oldest (TS: 10 000).
    pub(crate) max_pending: usize,
    /// The most jobs in flight at once (TS: 5).
    pub(crate) concurrency: usize,
    /// The waits before the second, third and fourth attempts (TS: 1, 2 and 4 s).
    pub(crate) retry_delays: [Duration; 3],
    /// How long each call hook may take to settle (TS: 30 s).
    pub(crate) hook_budget: Duration,
    /// How long the drain on the last drop may take (5 s).
    pub(crate) drop_drain: Duration,
}

impl Default for DeliveryConfig {
    fn default() -> Self {
        Self {
            max_pending: 10_000,
            concurrency: 5,
            retry_delays: [
                Duration::from_secs(1),
                Duration::from_secs(2),
                Duration::from_secs(4),
            ],
            hook_budget: Duration::from_secs(30),
            drop_drain: Duration::from_secs(5),
        }
    }
}

/// The delivery queue of one handle and its worker (see the module docs). Its `Debug` output
/// shows the project ID and the configuration only.
pub(crate) struct Delivery {
    shared: Arc<Shared>,
}

impl fmt::Debug for Delivery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Delivery")
            .field("project_id", &self.shared.project_id)
            .field("config", &self.shared.config)
            .finish_non_exhaustive()
    }
}

impl Delivery {
    /// The delivery of the handle for `project_id`, with the redaction hooks of `options`, that
    /// posts through `publisher`, tells `observer` about each finished event and writes its lines
    /// through the process logger ([`logging::write`]). It starts nothing.
    pub(crate) fn new(
        project_id: String,
        options: &Options,
        publisher: Arc<dyn Publisher>,
        observer: Option<Arc<dyn Observer>>,
        config: DeliveryConfig,
    ) -> Self {
        Self::with_writer(
            project_id,
            options,
            publisher,
            observer,
            config,
            Arc::new(logging::write),
        )
    }

    /// [`Delivery::new`] that writes its lines through `write`.
    pub(crate) fn with_writer(
        project_id: String,
        options: &Options,
        publisher: Arc<dyn Publisher>,
        observer: Option<Arc<dyn Observer>>,
        config: DeliveryConfig,
        write: LineWriter,
    ) -> Self {
        let redactors = Redactors {
            string: options.string_redactor().cloned(),
            event: options.event_redactor().cloned(),
        };
        Self {
            shared: Arc::new(Shared::new(
                project_id, redactors, publisher, observer, config, write,
            )),
        }
    }

    /// Queues `job` (B-100, C12): see [`Queue::push_job`](queue::Queue::push_job). The job it
    /// drops is dropped after the lock is let go, which aborts its hooks.
    fn push(&self, job: Job) {
        let (dropped, start) = {
            let mut queue = self.shared.lock();
            let dropped = queue.push_job(job, self.shared.config.max_pending);
            (dropped, queue.claim_start())
        };
        drop(dropped);
        self.wake(start);
    }

    /// Starts the worker when this caller claimed it, then wakes it.
    fn wake(&self, start: bool) {
        if start {
            worker::start(&self.shared);
        }
        self.shared.wake();
    }

    /// Hands a request of `kind` with `timeout` and `reply` to the worker; `Some` is the answer
    /// when it comes at once (see [`Queue::push_request`](queue::Queue::push_request)).
    fn request(&self, kind: Kind, timeout: Duration, reply: Reply) -> Option<bool> {
        let answer = self
            .shared
            .lock()
            .push_request(Request::new(kind, timeout, reply));
        if answer.is_none() {
            self.shared.wake();
        }
        answer
    }

    /// The async form of a request: the worker's answer, `false` when the worker goes away first.
    fn answer(&self, kind: Kind, timeout: Duration) -> BoxFuture<'static, bool> {
        let (sender, receiver) = oneshot::channel();
        match self.request(kind, timeout, Reply::Async(sender)) {
            Some(answer) => Box::pin(ready(answer)),
            None => Box::pin(async move { receiver.await.unwrap_or(false) }),
        }
    }

    /// The blocking form of a request: waits for the worker's answer for `timeout` and
    /// [`ANSWER_GRACE`] more; `false` when it does not come.
    fn answer_blocking(&self, kind: Kind, timeout: Duration) -> bool {
        let (sender, receiver) = mpsc::channel();
        match self.request(kind, timeout, Reply::Blocking(sender)) {
            Some(answer) => answer,
            None => receiver
                .recv_timeout(timeout.saturating_add(ANSWER_GRACE))
                .unwrap_or(false),
        }
    }
}

impl Sink for Delivery {
    /// Queues `message` for the worker, which writes it (C42); the first line starts the worker.
    fn log(&self, message: String) {
        let start = {
            let mut queue = self.shared.lock();
            queue.push_line(message);
            queue.claim_start()
        };
        self.wake(start);
    }

    fn record(&self, record: CallRecord) {
        self.push(Job::Call(Box::new(record)));
    }

    fn event(&self, job: EventJob) {
        self.push(Job::from(job));
    }

    fn flush(&self, timeout: Duration) -> BoxFuture<'static, bool> {
        self.answer(Kind::Flush, timeout)
    }

    fn shutdown(&self, timeout: Duration) -> BoxFuture<'static, bool> {
        self.answer(Kind::Shutdown, timeout)
    }

    fn flush_blocking(&self, timeout: Duration) -> bool {
        self.answer_blocking(Kind::Flush, timeout)
    }

    fn shutdown_blocking(&self, timeout: Duration) -> bool {
        self.answer_blocking(Kind::Shutdown, timeout)
    }
}

impl Drop for Delivery {
    /// The drain of the last drop (C36): a shutdown with `drop_drain` that also tells the worker
    /// to exit once it stopped. Off the worker thread it waits for the answer for at most
    /// `drop_drain` and 100 ms more; on the worker thread it does not wait (the answer would need
    /// the thread it blocks). Without a running worker there is nothing to drain.
    fn drop(&mut self) {
        let drain = self.shared.config.drop_drain;
        let (sender, receiver) = mpsc::channel();
        let wait = {
            let mut queue = self.shared.lock();
            if matches!(queue.worker, Worker::Idle | Worker::Gone) {
                return;
            }
            let on_worker = queue.worker == Worker::Running(thread::current().id());
            queue.closed = true;
            let reply = if on_worker {
                Reply::Nobody
            } else {
                Reply::Blocking(sender)
            };
            queue.push_request(Request::new(Kind::Shutdown, drain, reply));
            !on_worker
        };
        self.shared.wake();
        if wait {
            let _ = receiver.recv_timeout(drain.saturating_add(ANSWER_GRACE));
        }
    }
}

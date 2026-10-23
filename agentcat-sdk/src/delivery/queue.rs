//! The queue between the call path and the worker (B-100, C12, C36): the pending jobs, the log
//! lines, the `flush` and `shutdown` requests and the worker's state, behind one `Mutex` that the
//! call path holds only to append. Nothing here blocks or waits.
use std::collections::VecDeque;
use std::mem;
use std::sync::mpsc;
use std::thread::ThreadId;
use std::time::{Duration, Instant};

use tokio::sync::oneshot;

use crate::event::CallRecord;
use crate::event::assemble::EventJob;

/// TS's line when a push finds `max_pending` jobs pending (B-100).
const QUEUE_FULL: &str = "Event queue full, dropping oldest event";

/// TS's line for a push after `shutdown` (C12).
const SHUTTING_DOWN: &str = "Queue is shutting down, event dropped";

/// How far ahead a deadline goes when the timeout is past what [`Instant`] can hold: about a
/// century, which is forever in practice.
const FOREVER: Duration = Duration::from_secs(100 * 365 * 24 * 60 * 60);

/// One unit of work for the worker.
#[derive(Debug)]
pub(super) enum Job {
    /// A tool call, assembled into its event on the worker (`from_call`); boxed, so a queue
    /// slot is the size of the smaller job.
    Call(Box<CallRecord>),
    /// An event assembled by the caller, such as a custom event.
    Event(EventJob),
}

impl From<EventJob> for Job {
    fn from(job: EventJob) -> Self {
        Self::Event(job)
    }
}

/// What a request asks the worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    /// Answer once the queue and the in-flight events are empty.
    Flush,
    /// The same, then stop for good (C12).
    Shutdown,
}

/// Where the worker sends the answer to a request.
#[derive(Debug)]
pub(super) enum Reply {
    /// An async caller awaits it.
    Async(oneshot::Sender<bool>),
    /// A blocked caller waits for it.
    Blocking(mpsc::Sender<bool>),
    /// Nobody waits: the last handle dropped on the worker thread.
    Nobody,
}

impl Reply {
    /// Sends `answer`; a caller that stopped waiting is ignored.
    pub(super) fn send(self, answer: bool) {
        match self {
            Self::Async(sender) => {
                let _ = sender.send(answer);
            }
            Self::Blocking(sender) => {
                let _ = sender.send(answer);
            }
            Self::Nobody => {}
        }
    }
}

/// A `flush` or `shutdown` for the worker, with its deadline.
#[derive(Debug)]
pub(super) struct Request {
    /// What it asks.
    pub(super) kind: Kind,
    /// When the worker answers `false` if the queue has not emptied.
    pub(super) deadline: Instant,
    /// Where the answer goes.
    pub(super) reply: Reply,
}

impl Request {
    /// A request of `kind` whose deadline is `timeout` from now (or far ahead when `timeout` is
    /// past what [`Instant`] can hold).
    pub(super) fn new(kind: Kind, timeout: Duration, reply: Reply) -> Self {
        let now = Instant::now();
        let deadline = now
            .checked_add(timeout)
            .or_else(|| now.checked_add(FOREVER))
            .unwrap_or(now);
        Self {
            kind,
            deadline,
            reply,
        }
    }
}

/// The state of a handle's worker thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Worker {
    /// Not started: nothing was pushed or logged yet.
    Idle,
    /// A push is spawning it.
    Starting,
    /// Running on the thread with this ID.
    Running(ThreadId),
    /// Gone: its spawn or its runtime failed, or it exited.
    Gone,
}

/// What the call path and the worker share (see the module docs).
#[derive(Debug)]
pub(super) struct Queue {
    /// The jobs no slot has taken yet, oldest first.
    pub(super) pending: VecDeque<Job>,
    /// The log lines the worker has not written yet; they are not jobs and do not count toward
    /// `max_pending`.
    pub(super) lines: Vec<String>,
    /// The requests the worker has not taken yet.
    pub(super) requests: Vec<Request>,
    /// The jobs the worker runs now.
    pub(super) in_flight: usize,
    /// Set by `shutdown` and by the last drop: later jobs are dropped (C12).
    pub(super) shutting_down: bool,
    /// Set by the last drop: the worker exits once it stopped.
    pub(super) closed: bool,
    /// The worker thread.
    pub(super) worker: Worker,
}

impl Queue {
    /// An empty queue whose worker has not started.
    pub(super) fn new() -> Self {
        Self {
            pending: VecDeque::new(),
            lines: Vec::new(),
            requests: Vec::new(),
            in_flight: 0,
            shutting_down: false,
            closed: false,
            worker: Worker::Idle,
        }
    }

    /// Whether a log line has a worker to write it: one that runs or is starting, or an idle one
    /// that a line may still start (not after `shutdown`).
    fn takes_lines(&self) -> bool {
        match self.worker {
            Worker::Idle => !self.shutting_down,
            Worker::Starting | Worker::Running(_) => true,
            Worker::Gone => false,
        }
    }

    /// Queues `line` for the worker; a line no worker will write is dropped.
    pub(super) fn push_line(&mut self, line: String) {
        if self.takes_lines() {
            self.lines.push(line);
        }
    }

    /// Queues `job` and gives back the job it drops, which the caller drops after it lets go of
    /// the lock:
    ///
    /// - After `shutdown`, `job` itself, with TS's `Queue is shutting down, event dropped` (C12).
    /// - With no worker left, `job` itself.
    /// - Onto `max_pending` pending jobs, the oldest pending one, with TS's `Event queue full,
    ///   dropping oldest event` (B-100). As in TS, the line comes whenever the pending count is at
    ///   the limit, so a limit of 0 keeps one job.
    pub(super) fn push_job(&mut self, job: Job, max_pending: usize) -> Option<Job> {
        if self.shutting_down {
            self.push_line(SHUTTING_DOWN.to_owned());
            return Some(job);
        }
        if self.worker == Worker::Gone {
            return Some(job);
        }
        let dropped = if self.pending.len() >= max_pending {
            self.push_line(QUEUE_FULL.to_owned());
            self.pending.pop_front()
        } else {
            None
        };
        self.pending.push_back(job);
        dropped
    }

    /// Marks an idle worker as starting when it has work; `true` when the caller must spawn it.
    pub(super) fn claim_start(&mut self) -> bool {
        let has_work = !self.pending.is_empty() || !self.lines.is_empty();
        if self.worker == Worker::Idle && has_work {
            self.worker = Worker::Starting;
            return true;
        }
        false
    }

    /// Hands `request` to the worker, or gives its answer now: `true` when the worker never
    /// started (nothing was taken), `false` when it is gone. A shutdown first marks the queue
    /// shutting down, so later jobs are dropped (C12).
    pub(super) fn push_request(&mut self, request: Request) -> Option<bool> {
        if request.kind == Kind::Shutdown {
            self.shutting_down = true;
        }
        match self.worker {
            Worker::Idle => Some(true),
            Worker::Gone => Some(false),
            Worker::Starting | Worker::Running(_) => {
                self.requests.push(request);
                None
            }
        }
    }

    /// Marks the worker gone and takes what it would have handled: the pending jobs and the
    /// requests, which the caller drops and answers `false` after it lets go of the lock. The
    /// lines are dropped.
    pub(super) fn take_all(&mut self) -> (VecDeque<Job>, Vec<Request>) {
        self.worker = Worker::Gone;
        self.lines.clear();
        (mem::take(&mut self.pending), mem::take(&mut self.requests))
    }
}

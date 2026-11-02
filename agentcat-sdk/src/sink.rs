//! [`Sink`]: where the SDK hands what it produces: its log lines, the record of each tool call
//! and each assembled event, and the lifecycle calls `flush` and `shutdown`. The delivery queue
//! ([`Delivery`](crate::delivery::Delivery)) is the sink of a handle that publishes.

use std::future::{Future, ready};
use std::pin::Pin;
use std::time::Duration;

use crate::event::CallRecord;
use crate::event::assemble::EventJob;

/// A boxed `Send` future, as the lifecycle methods of a [`Sink`] return it.
pub(crate) type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Receives the SDK's own log lines (the message texts), the records of tool calls and the
/// events assembled elsewhere (custom events), and answers `flush` and `shutdown`. Each method
/// returns at once, except the blocking twins, which block the calling thread for at most their
/// timeout and a little more.
pub(crate) trait Sink: Send + Sync {
    /// Takes one log line.
    fn log(&self, message: String);

    /// Takes the record of one tool call.
    fn record(&self, record: CallRecord);

    /// Takes one assembled event.
    fn event(&self, job: EventJob);

    /// Takes one assembled event and then the log line `message`, in that order and with no line
    /// of another caller in between: the queue's own lines about the event (a drop) come before
    /// `message`, the lines of the event's later stages after it: the line follows the queueing
    /// of the event.
    fn event_then_log(&self, job: EventJob, message: String) {
        self.event(job);
        self.log(message);
    }

    /// Whether everything taken so far is delivered within `timeout`.
    fn flush(&self, _timeout: Duration) -> BoxFuture<'static, bool> {
        Box::pin(ready(true))
    }

    /// Like [`flush`](Self::flush), and then nothing more is taken.
    fn shutdown(&self, _timeout: Duration) -> BoxFuture<'static, bool> {
        Box::pin(ready(true))
    }

    /// [`flush`](Self::flush) that blocks the calling thread.
    fn flush_blocking(&self, _timeout: Duration) -> bool {
        true
    }

    /// [`shutdown`](Self::shutdown) that blocks the calling thread.
    fn shutdown_blocking(&self, _timeout: Duration) -> bool {
        true
    }
}

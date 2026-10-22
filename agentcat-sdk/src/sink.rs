//! [`Sink`]: where the SDK hands what it produces. In this plan nothing reads it: the
//! [`DiscardSink`] drops everything, and plan 4 puts the delivery queue behind the same seam.

use std::fmt;

/// Receives the SDK's own log lines (the TS message texts).
pub(crate) trait Sink: Send + Sync + fmt::Debug {
    /// Takes one log line.
    fn log(&self, message: String);
}

/// A [`Sink`] that drops everything.
#[derive(Debug)]
pub(crate) struct DiscardSink;

impl Sink for DiscardSink {
    fn log(&self, _message: String) {}
}

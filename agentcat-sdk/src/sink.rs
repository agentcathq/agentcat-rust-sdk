//! [`Sink`]: where the SDK hands what it produces, its log lines and the record of each tool
//! call. In this plan nothing reads it: the [`DiscardSink`] drops everything, and plan 4 puts the
//! delivery queue behind the same seam.

use std::fmt;

use crate::event::CallRecord;

/// Receives the SDK's own log lines (the TS message texts) and the records of tool calls.
pub(crate) trait Sink: Send + Sync + fmt::Debug {
    /// Takes one log line.
    fn log(&self, message: String);

    /// Takes the record of one tool call.
    fn record(&self, record: CallRecord);
}

/// A [`Sink`] that drops everything.
#[derive(Debug)]
pub(crate) struct DiscardSink;

impl Sink for DiscardSink {
    fn log(&self, _message: String) {}

    fn record(&self, _record: CallRecord) {}
}

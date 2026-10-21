//! The event pipeline of the TS SDK: tag validation now; normalization, truncation,
//! sanitization and redaction in later tasks.
//!
//! The pipeline works on the event as TS holds it in memory when the queue runs ([`TsEvent`]): a
//! JSON object with TS's camelCase field names (`sessionId`, `eventType`, `userIntent`,
//! `inputTokens`, ...). Its `timestamp` is an ISO-8601 string with milliseconds, and the
//! redaction walk treats that string as TS treats a `Date`: it keeps it and does not pass it to
//! the redaction hook. Field values are JSON values; TS values that JSON cannot hold (functions,
//! `undefined`) are absent keys.
use serde_json::{Map, Value};

pub(crate) mod normalize;
pub(crate) mod tags;

/// The event as TS holds it in memory (see the module docs).
#[cfg_attr(test, expect(dead_code, reason = "the queue uses it from plan 3 on"))]
pub(crate) type TsEvent = Map<String, Value>;

//! The event pipeline of the TS SDK: tag validation, normalization, truncation and
//! sanitization now; redaction in a later task.
//!
//! The pipeline works on the event as TS holds it in memory when the queue runs ([`TsEvent`]): a
//! JSON object with TS's camelCase field names (`sessionId`, `eventType`, `userIntent`,
//! `inputTokens`, ...). Its `timestamp` is an ISO-8601 string with milliseconds, and the
//! redaction walk treats that string as TS treats a `Date`: it keeps it and does not pass it to
//! the redaction hook. Field values are JSON values; TS values that JSON cannot hold (functions,
//! `undefined`) are absent keys.
use serde_json::{Map, Value};

pub(crate) mod normalize;
pub(crate) mod sanitize;
pub(crate) mod tags;
pub(crate) mod truncate;

/// The event as TS holds it in memory (see the module docs).
pub(crate) type TsEvent = Map<String, Value>;

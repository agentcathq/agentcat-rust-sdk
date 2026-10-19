//! Deterministic trace and span IDs: the session is the trace, the event is
//! the span. Sessionless events get a random trace ID.

use crate::util::{hex, sha256};

fn random_hex(len_bytes: usize) -> String {
    let mut buf = vec![0u8; len_bytes];
    crate::ksuid::fill_random(&mut buf);
    hex(&buf)
}

/// 32 hex chars: the first 16 bytes of SHA-256(session ID).
pub(crate) fn trace_id(session_id: &str) -> String {
    if session_id.is_empty() {
        return random_hex(16);
    }
    hex(&sha256(session_id.as_bytes()))[..32].to_owned()
}

/// 16 hex chars: the first 8 bytes of SHA-256(event ID).
pub(crate) fn span_id(event_id: &str) -> String {
    if event_id.is_empty() {
        return random_hex(8);
    }
    hex(&sha256(event_id.as_bytes()))[..16].to_owned()
}

/// Datadog's 64-bit decimal trace ID: the low half of [`trace_id`].
pub(crate) fn datadog_trace_id(session_id: &str) -> String {
    let hex = trace_id(session_id);
    u64::from_str_radix(&hex[16..32], 16).unwrap_or(0).to_string()
}

/// Datadog's 64-bit decimal span ID.
pub(crate) fn datadog_span_id(event_id: &str) -> String {
    u64::from_str_radix(&span_id(event_id), 16)
        .unwrap_or(0)
        .to_string()
}

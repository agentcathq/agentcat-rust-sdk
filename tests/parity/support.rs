//! Shared helpers for the parity suite.
#![allow(
    dead_code,
    unused_imports,
    reason = "each helper serves some parity files, not all"
)]

use serde_json::{Map, Value};

pub(crate) use crate::logging::capture_logs;

pub(crate) fn fixture_text(rel: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

pub(crate) fn fixture_json(rel: &str) -> Value {
    serde_json::from_str(&fixture_text(rel)).expect("fixture is JSON")
}

/// The TS test helper `sid(label)`: a well-formed `ses_` ID built from a label.
pub(crate) fn sid(label: &str) -> String {
    let body: String = label
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .chain(std::iter::repeat('0'))
        .take(27)
        .collect();
    format!("ses_{body}")
}

pub(crate) fn obj(v: Value) -> Map<String, Value> {
    match v {
        Value::Object(map) => map,
        other => panic!("expected a JSON object, got {other}"),
    }
}

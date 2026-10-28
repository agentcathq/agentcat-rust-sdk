//! JSON helpers shared by the unit tests.
use serde_json::{Map, Value};

/// The JSON object `value`.
///
/// # Panics
///
/// When `value` is not an object, with the value in the message.
pub(crate) fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(object) => object,
        other => panic!("{other} is not an object"),
    }
}

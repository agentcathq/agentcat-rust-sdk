//! The depth, breadth and string-length limiter of the TS SDK (`normalize`, B-092), for JSON
//! values. The TS cases for values JSON cannot hold (`undefined`, `NaN`, `Infinity`, `BigInt`,
//! symbols, functions, `Date`, circular references) have no Rust input (RD-20).
//!
//! A `__proto__` key is an ordinary key here and keeps its normalized value (RD-24); TS assigns
//! it into a fresh object, where it sets the prototype and vanishes from the output.
use serde_json::{Map, Value};

use crate::js;

#[cfg(test)]
mod tests;

/// The default number of container levels `normalize` keeps.
pub(crate) const MAX_DEPTH: usize = 10;
/// The default number of items or keys `normalize` keeps per container.
pub(crate) const MAX_BREADTH: usize = 100;
/// The default longest string, in UTF-16 units.
pub(crate) const MAX_STRING_LENGTH: usize = 32_768;

/// The marker that replaces the items beyond the breadth limit.
const MAX_PROPERTIES: &str = "[MaxProperties ~]";
/// The suffix of a cut string.
const TRUNCATION_SUFFIX: &str = "...";

/// Limits the depth, breadth and string length of `value`, as TS's `normalize` does.
///
/// `depth` is the number of container levels left: an array or object with
/// none left becomes `"[Array]"` or `"[Object]"`. A container keeps `max_breadth` items; an
/// array then gets the item `"[MaxProperties ~]"`, an object the key `"..."` with that value
/// (an existing `"..."` key keeps its position and takes the value). A string longer than
/// `max_string_length` UTF-16 units is cut there (RD-22) and gets `"..."`.
pub(crate) fn normalize(
    value: &Value,
    depth: usize,
    max_breadth: usize,
    max_string_length: usize,
) -> Value {
    match value {
        Value::String(text) => {
            if js::utf16_len(text) > max_string_length {
                let mut cut = js::cut_utf16(text, max_string_length).to_owned();
                cut.push_str(TRUNCATION_SUFFIX);
                Value::String(cut)
            } else {
                value.clone()
            }
        }
        Value::Array(items) => {
            if depth == 0 {
                return Value::String("[Array]".to_owned());
            }
            let mut result: Vec<Value> = items
                .iter()
                .take(max_breadth)
                .map(|item| normalize(item, depth - 1, max_breadth, max_string_length))
                .collect();
            if items.len() > max_breadth {
                result.push(Value::String(MAX_PROPERTIES.to_owned()));
            }
            Value::Array(result)
        }
        Value::Object(map) => {
            if depth == 0 {
                return Value::String("[Object]".to_owned());
            }
            let entries = js::own_entries(map);
            let mut result = Map::new();
            for (count, (key, item)) in entries.iter().enumerate() {
                if count >= max_breadth {
                    result.insert("...".to_owned(), Value::String(MAX_PROPERTIES.to_owned()));
                    break;
                }
                let kept = normalize(item, depth - 1, max_breadth, max_string_length);
                result.insert((*key).clone(), kept);
            }
            Value::Object(result)
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => value.clone(),
    }
}

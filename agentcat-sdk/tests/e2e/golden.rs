//! The golden files: `golden/<scenario>/<transport>-<lane>.json`, the normalized document as
//! pretty JSON with a final newline.
//!
//! With `UPDATE_GOLDEN=1`, [`assert_golden`] writes the file. Otherwise the document must equal
//! the parsed file, key order included; the panic names the first path that differs. Line ends
//! do not matter: the file is compared as JSON, not as text.
use std::path::PathBuf;

use serde_json::{Map, Value};

use super::{Lane, Transport};

/// The longest text of a value in a difference message, in characters.
const SHOWN_CHARS: usize = 200;

/// Compares `actual`, the document of `scenario` over `transport` on `lane`, with its golden
/// file, or writes the file when `UPDATE_GOLDEN=1`.
///
/// # Panics
///
/// When the file is missing, is not JSON or differs from `actual`, or cannot be written.
pub(crate) fn assert_golden(scenario: &str, transport: Transport, lane: Lane, actual: &Value) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/e2e/golden")
        .join(scenario)
        .join(format!("{}-{}.json", transport.name(), lane.version()));
    if update_requested() {
        let directory = path.parent().expect("a golden file has a directory");
        std::fs::create_dir_all(directory)
            .unwrap_or_else(|error| panic!("cannot create {}: {error}", directory.display()));
        let mut text = serde_json::to_string_pretty(actual).expect("a document serializes");
        text.push('\n');
        std::fs::write(&path, text)
            .unwrap_or_else(|error| panic!("cannot write {}: {error}", path.display()));
        return;
    }
    let golden = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "cannot read the golden file {} ({error}): run with UPDATE_GOLDEN=1 to write it",
            path.display()
        )
    });
    assert_matches(&golden, actual, &path.display().to_string());
}

/// Compares `actual` with the JSON text `golden`; `origin` names the golden in the panic.
///
/// # Panics
///
/// When `golden` is not JSON, or differs from `actual`: the message names the first differing
/// path, in document order.
pub(crate) fn assert_matches(golden: &str, actual: &Value, origin: &str) {
    let expected: Value = serde_json::from_str(golden)
        .unwrap_or_else(|error| panic!("{origin} is not JSON: {error}"));
    if let Some(difference) = first_difference(&expected, actual, "$") {
        panic!("{origin} differs at {difference} (UPDATE_GOLDEN=1 rewrites the golden files)");
    }
}

/// Whether `UPDATE_GOLDEN=1` asks for the golden files to be written.
#[expect(
    clippy::disallowed_methods,
    reason = "the variable that asks for the golden files to be written"
)]
fn update_requested() -> bool {
    std::env::var("UPDATE_GOLDEN").is_ok_and(|value| value == "1")
}

/// The first place, in document order, where `actual` differs from `expected` (at `path`), as
/// `<path>: expected <value>, got <value>`.
fn first_difference(expected: &Value, actual: &Value, path: &str) -> Option<String> {
    match (expected, actual) {
        (Value::Object(expected), Value::Object(actual)) => {
            object_difference(expected, actual, path)
        }
        (Value::Array(expected), Value::Array(actual)) => {
            for (index, (expected, actual)) in expected.iter().zip(actual).enumerate() {
                let difference = first_difference(expected, actual, &format!("{path}[{index}]"));
                if difference.is_some() {
                    return difference;
                }
            }
            (expected.len() != actual.len()).then(|| {
                format!(
                    "{path}: expected {} items, got {}",
                    expected.len(),
                    actual.len()
                )
            })
        }
        _ => (expected != actual).then(|| {
            format!(
                "{path}: expected {}, got {}",
                shown(expected),
                shown(actual)
            )
        }),
    }
}

/// The first difference of two objects at `path`: a field missing, a field more, a field out of
/// order, or a differing value.
fn object_difference(
    expected: &Map<String, Value>,
    actual: &Map<String, Value>,
    path: &str,
) -> Option<String> {
    let mut expected_fields = expected.iter();
    let mut actual_fields = actual.iter();
    loop {
        match (expected_fields.next(), actual_fields.next()) {
            (None, None) => return None,
            (Some((key, value)), actual_field) => {
                let at = field_path(path, key);
                let Some(found) = actual.get(key) else {
                    return Some(format!("{at}: expected {}, got nothing", shown(value)));
                };
                if let Some((actual_key, _)) = actual_field.filter(|(other, _)| *other != key) {
                    if !expected.contains_key(actual_key) {
                        let value = &actual[actual_key];
                        let at = field_path(path, actual_key);
                        return Some(format!("{at}: expected nothing, got {}", shown(value)));
                    }
                    return Some(format!(
                        "{path}: expected the field {key:?} before {actual_key:?}"
                    ));
                }
                let difference = first_difference(value, found, &at);
                if difference.is_some() {
                    return difference;
                }
            }
            (None, Some((key, value))) => {
                let at = field_path(path, key);
                return Some(format!("{at}: expected nothing, got {}", shown(value)));
            }
        }
    }
}

/// The path of the field `key` of the object at `path`: `.key`, or `["key"]` when the key is not
/// made of ASCII letters, digits, `_` and `-`.
pub(super) fn field_path(path: &str, key: &str) -> String {
    let plain = !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if plain {
        format!("{path}.{key}")
    } else {
        format!("{path}[{}]", Value::from(key))
    }
}

/// `value` as JSON, cut to [`SHOWN_CHARS`] characters.
pub(super) fn shown(value: &Value) -> String {
    let text = value.to_string();
    match text.char_indices().nth(SHOWN_CHARS) {
        Some((cut, _)) => format!("{}... ({} characters)", &text[..cut], text.chars().count()),
        None => text,
    }
}

//! The TS-generated vector files in `agentcat-sdk/tests/vectors/`: `{"ts_sha", "generator",
//! "cases": [...]}`, each case an object with a unique `name` (written by
//! `oracle/src/vectors/write.mjs`; `tests/vector_integrity.rs` checks hashes and the TS SHA).
use std::path::Path;

use serde_json::{Map, Value};

use super::markers;

/// The cases of `tests/vectors/<set>.json`, with every `$f64` marker decoded and every `$repeat`
/// macro of a case's `input` expanded. `$utf16` markers stay: a test compares through
/// [`markers::d09`] where RD-22 applies.
///
/// # Panics
///
/// With the file name, on a file that cannot be read or parsed, a case that is not an object or
/// has no string `name`, and a malformed marker.
pub(crate) fn cases(set: &str) -> Vec<Map<String, Value>> {
    let file = format!("tests/vectors/{set}.json");
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(&file);
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{file}: cannot read: {e}"));
    let cases = match serde_json::from_str(&text) {
        Ok(Value::Object(mut doc)) => doc.remove("cases"),
        Ok(_) => None,
        Err(e) => panic!("{file}: {e}"),
    };
    let Some(Value::Array(cases)) = cases else {
        panic!("{file}: no \"cases\" array");
    };
    cases
        .into_iter()
        .enumerate()
        .map(|(i, case)| {
            let label = case
                .get("name")
                .and_then(Value::as_str)
                .map_or_else(String::new, |name| format!(" {name:?}"));
            load(case).unwrap_or_else(|e| panic!("{file}: case{label} (#{i}): {e}"))
        })
        .collect()
}

/// One case of a vector file, decoded as [`cases`] says.
fn load(case: Value) -> Result<Map<String, Value>, String> {
    if case.get("name").and_then(Value::as_str).is_none() {
        return Err("not an object with a string \"name\"".to_owned());
    }
    let Value::Object(mut case) = markers::try_decode_f64(case)? else {
        return Err("not an object".to_owned());
    };
    if let Some(input) = case.get_mut("input") {
        *input = markers::try_expand(input.take())?;
    }
    Ok(case)
}

/// The `name` of a case of [`cases`].
///
/// # Panics
///
/// When the case has no string `name` (never for a case of [`cases`]).
pub(crate) fn name(case: &Map<String, Value>) -> &str {
    case.get("name")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("a vector case has no string \"name\": {case:?}"))
}

mod tests {
    use serde_json::json;

    use super::{cases, name};

    #[test]
    fn constants_has_cases_and_every_case_has_a_name() {
        let cases = cases("constants");
        assert_ne!(cases.len(), 0, "constants.json has no case");
        for case in &cases {
            assert!(!name(case).is_empty(), "{case:?}");
        }
    }

    #[test]
    fn name_is_the_name_field() {
        let case = json!({"name": "x:y", "input": 1});
        assert_eq!(name(case.as_object().unwrap()), "x:y");
    }

    #[test]
    #[should_panic(expected = "tests/vectors/no-such-set.json")]
    fn a_missing_set_panics_with_the_file_name() {
        let _ = cases("no-such-set");
    }
}

//! Integrity of the fixture files in `tests/vectors/`: each one parses, holds named cases whose
//! names are unique, has no provenance fields, and is loaded by at least one test.
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

type Res<T> = Result<T, String>;

fn vectors_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/vectors")
}

/// The parsed document of the fixture file `name`.
fn document(name: &str) -> Res<Value> {
    let path = vectors_dir().join(name);
    let bytes = fs::read(&path).map_err(|e| format!("{name}: cannot read: {e}"))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("{name}: invalid JSON: {e}"))
}

/// The cases of the fixture file `name`.
fn cases(name: &str) -> Res<Vec<Value>> {
    match document(name)?.get_mut("cases").map(Value::take) {
        Some(Value::Array(cases)) => Ok(cases),
        _ => Err(format!("{name}: no \"cases\" array")),
    }
}

/// The names of all `*.json` files in the vectors directory, sorted.
fn json_files() -> Res<Vec<String>> {
    let mut names = Vec::new();
    let entries = fs::read_dir(vectors_dir()).map_err(|e| format!("read vectors dir: {e}"))?;
    for entry in entries {
        let name = entry
            .map_err(|e| format!("read vectors dir: {e}"))?
            .file_name()
            .to_string_lossy()
            .into_owned();
        if Path::new(&name)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
        {
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}

/// The text of every Rust file under `dir`, appended to `out`.
fn rust_sources(dir: &Path, out: &mut String) -> Res<()> {
    let entries = fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries {
        let path = entry.map_err(|e| format!("{}: {e}", dir.display()))?.path();
        if path.is_dir() {
            rust_sources(&path, out)?;
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            out.push_str(&text);
        }
    }
    Ok(())
}

/// The checks below pass vacuously on an empty directory.
#[test]
fn at_least_one_fixture_file_exists() -> Res<()> {
    assert!(
        !json_files()?.is_empty(),
        "tests/vectors holds no fixture file"
    );
    Ok(())
}

#[test]
fn every_fixture_file_has_named_cases() -> Res<()> {
    for file in json_files()? {
        let cases = cases(&file)?;
        assert!(!cases.is_empty(), "{file}: no case");
        for (i, case) in cases.iter().enumerate() {
            assert!(
                case.get("name").is_some_and(Value::is_string),
                "{file}: case #{i} has no string \"name\""
            );
        }
    }
    Ok(())
}

#[test]
fn case_names_are_unique_within_a_file() -> Res<()> {
    for file in json_files()? {
        let mut seen = std::collections::BTreeSet::new();
        for (i, case) in cases(&file)?.iter().enumerate() {
            let name = case["name"]
                .as_str()
                .ok_or_else(|| format!("{file}: case #{i} has no string \"name\""))?;
            assert!(seen.insert(name), "{file}: duplicate case {name:?}");
        }
    }
    Ok(())
}

#[test]
fn no_fixture_file_has_a_provenance_field() -> Res<()> {
    for file in json_files()? {
        let doc = document(&file)?;
        for key in ["ts_sha", "generator"] {
            assert!(doc.get(key).is_none(), "{file}: has the field {key:?}");
        }
        for case in cases(&file)? {
            assert!(
                case.get("ts_test").is_none(),
                "{file}: case {} has the field \"ts_test\"",
                case["name"]
            );
        }
    }
    Ok(())
}

/// A file `<stem>.json` counts as loaded when a Rust file under `src/` or `tests/` calls
/// `vectors::cases` with the literal stem.
#[test]
fn every_fixture_file_is_loaded_by_a_test() -> Res<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut sources = String::new();
    rust_sources(&root.join("src"), &mut sources)?;
    rust_sources(&root.join("tests"), &mut sources)?;
    for file in json_files()? {
        let stem = file.trim_end_matches(".json");
        assert!(
            sources.contains(&format!("vectors::cases(\"{stem}\")")),
            "{file}: no test loads it"
        );
    }
    Ok(())
}

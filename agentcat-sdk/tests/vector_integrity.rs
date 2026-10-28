//! Integrity of the TS-generated vector files: each one is listed in the manifest, matches its
//! hash, and comes from the TS commit pinned in `oracle/oracle.lock`.
use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

type Res<T> = Result<T, String>;

fn vectors_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/vectors")
}

fn read(path: &Path) -> Res<Vec<u8>> {
    fs::read(path).map_err(|e| format!("{}: cannot read: {e}", path.display()))
}

/// The `(hash, file name)` pairs of `MANIFEST.sha256`.
fn manifest() -> Res<Vec<(String, String)>> {
    let text = String::from_utf8(read(&vectors_dir().join("MANIFEST.sha256"))?)
        .map_err(|e| format!("MANIFEST.sha256: {e}"))?;
    text.lines()
        .filter(|l| !l.is_empty())
        .map(|l| {
            l.split_once("  ")
                .map(|(hash, name)| (hash.to_owned(), name.to_owned()))
                .ok_or_else(|| format!("MANIFEST.sha256: malformed line {l:?}"))
        })
        .collect()
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

/// The checks below pass vacuously on an empty directory or manifest.
#[test]
fn at_least_one_vector_set_exists() -> Res<()> {
    assert!(
        !json_files()?.is_empty(),
        "tests/vectors holds no vector file"
    );
    assert!(
        !manifest()?.is_empty(),
        "MANIFEST.sha256 lists no vector file"
    );
    Ok(())
}

#[test]
fn every_vector_file_matches_the_manifest() -> Res<()> {
    for (hash, name) in manifest()? {
        let bytes = read(&vectors_dir().join(&name))?;
        let actual = hex::encode(Sha256::digest(&bytes));
        assert_eq!(actual, hash, "{name}: sha256 differs from MANIFEST.sha256");
    }
    Ok(())
}

#[test]
fn every_vector_file_is_listed() -> Res<()> {
    let listed: Vec<String> = manifest()?.into_iter().map(|(_, name)| name).collect();
    for name in json_files()? {
        assert!(
            listed.contains(&name),
            "{name}: not listed in MANIFEST.sha256"
        );
    }
    Ok(())
}

#[test]
fn every_vector_file_comes_from_the_locked_sha() -> Res<()> {
    let lock_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../oracle/oracle.lock");
    let lock: serde_json::Value =
        serde_json::from_slice(&read(&lock_path)?).map_err(|e| format!("oracle.lock: {e}"))?;
    let locked = lock["sha"].as_str().ok_or("oracle.lock has no sha")?;
    for name in json_files()? {
        let doc: serde_json::Value = serde_json::from_slice(&read(&vectors_dir().join(&name))?)
            .map_err(|e| format!("{name}: invalid JSON: {e}"))?;
        assert_eq!(
            doc["ts_sha"].as_str(),
            Some(locked),
            "{name}: ts_sha differs from oracle.lock"
        );
    }
    Ok(())
}

//! The tripwire: every TS test case of an enforced file has a Rust test at
//! the path the manifest names, and every `N/A` case is in `NOT_PORTED.md`.

use std::collections::BTreeSet;
use std::process::Command;

const MANIFEST: &str = include_str!("manifest.tsv");
const NOT_PORTED: &str = include_str!("NOT_PORTED.md");

/// TS test files whose cases must all have Rust tests.
const ENFORCED: &[&str] = &[
    "constants.test.ts",
    "ksuid.test.ts",
    "posthog-uuidv7.test.ts",
    "tokenEstimate.test.ts",
    "validation.test.ts",
];

/// Cases of enforced files that a later plan ports: (TS file, TS test, plan).
const DEFERRED: &[(&str, &str, u8)] = &[];

struct Case {
    ts_file: &'static str,
    ts_name: &'static str,
    disposition: &'static str,
    rust_test: &'static str,
}

fn cases() -> Vec<Case> {
    MANIFEST
        .lines()
        .skip(1)
        .map(|line| {
            let mut cols = line.split('\t');
            let mut next = || cols.next().expect("four tab-separated columns");
            Case {
                ts_file: next(),
                ts_name: next(),
                disposition: next(),
                rust_test: next(),
            }
        })
        .collect()
}

fn list_with(extra: &[&str]) -> BTreeSet<String> {
    let exe = std::env::current_exe().expect("path of the running test binary");
    let output = Command::new(exe)
        .args(["--list", "--format", "terse"])
        .args(extra)
        .output()
        .expect("list the tests of this binary");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|l| l.strip_suffix(": test"))
        .map(str::to_owned)
        .collect()
}

/// Every test of this binary except the `#[ignore]` ones, which never run.
fn listed_tests() -> BTreeSet<String> {
    let ignored = list_with(&["--ignored"]);
    list_with(&[]).difference(&ignored).cloned().collect()
}

fn deferred(case: &Case) -> bool {
    DEFERRED
        .iter()
        .any(|(f, n, _)| *f == case.ts_file && *n == case.ts_name)
}

#[test]
fn manifest_covers_every_ts_case() {
    let all = cases();
    assert_eq!(all.len(), 1024);
    let count = |d: &str| all.iter().filter(|c| c.disposition == d).count();
    assert_eq!((count("port"), count("adapt"), count("na")), (672, 201, 151));
    let paths: BTreeSet<_> = all.iter().map(|c| c.rust_test).collect();
    assert_eq!(paths.len(), all.len(), "duplicate rust_test paths");
}

#[test]
fn enforced_and_deferred_entries_name_real_cases() {
    let all = cases();
    for file in ENFORCED {
        assert!(
            all.iter().any(|c| c.ts_file == *file),
            "{file} is not in the manifest"
        );
    }
    for (file, name, plan) in DEFERRED {
        assert!(
            ENFORCED.contains(file),
            "deferred case of a file that is not enforced: {file}"
        );
        assert!(
            all.iter()
                .any(|c| c.ts_file == *file && c.ts_name == *name && c.disposition != "na"),
            "no portable case {file} > {name}"
        );
        assert!((2..=7).contains(plan), "plan {plan} for {name}");
    }
}

#[test]
fn every_enforced_case_has_its_rust_test() {
    let listed = listed_tests();
    let missing: Vec<String> = cases()
        .iter()
        .filter(|c| ENFORCED.contains(&c.ts_file) && c.disposition != "na" && !deferred(c))
        .map(|c| format!("parity::{}", c.rust_test))
        .filter(|path| !listed.contains(path))
        .collect();
    assert!(
        missing.is_empty(),
        "{} TS cases have no Rust test:\n{}",
        missing.len(),
        missing.join("\n")
    );
}

#[test]
fn every_enforced_na_case_is_listed_in_not_ported() {
    let missing: Vec<&str> = cases()
        .iter()
        .filter(|c| ENFORCED.contains(&c.ts_file) && c.disposition == "na")
        .filter(|c| {
            !NOT_PORTED.contains(&format!(
                "| `{}` | {} |",
                c.ts_file,
                c.ts_name.replace('|', "\\|")
            ))
        })
        .map(|c| c.ts_name)
        .collect();
    assert!(
        missing.is_empty(),
        "N/A cases missing from NOT_PORTED.md: {missing:?}"
    );
}

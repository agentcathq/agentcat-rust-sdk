//! `scripts/mutants-run.sh`, which runs `cargo mutants` and maps its exit code. Each test runs the
//! script in a scratch directory with `MUTANTS_BIN` set to a stub of cargo: the stub logs its
//! arguments, writes `mutants.out/missed.txt` with the lines of `STUB_MISSED` (no file when
//! `STUB_MISSED` is unset), and exits with `STUB_CODE`. Each run starts from an empty environment.
#![cfg(unix)]
#![expect(clippy::expect_used, reason = "test helpers")]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// The stub of cargo.
const STUB: &str = r#"#!/usr/bin/env bash
printf '%s\n' "$*" > stub-args.log
if [ -n "${STUB_MISSED+set}" ]; then
  mkdir -p mutants.out
  printf '%s' "$STUB_MISSED" > mutants.out/missed.txt
fi
exit "$STUB_CODE"
"#;

/// One missed mutant, as cargo-mutants lists it.
const MISSED: &str = "agentcat-sdk/src/wire.rs:10:5: replace f -> bool with true\n";

/// A temporary directory with the stub, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static COUNT: AtomicUsize = AtomicUsize::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos());
        let path = std::env::temp_dir().join(format!(
            "mutants-run-{}-{}-{nanos}",
            std::process::id(),
            COUNT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("the scratch directory is created");
        let stub = path.join("cargo-stub");
        fs::write(&stub, STUB).expect("the stub is written");
        fs::set_permissions(&stub, fs::Permissions::from_mode(0o755))
            .expect("the stub is executable");
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A run of the script.
struct Run {
    code: Option<i32>,
    stdout: String,
    stderr: String,
    /// The arguments the stub got.
    args: String,
}

/// The `PATH` of the test process, which finds bash.
#[expect(
    clippy::disallowed_methods,
    reason = "the script under test needs the tools of the test process"
)]
fn system_path() -> std::ffi::OsString {
    std::env::var_os("PATH").unwrap_or_default()
}

/// Runs the script with the arguments `--in-diff d.diff` in a new scratch directory; the stub exits
/// with `code` and lists `missed` (no file when `None`); `ok_codes` sets `MUTANTS_OK_CODES`.
fn run(code: i32, missed: Option<&str>, ok_codes: Option<&str>) -> Run {
    run_in(&Scratch::new(), code, missed, ok_codes)
}

/// [`run`] in the scratch directory `scratch`.
fn run_in(scratch: &Scratch, code: i32, missed: Option<&str>, ok_codes: Option<&str>) -> Run {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/mutants-run.sh");
    let mut command = Command::new("bash");
    command
        .arg(script)
        .args(["--in-diff", "d.diff"])
        .current_dir(&scratch.0)
        .env_clear()
        .env("PATH", system_path())
        .env("MUTANTS_BIN", scratch.0.join("cargo-stub"))
        .env("STUB_CODE", code.to_string());
    if let Some(missed) = missed {
        command.env("STUB_MISSED", missed);
    }
    if let Some(ok_codes) = ok_codes {
        command.env("MUTANTS_OK_CODES", ok_codes);
    }
    let output = command.output().expect("the script runs");
    Run {
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        args: fs::read_to_string(scratch.0.join("stub-args.log")).unwrap_or_default(),
    }
}

#[test]
fn the_script_runs_cargo_mutants_with_its_arguments() {
    let result = run(0, Some(""), None);
    assert_eq!(result.args, "mutants --in-diff d.diff\n");
}

#[test]
fn a_clean_run_passes() {
    let result = run(0, Some(""), None);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(result.stdout, "");
    assert_eq!(result.stderr, "");
}

#[test]
fn a_timeout_without_a_missed_mutant_passes() {
    for missed in [Some(""), None] {
        let result = run(3, missed, None);
        assert_eq!(result.code, Some(0), "{missed:?}: {}", result.stderr);
        assert_eq!(
            result.stdout,
            "mutants-run: cargo mutants exited 3, which passes here\n"
        );
    }
}

/// cargo-mutants exits 3 when one mutant timed out and another was missed: the missed mutant
/// fails the run.
#[test]
fn a_timeout_with_a_missed_mutant_fails() {
    let result = run(3, Some(MISSED), None);
    assert_eq!(result.code, Some(2));
    assert_eq!(
        result.stderr,
        format!("mutants-run: cargo mutants exited 3, but it missed 1 mutant(s):\n{MISSED}")
    );
    assert_eq!(result.stdout, "");
}

#[test]
fn a_missed_mutant_fails_whatever_the_exit_code() {
    let result = run(0, Some(MISSED), Some("0 3"));
    assert_eq!(result.code, Some(2));
    assert!(
        result.stderr.contains("missed 1 mutant(s)"),
        "{}",
        result.stderr
    );
}

/// The nightly shards accept a missed mutant (exit 2): `mutants-score` judges them all together.
#[test]
fn a_missed_mutant_passes_when_2_is_an_accepted_code() {
    for code in [2, 3] {
        let result = run(code, Some(MISSED), Some("0 2 3"));
        assert_eq!(result.code, Some(0), "{code}: {}", result.stderr);
        assert_eq!(
            result.stdout,
            format!("mutants-run: cargo mutants exited {code}, which passes here\n")
        );
    }
}

#[test]
fn an_exit_code_not_in_the_list_is_returned_as_it_is() {
    for (code, missed) in [(1, None), (2, Some(MISSED)), (4, Some(""))] {
        let result = run(code, missed, None);
        assert_eq!(result.code, Some(code), "{}", result.stderr);
        assert_eq!(
            result.stderr,
            format!("mutants-run: cargo mutants exited {code}\n")
        );
    }
    let result = run(3, Some(""), Some("0 2"));
    assert_eq!(result.code, Some(3), "{}", result.stderr);
}

/// cargo-mutants writes no mutants.out when it has nothing to do (an empty diff): the missed
/// mutants of an earlier run do not count, and its output moves to mutants.out.old, as
/// cargo-mutants itself moves it when it starts.
#[test]
fn an_earlier_runs_missed_mutants_do_not_count() {
    let scratch = Scratch::new();
    fs::create_dir_all(scratch.0.join("mutants.out")).expect("the old output is created");
    fs::write(scratch.0.join("mutants.out/missed.txt"), MISSED).expect("the old list is written");
    fs::create_dir_all(scratch.0.join("mutants.out.old")).expect("an older output is created");
    let result = run_in(&scratch, 0, None, None);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert!(!scratch.0.join("mutants.out").exists());
    let moved = fs::read_to_string(scratch.0.join("mutants.out.old/missed.txt"));
    assert_eq!(moved.expect("the old list moved"), MISSED);
}

//! Repository tasks for the `AgentCat` Rust SDK workspace.
//!
//! `cargo run -p xtask -- parity [--final]` checks `parity/manifest.toml` against the TS test list
//! (`parity/ts-tests.json`), the Rust tests that `cargo nextest list` reports, and the harness
//! scenarios in `harness/scenarios/` (see [`parity`]). It prints each problem and exits 1 if there
//! is any. `todo` entries are problems only with `--final`.
pub mod parity;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use parity::{Manifest, Problem, Status, TsTests};

const USAGE: &str = "usage: cargo run -p xtask -- parity [--final]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let final_gate = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["parity"] => false,
        ["parity", "--final"] => true,
        _ => {
            report_error(USAGE);
            return ExitCode::from(2);
        }
    };
    match parity_command(final_gate) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(message) => {
            report_error(&format!("xtask parity: {message}"));
            ExitCode::FAILURE
        }
    }
}

/// Runs the parity check; `Ok(true)` when there is no problem.
fn parity_command(final_gate: bool) -> Result<bool, String> {
    let root = workspace_root();
    let ts = TsTests::from_json(&read(&root.join("parity/ts-tests.json"))?)?;
    let manifest = Manifest::from_toml(&read(&root.join("parity/manifest.toml"))?)?;
    let rust_tests = rust_tests(&root)?;
    let scenarios = parity::scenario_names(&root.join("harness/scenarios"))?;
    let problems = parity::check(&ts, &manifest, &rust_tests, &scenarios, final_gate);
    report(&ts, &manifest, &problems, final_gate);
    Ok(problems.is_empty())
}

/// The workspace root: the parent of this crate's directory.
fn workspace_root() -> PathBuf {
    let xtask = Path::new(env!("CARGO_MANIFEST_DIR"));
    xtask.parent().unwrap_or(xtask).to_path_buf()
}

fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

/// The workspace's Rust test names, from `cargo nextest list` (which builds the test binaries).
fn rust_tests(root: &Path) -> Result<BTreeSet<String>, String> {
    let args = [
        "nextest",
        "list",
        "--workspace",
        "--all-features",
        "--message-format",
        "json",
    ];
    let output = Command::new("cargo")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|e| format!("cannot run `cargo nextest list`: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "`cargo {}` failed ({}):\n{}",
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let stdout = String::from_utf8(output.stdout)
        .map_err(|e| format!("`cargo nextest list` printed invalid UTF-8: {e}"))?;
    parity::rust_test_names(&stdout)
}

#[expect(
    clippy::print_stdout,
    reason = "xtask reports its result on the terminal"
)]
fn report(ts: &TsTests, manifest: &Manifest, problems: &[Problem], final_gate: bool) {
    for problem in problems {
        println!("{problem}");
    }
    let count = |status| manifest.tests.iter().filter(|e| e.status == status).count();
    println!(
        "parity{}: {} TS tests; {} ported, {} n/a, {} todo; {} problem(s)",
        if final_gate { " --final" } else { "" },
        ts.tests.len(),
        count(Status::Ported),
        count(Status::NotApplicable),
        count(Status::Todo),
        problems.len()
    );
}

#[expect(clippy::print_stderr, reason = "xtask reports errors on the terminal")]
fn report_error(message: &str) {
    eprintln!("{message}");
}

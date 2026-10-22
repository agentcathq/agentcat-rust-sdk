//! Repository tasks for the `AgentCat` Rust SDK workspace.
//!
//! `cargo run -p xtask -- parity [--final | --print-missing]` checks `parity/manifest.toml` against the TS test list
//! (`parity/ts-tests.json`, which must come from the commit `oracle/oracle.lock` pins), the Rust
//! tests that `cargo nextest list` reports, the harness scenarios in `harness/scenarios/`, the
//! vector files and the spec's deviation register (see [`parity`]). It prints each problem and
//! exits 1 if there is any. `todo` entries are problems only with `--final`. `--print-missing`
//! instead prints a `todo` block for each TS test with no manifest entry and exits 0.
//!
//! `cargo run -q -p xtask -- rmcp-canary` checks that `Tracked<H>` forwards every method of rmcp's
//! `ServerHandler` (see [`rmcp_canary`]).
//!
//! [`sdk_manifest`] checks R07 in `agentcat-sdk/Cargo.toml` (its test runs with every
//! `cargo nextest run --workspace`).
pub mod parity;
pub mod rmcp_canary;
pub mod sdk_manifest;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use parity::{Report, Status};

const USAGE: &str = "usage: cargo run -p xtask -- parity [--final | --print-missing] | rmcp-canary";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args
        .iter()
        .map(String::as_str)
        .eq(["parity", "--print-missing"])
    {
        return match parity::print_missing_repo(&workspace_root()) {
            Ok(text) => {
                print_text(&text);
                ExitCode::SUCCESS
            }
            Err(message) => {
                report_error(&format!("xtask parity: {message}"));
                ExitCode::FAILURE
            }
        };
    }
    if args.first().map(String::as_str) == Some("rmcp-canary") {
        let (code, lines) = rmcp_canary::run(&args[1..], &workspace_root());
        for line in &lines {
            report_error(line);
        }
        return ExitCode::from(code);
    }
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
    let rust_tests = rust_tests(&root)?;
    let report = parity::check_repo(&root, &rust_tests, final_gate)?;
    print_report(&report, final_gate);
    Ok(report.problems.is_empty())
}

/// The workspace root: the parent of this crate's directory.
fn workspace_root() -> PathBuf {
    let xtask = Path::new(env!("CARGO_MANIFEST_DIR"));
    xtask.parent().unwrap_or(xtask).to_path_buf()
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
fn print_report(report: &Report, final_gate: bool) {
    for problem in &report.problems {
        println!("{problem}");
    }
    let count = |status| {
        report
            .manifest
            .tests
            .iter()
            .filter(|e| e.status == status)
            .count()
    };
    println!(
        "parity{}: {} TS tests; {} ported, {} n/a, {} todo; {} problem(s)",
        if final_gate { " --final" } else { "" },
        report.ts.tests.len(),
        count(Status::Ported),
        count(Status::NotApplicable),
        count(Status::Todo),
        report.problems.len()
    );
}

#[expect(
    clippy::print_stdout,
    reason = "xtask prints the blocks for the caller to append"
)]
fn print_text(text: &str) {
    print!("{text}");
}

#[expect(clippy::print_stderr, reason = "xtask reports errors on the terminal")]
fn report_error(message: &str) {
    eprintln!("{message}");
}

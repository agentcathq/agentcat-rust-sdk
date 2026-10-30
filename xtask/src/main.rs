//! Repository tasks for the `AgentCat` Rust SDK workspace.
//!
//! `cargo run -q -p xtask -- rmcp-canary` checks that `Tracked<H>` forwards every method of rmcp's
//! `ServerHandler` (see [`rmcp_canary`]).
//!
//! `cargo run -p xtask -- public-api [--bless]` checks the public API of `agentcat-sdk` against
//! `agentcat-sdk/public-api.txt` and an allowlist of crates (see [`public_api`]);
//! `cargo run -p xtask -- package-list` checks the files of the published package.
//!
//! `cargo run -p xtask -- coverage <summary.json>` checks the line coverage floors of
//! `agentcat-sdk` on an llvm-cov summary (see [`coverage`]).
//!
//! `cargo run -q -p xtask -- rmcp-versions` prints the JSON list of the rmcp versions that CI
//! tests, from the crates.io API (see [`rmcp_versions`]).
//!
//! `cargo run -q -p xtask -- examples-check <--all | stdio | streamable_http | hooks | stateless_http>`
//! runs one customer example, or with `--all` every one, against a local mock API and checks it
//! (see [`examples_check`]).
//!
//! [`sdk_manifest`] checks that `agentcat-sdk/Cargo.toml` turns on `serde_json`'s
//! `preserve_order` (its test runs with every `cargo nextest run --workspace`).
pub mod coverage;
pub mod examples_check;
pub mod public_api;
pub mod rmcp_canary;
pub mod rmcp_versions;
pub mod sdk_manifest;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "usage: cargo run -p xtask -- rmcp-canary | public-api [--bless] | package-list | coverage <llvm-cov summary json> | rmcp-versions | examples-check <example>";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("rmcp-canary") {
        let (code, lines) = rmcp_canary::run(&args[1..], &workspace_root());
        for line in &lines {
            report_error(line);
        }
        return ExitCode::from(code);
    }
    if args.first().map(String::as_str) == Some("public-api") {
        let (code, lines) = public_api::run(&args[1..], &workspace_root());
        for line in &lines {
            report_error(line);
        }
        return ExitCode::from(code);
    }
    if args.first().map(String::as_str) == Some("package-list") {
        let (code, lines) = public_api::run_package_list(&args[1..], &workspace_root());
        for line in &lines {
            report_error(line);
        }
        return ExitCode::from(code);
    }
    if args.first().map(String::as_str) == Some("rmcp-versions") {
        let (code, json, problems) = rmcp_versions::run(&args[1..]);
        if let Some(json) = json {
            print_text(&format!("{json}\n"));
        }
        for line in &problems {
            report_error(line);
        }
        return ExitCode::from(code);
    }
    if args.first().map(String::as_str) == Some("examples-check") {
        let (code, lines) = examples_check::run(&args[1..], &workspace_root());
        for line in &lines {
            report_error(line);
        }
        return ExitCode::from(code);
    }
    if args.first().map(String::as_str) == Some("coverage") {
        let (code, table, problems) = coverage::run(&args[1..]);
        print_text(&table);
        for line in &problems {
            report_error(line);
        }
        return ExitCode::from(code);
    }
    report_error(USAGE);
    ExitCode::from(2)
}

/// The workspace root: the parent of this crate's directory.
fn workspace_root() -> PathBuf {
    let xtask = Path::new(env!("CARGO_MANIFEST_DIR"));
    xtask.parent().unwrap_or(xtask).to_path_buf()
}

#[expect(
    clippy::print_stdout,
    reason = "xtask prints the coverage table and the rmcp version list"
)]
fn print_text(text: &str) {
    print!("{text}");
}

#[expect(clippy::print_stderr, reason = "xtask reports errors on the terminal")]
fn report_error(message: &str) {
    eprintln!("{message}");
}

//! Records the rustc version for the log line prefix (C42). It never fails the build.
#![expect(
    clippy::disallowed_methods,
    reason = "a build script reads RUSTC, which cargo sets for it"
)]

use std::io::Write;
use std::process::Command;

fn rustc_version() -> Option<String> {
    let rustc = std::env::var_os("RUSTC")?;
    let output = Command::new(rustc).arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    text.split_whitespace().nth(1).map(str::to_owned)
}

fn main() {
    let version = rustc_version().unwrap_or_else(|| "unknown".to_owned());
    let mut out = std::io::stdout();
    // A closed stdout cannot fail the build either.
    let _ = writeln!(out, "cargo:rerun-if-env-changed=RUSTC");
    let _ = writeln!(out, "cargo:rustc-env=AGENTCAT_RUSTC_VERSION={version}");
}

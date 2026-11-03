//! Records the rustc version for the log line prefix, and probes the resolved rmcp for the
//! SEP-2640 Skills methods (`cfg(agentcat_rmcp_skills)`). It never fails the build.
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

/// Whether the resolved rmcp knows the SEP-2640 Skills extension: `skills/list`, `skills/get` and
/// `resources/directory/read`, added to `ServerHandler` together (rmcp commit e5698cb, after 3.8.0).
/// The build dependency is the same rmcp as the library's (one rmcp 3.x in the lock), so a
/// `skills/list` request that parses to its own variant means the trait has the three methods.
/// An older rmcp parses it to another variant or not at all.
fn rmcp_has_skills() -> bool {
    let request = serde_json::json!({"method": "skills/list", "params": {}});
    serde_json::from_value::<rmcp::model::ClientRequest>(request)
        .is_ok_and(|parsed| format!("{parsed:?}").starts_with("ListSkillsRequest("))
}

fn main() {
    let version = rustc_version().unwrap_or_else(|| "unknown".to_owned());
    let mut out = std::io::stdout();
    // A closed stdout cannot fail the build either.
    let _ = writeln!(out, "cargo:rerun-if-env-changed=RUSTC");
    let _ = writeln!(out, "cargo:rustc-env=AGENTCAT_RUSTC_VERSION={version}");
    let _ = writeln!(out, "cargo:rustc-check-cfg=cfg(agentcat_rmcp_skills)");
    if rmcp_has_skills() {
        let _ = writeln!(out, "cargo:rustc-cfg=agentcat_rmcp_skills");
    }
}

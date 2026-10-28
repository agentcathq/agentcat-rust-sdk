//! The public API gate (decision C51) and the package file list check.
//!
//! `cargo run -p xtask -- public-api [--bless]` runs
//! `cargo +nightly public-api -p agentcat-sdk --all-features -ss` and compares the output with
//! `agentcat-sdk/public-api.txt`. With `--bless` it writes the file instead. Either way, every
//! crate path in the output must start with an allowed crate ([`ALLOWED_CRATES`]), so a type of
//! `reqwest`, `tokio`, `chrono` or `agentcat-api` never appears in a public signature (C08).
//! It exits 1 on a difference or a disallowed path.
//!
//! `cargo run -p xtask -- package-list` runs `cargo package -p agentcat-sdk --list` and exits 1
//! if the list holds a file outside [`check_package_list`]'s set.
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

/// The crates that may appear in a public signature (C51). `http` is not here: the current API
/// does not use it, so a signature that starts to use it is a finding.
pub const ALLOWED_CRATES: &[&str] = &["std", "core", "alloc", "agentcat_sdk", "rmcp", "serde_json"];

/// Where the committed snapshot lives, relative to the workspace root.
pub const SNAPSHOT: &str = "agentcat-sdk/public-api.txt";

/// Files outside `src/` and `examples/` that the package may hold.
const PACKAGE_FILES: &[&str] = &[
    "build.rs",
    "README.md",
    "CHANGELOG.md",
    "LICENSE",
    "Cargo.toml",
    "Cargo.toml.orig",
    ".cargo_vcs_info.json",
    "Cargo.lock",
];

/// The crate roots of every path in `text` (`a::b::c` has the root `a`) that are not allowed.
/// A root that starts with an uppercase letter (`Self`, a generic parameter) is not a crate.
#[must_use]
pub fn disallowed_crates(text: &str) -> BTreeSet<String> {
    let bytes = text.as_bytes();
    let is_ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut found = BTreeSet::new();
    let mut i = 0;
    while i < bytes.len() {
        if !is_ident(bytes[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && is_ident(bytes[i]) {
            i += 1;
        }
        // A path root is not preceded by `::`; the segments after the root are skipped here.
        let after_path = start >= 2 && &bytes[start - 2..start] == b"::";
        let word = &text[start..i];
        let starts_lower = word.starts_with(|c: char| c.is_ascii_lowercase() || c == '_');
        if !after_path
            && starts_lower
            && text[i..].starts_with("::")
            && !ALLOWED_CRATES.contains(&word)
        {
            found.insert(word.to_owned());
        }
    }
    found
}

/// A line diff of `expected` (the snapshot) against `actual`; empty when they are equal.
/// The format is a unified diff with no context lines.
#[must_use]
pub fn diff(expected: &str, actual: &str) -> String {
    let old: Vec<&str> = expected.lines().collect();
    let new: Vec<&str> = actual.lines().collect();
    // Longest common subsequence table, filled from the end.
    let mut lcs = vec![vec![0_usize; new.len() + 1]; old.len() + 1];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            lcs[i][j] = if old[i] == new[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let mut out = String::new();
    let (mut i, mut j) = (0, 0);
    let mut in_hunk = false;
    while i < old.len() || j < new.len() {
        if i < old.len() && j < new.len() && old[i] == new[j] {
            i += 1;
            j += 1;
            in_hunk = false;
            continue;
        }
        if !in_hunk {
            let _ = writeln!(out, "@@ -{} +{} @@", i + 1, j + 1);
            in_hunk = true;
        }
        if j >= new.len() || (i < old.len() && lcs[i + 1][j] >= lcs[i][j + 1]) {
            let _ = writeln!(out, "-{}", old[i]);
            i += 1;
        } else {
            let _ = writeln!(out, "+{}", new[j]);
            j += 1;
        }
    }
    if expected.ends_with('\n') != actual.ends_with('\n') && out.is_empty() {
        out.push_str("@@ the final newline differs @@\n");
    }
    out
}

/// The files of a `cargo package --list` output that the package must not hold.
#[must_use]
pub fn check_package_list(list: &str) -> Vec<String> {
    list.lines()
        .map(|l| l.trim().replace('\\', "/"))
        .filter(|l| !l.is_empty())
        .filter(|l| {
            !(l.starts_with("src/")
                || l.starts_with("examples/")
                || PACKAGE_FILES.contains(&l.as_str()))
        })
        .map(|l| format!("package-list: unexpected file in the package: {l}"))
        .collect()
}

/// Runs `public-api [--bless]`; returns the exit code and the lines to print.
#[must_use]
pub fn run(args: &[String], root: &Path) -> (u8, Vec<String>) {
    let bless = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] => false,
        ["--bless"] => true,
        _ => {
            return (
                2,
                vec!["usage: cargo run -p xtask -- public-api [--bless]".to_owned()],
            );
        }
    };
    match public_api(root, bless) {
        Ok(lines) => (u8::from(!lines.is_empty()), lines),
        Err(message) => (1, vec![format!("xtask public-api: {message}")]),
    }
}

/// Runs `package-list`; returns the exit code and the lines to print.
#[must_use]
pub fn run_package_list(args: &[String], root: &Path) -> (u8, Vec<String>) {
    if !args.is_empty() {
        return (
            2,
            vec!["usage: cargo run -p xtask -- package-list".to_owned()],
        );
    }
    let output = Command::new("cargo")
        .args(["package", "-p", "agentcat-sdk", "--list", "--allow-dirty"])
        .current_dir(root)
        .output();
    match output {
        Ok(o) if o.status.success() => {
            let problems = check_package_list(&String::from_utf8_lossy(&o.stdout));
            (u8::from(!problems.is_empty()), problems)
        }
        Ok(o) => (
            1,
            vec![format!(
                "xtask package-list: `cargo package --list` failed ({}):\n{}",
                o.status,
                String::from_utf8_lossy(&o.stderr)
            )],
        ),
        Err(e) => (
            1,
            vec![format!("xtask package-list: cannot run cargo: {e}")],
        ),
    }
}

/// The problems of the generated API against the snapshot; empty when the gate passes.
fn public_api(root: &Path, bless: bool) -> Result<Vec<String>, String> {
    let output = Command::new("cargo")
        .args([
            "+nightly",
            "public-api",
            "-p",
            "agentcat-sdk",
            "--all-features",
            "-ss",
        ])
        .current_dir(root)
        .output()
        .map_err(|e| format!("cannot run `cargo +nightly public-api`: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "`cargo +nightly public-api` failed ({}):\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let actual = String::from_utf8(output.stdout)
        .map_err(|e| format!("`cargo public-api` printed invalid UTF-8: {e}"))?;
    let mut problems: Vec<String> = disallowed_crates(&actual)
        .into_iter()
        .map(|c| {
            format!(
                "public-api: the public API uses the crate `{c}`; allowed: {}",
                ALLOWED_CRATES.join(", ")
            )
        })
        .collect();
    let path = root.join(SNAPSHOT);
    if bless {
        if problems.is_empty() {
            std::fs::write(&path, &actual).map_err(|e| format!("cannot write {SNAPSHOT}: {e}"))?;
        }
        return Ok(problems);
    }
    let expected = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read {SNAPSHOT} (run with --bless): {e}"))?;
    let changes = diff(&expected, &actual);
    if !changes.is_empty() {
        problems.push(format!(
            "public-api: {SNAPSHOT} differs from the generated API (bless it with `cargo run -p xtask -- public-api --bless` if the change is intended):\n--- {SNAPSHOT}\n+++ generated\n{}",
            changes.trim_end()
        ));
    }
    Ok(problems)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_accepts_rmcp_and_std() {
        let text = "pub fn agentcat_sdk::AgentCat::track<H: rmcp::handler::server::ServerHandler>(H) -> core::result::Result<std::time::SystemTime, alloc::string::String>\npub fn x(&self) -> serde_json::value::Value where Self::Item: Send";
        assert_eq!(disallowed_crates(text), BTreeSet::new());
    }

    #[test]
    fn allowlist_rejects_reqwest_tokio_chrono_agentcat_api() {
        let text = "pub fn a(reqwest::Client)\npub fn b(&self) -> tokio::runtime::Handle\npub fn c(chrono::DateTime<chrono::Utc>)\npub fn d(agentcat_api::models::Event)\npub fn e(http::request::Parts)";
        let found: Vec<String> = disallowed_crates(text).into_iter().collect();
        assert_eq!(
            found,
            ["agentcat_api", "chrono", "http", "reqwest", "tokio"]
        );
    }

    #[test]
    fn diff_reports_changed_line() {
        let out = diff("a\nb\nc\n", "a\nB\nc\n");
        assert_eq!(out, "@@ -2 +2 @@\n-b\n+B\n");
        assert_eq!(diff("a\nb\n", "a\nb\n"), "");
    }

    #[test]
    fn diff_reports_added_and_removed_lines() {
        assert_eq!(diff("a\n", "a\nb\n"), "@@ -2 +2 @@\n+b\n");
        assert_eq!(diff("a\nb\n", "b\n"), "@@ -1 +1 @@\n-a\n");
    }

    #[test]
    fn package_list_accepts_the_expected_files() {
        let list = "Cargo.lock\nCargo.toml\nCargo.toml.orig\nLICENSE\nREADME.md\nCHANGELOG.md\nbuild.rs\n.cargo_vcs_info.json\nsrc/lib.rs\nexamples/stdio.rs\n";
        assert_eq!(check_package_list(list), Vec::<String>::new());
    }

    #[test]
    fn package_list_rejects_other_files() {
        let list = "Cargo.toml\nsrc/lib.rs\ntests/x.rs\npublic-api.txt\n";
        assert_eq!(
            check_package_list(list),
            [
                "package-list: unexpected file in the package: tests/x.rs",
                "package-list: unexpected file in the package: public-api.txt"
            ]
        );
    }
}

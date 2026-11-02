//! The release steps of `.github/workflows/release.yml`, on `agentcat-sdk/Cargo.toml`,
//! `agentcat-sdk/CHANGELOG.md` and the `v*` tags of the repository.
//!
//! - `release prepare <patch | minor | major | X.Y.Z> <YYYY-MM-DD>` picks the next version,
//!   writes it into `Cargo.toml`, turns the `[Unreleased]` section of the CHANGELOG into the
//!   section of that version with the date, and prints the version. `patch`, `minor` and `major`
//!   count from the newest `v*` tag. With no tag yet, the first release is the version that
//!   `Cargo.toml` holds, whatever the input word. An exact version must be newer than the newest
//!   tag. An `[Unreleased]` section with no entry stops the step: a release needs a change.
//! - `release status` prints `<version> publish` when the version of `Cargo.toml` has no tag and
//!   the CHANGELOG has its dated section, else `<version> skip`.
//! - `release notes <X.Y.Z>` prints the body of the CHANGELOG section of that version.
use std::cmp::Ordering;
use std::fmt;
use std::fs;
use std::path::Path;
use std::process::Command;

const USAGE: &str = "usage: cargo run -q -p xtask -- release prepare <patch | minor | major | X.Y.Z> <YYYY-MM-DD> | release status | release notes <X.Y.Z>";

/// The repository URL that the CHANGELOG links point to.
const REPOSITORY: &str = "https://github.com/agentcathq/agentcat-rust-sdk";

const MANIFEST: &str = "agentcat-sdk/Cargo.toml";
const CHANGELOG: &str = "agentcat-sdk/CHANGELOG.md";
const UNRELEASED: &str = "## [Unreleased]";

/// A release version: `MAJOR.MINOR.PATCH`, with no pre-release or build part.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Version {
    major: u64,
    minor: u64,
    patch: u64,
}

impl Version {
    /// `MAJOR.MINOR.PATCH` with decimal numbers and no leading zero.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let mut parts = text.split('.').map(|part| {
            let valid = !part.is_empty()
                && part.bytes().all(|b| b.is_ascii_digit())
                && (part == "0" || !part.starts_with('0'));
            valid.then(|| part.parse::<u64>().ok()).flatten()
        });
        let version = Self {
            major: parts.next()??,
            minor: parts.next()??,
            patch: parts.next()??,
        };
        parts.next().is_none().then_some(version)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// The newest of the `v<version>` tags; other tags do not count.
pub(crate) fn newest_tag(tags: &[String]) -> Option<Version> {
    tags.iter()
        .filter_map(|tag| tag.strip_prefix('v').and_then(Version::parse))
        .max()
}

/// The version of the release that `input` asks for: see the module documentation.
pub(crate) fn next_version(
    input: &str,
    current: Version,
    newest: Option<Version>,
) -> Result<Version, String> {
    let Some(newest) = newest else {
        return match Version::parse(input) {
            Some(exact) if exact != current => Err(format!(
                "the first release is the version of {MANIFEST}, {current}; set it there to release {exact}"
            )),
            Some(_) => Ok(current),
            None if ["patch", "minor", "major"].contains(&input) => Ok(current),
            None => Err(format!(
                "{input:?} is not patch, minor, major or a version X.Y.Z"
            )),
        };
    };
    let next = match input {
        "patch" => Version {
            patch: newest.patch + 1,
            ..newest
        },
        "minor" => Version {
            minor: newest.minor + 1,
            patch: 0,
            ..newest
        },
        "major" => Version {
            major: newest.major + 1,
            minor: 0,
            patch: 0,
        },
        other => Version::parse(other)
            .ok_or_else(|| format!("{other:?} is not patch, minor, major or a version X.Y.Z"))?,
    };
    match next.cmp(&newest) {
        Ordering::Greater => Ok(next),
        _ => Err(format!(
            "{next} is not newer than the newest tag, v{newest}"
        )),
    }
}

/// The `version` of the `[package]` table of `manifest`.
pub(crate) fn manifest_version(manifest: &str) -> Option<Version> {
    package_version_line(manifest).and_then(|(_, value)| Version::parse(value))
}

/// `manifest` with `version` as the version of its `[package]` table.
pub(crate) fn set_manifest_version(manifest: &str, version: Version) -> Result<String, String> {
    let (index, _) = package_version_line(manifest)
        .ok_or_else(|| format!("{MANIFEST} has no version line in [package]"))?;
    let mut lines: Vec<String> = manifest.lines().map(str::to_owned).collect();
    lines[index] = format!("version = \"{version}\"");
    let mut text = lines.join("\n");
    if manifest.ends_with('\n') {
        text.push('\n');
    }
    Ok(text)
}

/// The index and the quoted value of the `version = "..."` line of the `[package]` table.
fn package_version_line(manifest: &str) -> Option<(usize, &str)> {
    let mut in_package = false;
    for (index, line) in manifest.lines().enumerate() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
        } else if in_package
            && let Some(value) = line
                .strip_prefix("version = \"")
                .and_then(|rest| rest.strip_suffix('"'))
        {
            return Some((index, value));
        }
    }
    None
}

/// `changelog` with its `[Unreleased]` entries moved under `## [<version>] - <date>`, an empty
/// `[Unreleased]` section above it, and the links of both sections.
pub(crate) fn date_changelog(
    changelog: &str,
    version: Version,
    date: &str,
    previous: Option<Version>,
) -> Result<String, String> {
    if !is_date(date) {
        return Err(format!("{date:?} is not a date YYYY-MM-DD"));
    }
    let lines: Vec<&str> = changelog.lines().collect();
    let start = lines
        .iter()
        .position(|line| *line == UNRELEASED)
        .ok_or_else(|| format!("{CHANGELOG} has no {UNRELEASED} section"))?;
    let end = section_end(&lines, start);
    if !lines[start + 1..end]
        .iter()
        .any(|line| line.starts_with("- "))
    {
        return Err(format!(
            "the {UNRELEASED} section of {CHANGELOG} has no entry; add the changes of the release first"
        ));
    }
    let link = |label: &str| format!("[{label}]: ");
    let unreleased_link = lines
        .iter()
        .position(|line| line.starts_with(&link("Unreleased")))
        .ok_or_else(|| format!("{CHANGELOG} has no [Unreleased] link"))?;
    let version_link = match previous {
        Some(previous) => format!("[{version}]: {REPOSITORY}/compare/v{previous}...v{version}"),
        None => format!("[{version}]: {REPOSITORY}/releases/tag/v{version}"),
    };
    let mut out: Vec<String> = Vec::with_capacity(lines.len() + 4);
    for (index, line) in lines.iter().enumerate() {
        if index == start {
            out.push(UNRELEASED.to_owned());
            out.push(String::new());
            out.push(format!("## [{version}] - {date}"));
        } else if index == unreleased_link {
            out.push(format!(
                "[Unreleased]: {REPOSITORY}/compare/v{version}...HEAD"
            ));
            out.push(version_link.clone());
        } else {
            out.push((*line).to_owned());
        }
    }
    Ok(out.join("\n") + "\n")
}

/// The date and the body of the `## [<version>] - <date>` section, without its blank edges.
pub(crate) fn dated_section(changelog: &str, version: Version) -> Option<(String, String)> {
    let lines: Vec<&str> = changelog.lines().collect();
    let heading = format!("## [{version}] - ");
    let start = lines.iter().position(|line| line.starts_with(&heading))?;
    let date = lines[start][heading.len()..].trim();
    if !is_date(date) {
        return None;
    }
    let body = lines[start + 1..section_end(&lines, start)].join("\n");
    Some((date.to_owned(), body.trim_matches('\n').to_owned()))
}

/// The index of the line after the section that starts at `start`: the next `## ` heading or
/// the first link line.
fn section_end(lines: &[&str], start: usize) -> usize {
    lines[start + 1..]
        .iter()
        .position(|line| line.starts_with("## ") || (line.starts_with('[') && line.contains("]: ")))
        .map_or(lines.len(), |offset| start + 1 + offset)
}

fn is_date(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 10
        && bytes.iter().enumerate().all(|(index, b)| match index {
            4 | 7 => *b == b'-',
            _ => b.is_ascii_digit(),
        })
}

/// Runs `release <args>` in the workspace `root`: the exit code, the lines for stdout and the
/// problems for stderr.
pub fn run(args: &[String], root: &Path) -> (u8, Vec<String>, Vec<String>) {
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        ["prepare", input, date] => prepare(root, input, date),
        ["status"] => status(root),
        ["notes", version] => notes(root, version),
        _ => return (2, Vec::new(), vec![USAGE.to_owned()]),
    };
    match result {
        Ok(lines) => (0, lines, Vec::new()),
        Err(problem) => (1, Vec::new(), vec![format!("release: {problem}")]),
    }
}

fn prepare(root: &Path, input: &str, date: &str) -> Result<Vec<String>, String> {
    let manifest = read(root, MANIFEST)?;
    let current =
        manifest_version(&manifest).ok_or_else(|| format!("{MANIFEST} has no valid version"))?;
    let newest = newest_tag(&tags(root)?);
    let version = next_version(input, current, newest)?;
    let changelog = date_changelog(&read(root, CHANGELOG)?, version, date, newest)?;
    if version != current {
        write(root, MANIFEST, &set_manifest_version(&manifest, version)?)?;
    }
    write(root, CHANGELOG, &changelog)?;
    Ok(vec![version.to_string()])
}

fn status(root: &Path) -> Result<Vec<String>, String> {
    let version = manifest_version(&read(root, MANIFEST)?)
        .ok_or_else(|| format!("{MANIFEST} has no valid version"))?;
    let tagged = tags(root)?.contains(&format!("v{version}"));
    let dated = dated_section(&read(root, CHANGELOG)?, version).is_some();
    let action = if !tagged && dated { "publish" } else { "skip" };
    Ok(vec![format!("{version} {action}")])
}

fn notes(root: &Path, version: &str) -> Result<Vec<String>, String> {
    let version =
        Version::parse(version).ok_or_else(|| format!("{version:?} is not a version X.Y.Z"))?;
    let (_, body) = dated_section(&read(root, CHANGELOG)?, version)
        .ok_or_else(|| format!("{CHANGELOG} has no dated section for {version}"))?;
    Ok(vec![body])
}

/// The tags of the repository at `root`.
fn tags(root: &Path) -> Result<Vec<String>, String> {
    let output = Command::new("git")
        .args(["tag", "--list"])
        .current_dir(root)
        .output()
        .map_err(|e| format!("cannot run git: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "git tag --list failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_owned)
        .collect())
}

fn read(root: &Path, path: &str) -> Result<String, String> {
    fs::read_to_string(root.join(path)).map_err(|e| format!("cannot read {path}: {e}"))
}

fn write(root: &Path, path: &str, text: &str) -> Result<(), String> {
    fs::write(root.join(path), text).map_err(|e| format!("cannot write {path}: {e}"))
}

#[cfg(test)]
mod tests;

//! The line coverage floors (decision C52).
//!
//! `cargo run -p xtask -- coverage <summary.json>` reads the `--json --summary-only` output of
//! `cargo llvm-cov nextest -p agentcat-sdk --all-features --ignore-filename-regex
//! '(/tests\.rs$|/test_support/|/tests/)'`. It prints a table of the crate's files (lines run,
//! lines total, percent) with the totals of the crate and of the pure set ([`PURE_SET`]), and
//! exits 1 when the crate runs less than [`CRATE_FLOOR`] percent of its lines or the pure set
//! less than [`PURE_FLOOR`] percent of its lines, naming the set and, for the pure set, its files
//! below the floor.
//!
//! The pure floor applies to the set's total, not to each file: a small file with one unreachable
//! line would otherwise need a test of its own. Files outside `agentcat-sdk/src/` and the test
//! files that the regex names (which the summary should not hold) are left out.
use std::fmt::Write as _;

use serde::Deserialize;

/// The lowest share of the crate's lines, in percent, that the tests must run.
pub const CRATE_FLOOR: u64 = 90;

/// The lowest share of the pure set's lines ([`PURE_SET`]), in percent, that the tests must run.
pub const PURE_FLOOR: u64 = 95;

/// The pure set of C52: the modules whose work is mostly value in, value out, so tests reach
/// nearly all of it. Paths under `agentcat-sdk/src/`; an entry that ends in `/` is every file
/// under that directory.
pub const PURE_SET: &[&str] = &[
    "constants.rs",
    "js.rs",
    "handles/",
    "tokens.rs",
    "inject.rs",
    "pipeline/tags.rs",
    "pipeline/normalize.rs",
    "pipeline/truncate.rs",
    "pipeline/sanitize.rs",
    "pipeline/redact.rs",
];

/// The crate's source directory, as it appears in the file names of the summary.
const CRATE_SRC: &str = "agentcat-sdk/src/";

/// The `llvm-cov export` JSON, as far as the floors need it.
#[derive(Deserialize)]
struct Export {
    data: Vec<ExportData>,
}

#[derive(Deserialize)]
struct ExportData {
    files: Vec<ExportFile>,
}

#[derive(Deserialize)]
struct ExportFile {
    filename: String,
    summary: Summary,
}

#[derive(Deserialize)]
struct Summary {
    lines: Lines,
}

/// Lines run (`covered`) of lines with code (`count`).
#[derive(Deserialize, Clone, Copy, Default)]
struct Lines {
    count: u64,
    covered: u64,
}

impl Lines {
    fn add(&mut self, other: Self) {
        self.count += other.count;
        self.covered += other.covered;
    }

    /// `true` when less than `floor` percent of the lines ran (exact, with no rounding).
    fn below(self, floor: u64) -> bool {
        self.covered * 100 < floor * self.count
    }

    /// The percent, cut (not rounded) to one decimal place, so a total below a floor never shows
    /// as the floor; `-` for no lines.
    fn percent(self) -> String {
        if self.count == 0 {
            return "-".to_owned();
        }
        let tenths = self.covered * 1000 / self.count;
        format!("{}.{} %", tenths / 10, tenths % 10)
    }
}

/// A file of the crate: its path under [`CRATE_SRC`] and its lines.
struct CrateFile {
    path: String,
    lines: Lines,
    pure: bool,
}

/// What [`check`] found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Outcome {
    /// The table of the crate's files and the two totals.
    pub table: String,
    /// One line for each floor the report is below, and for each pure set entry with no file.
    pub problems: Vec<String>,
}

/// The path of `filename` under [`CRATE_SRC`], or `None` for a file outside the crate or a test
/// file (`tests.rs`, `test_support/`, `tests/`: the files the regex of C52 names).
fn crate_path(filename: &str) -> Option<String> {
    let name = filename.replace('\\', "/");
    let at = name.rfind(CRATE_SRC)?;
    if at > 0 && !name[..at].ends_with('/') {
        return None;
    }
    let path = &name[at + CRATE_SRC.len()..];
    let slashed = format!("/{path}");
    let test = slashed.ends_with("/tests.rs")
        || slashed.contains("/test_support/")
        || slashed.contains("/tests/");
    (!test).then(|| path.to_owned())
}

/// `true` when `path` (under [`CRATE_SRC`]) is in the pure set entry `entry`.
fn in_entry(path: &str, entry: &str) -> bool {
    if entry.ends_with('/') {
        path.starts_with(entry)
    } else {
        path == entry
    }
}

/// Checks the floors on the text of an llvm-cov summary.
///
/// # Errors
///
/// When the text is not an llvm-cov summary, or it holds no file of the crate (a wrong path or
/// regex would otherwise pass every floor).
pub fn check(json: &str) -> Result<Outcome, String> {
    let export: Export =
        serde_json::from_str(json).map_err(|e| format!("not an llvm-cov summary: {e}"))?;
    let mut files: Vec<CrateFile> = export
        .data
        .into_iter()
        .flat_map(|data| data.files)
        .filter_map(|file| {
            let path = crate_path(&file.filename)?;
            let pure = PURE_SET.iter().any(|entry| in_entry(&path, entry));
            Some(CrateFile {
                path,
                lines: file.summary.lines,
                pure,
            })
        })
        .collect();
    if files.is_empty() {
        return Err(format!("the report holds no file of {CRATE_SRC}"));
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let (mut all, mut pure) = (Lines::default(), Lines::default());
    for file in &files {
        all.add(file.lines);
        if file.pure {
            pure.add(file.lines);
        }
    }
    let mut problems: Vec<String> = PURE_SET
        .iter()
        .filter(|entry| !files.iter().any(|file| in_entry(&file.path, entry)))
        .map(|entry| format!("coverage: the pure set entry {entry} matches no file of the report"))
        .collect();
    if all.below(CRATE_FLOOR) {
        problems.push(format!(
            "coverage: the crate runs {} of its lines ({} of {}), below its floor of {CRATE_FLOOR} %",
            all.percent(),
            all.covered,
            all.count
        ));
    }
    if pure.below(PURE_FLOOR) {
        let low: Vec<String> = files
            .iter()
            .filter(|file| file.pure && file.lines.below(PURE_FLOOR))
            .map(|file| format!("{} ({})", file.path, file.lines.percent()))
            .collect();
        problems.push(format!(
            "coverage: the pure set runs {} of its lines ({} of {}), below its floor of {PURE_FLOOR} %; its files below {PURE_FLOOR} %: {}",
            pure.percent(),
            pure.covered,
            pure.count,
            low.join(", ")
        ));
    }
    let mut rows: Vec<(String, Lines)> = files
        .iter()
        .map(|file| {
            let name = if file.pure {
                format!("{} (pure)", file.path)
            } else {
                file.path.clone()
            };
            (name, file.lines)
        })
        .collect();
    rows.push(("crate".to_owned(), all));
    rows.push(("pure set".to_owned(), pure));
    Ok(Outcome {
        table: table(&rows),
        problems,
    })
}

/// The rows as a table under a header, the names padded to the longest.
fn table(rows: &[(String, Lines)]) -> String {
    let width = rows
        .iter()
        .map(|(name, _)| name.len())
        .chain(std::iter::once("file".len()))
        .max()
        .unwrap_or_default();
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{:<width$}  {:>7}  {:>7}  {:>7}",
        "file", "covered", "total", "percent"
    );
    for (name, lines) in rows {
        let _ = writeln!(
            out,
            "{name:<width$}  {:>7}  {:>7}  {:>7}",
            lines.covered,
            lines.count,
            lines.percent()
        );
    }
    out
}

/// Runs `coverage <summary.json>`; returns the exit code, the table (for stdout) and the
/// problems (for stderr).
#[must_use]
pub fn run(args: &[String]) -> (u8, String, Vec<String>) {
    let [path] = args else {
        return (
            2,
            String::new(),
            vec!["usage: cargo run -p xtask -- coverage <llvm-cov summary json>".to_owned()],
        );
    };
    let outcome = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {path}: {e}"))
        .and_then(|json| check(&json).map_err(|e| format!("{path}: {e}")));
    match outcome {
        Ok(Outcome { table, problems }) => (u8::from(!problems.is_empty()), table, problems),
        Err(message) => (1, String::new(), vec![format!("xtask coverage: {message}")]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The summary of llvm-cov for `files` (path, lines covered, lines total), in its full shape.
    fn summary(files: &[(&str, u64, u64)]) -> String {
        let counts = |covered: u64, count: u64| {
            let float = |n: u64| f64::from(u32::try_from(n).unwrap());
            let percent = if count == 0 {
                0.0
            } else {
                100.0 * float(covered) / float(count)
            };
            serde_json::json!({"count": count, "covered": covered, "percent": percent})
        };
        let files: Vec<serde_json::Value> = files
            .iter()
            .map(|&(path, covered, count)| {
                serde_json::json!({
                    "filename": format!("/home/runner/work/sdk/sdk/{path}"),
                    "summary": {
                        "branches": counts(0, 0),
                        "functions": counts(1, 1),
                        "instantiations": counts(1, 1),
                        "lines": counts(covered, count),
                        "mcdc": counts(0, 0),
                        "regions": counts(covered, count),
                    },
                })
            })
            .collect();
        serde_json::json!({
            "data": [{"files": files, "totals": {"lines": counts(0, 0)}}],
            "type": "llvm.coverage.json.export",
            "version": "3.0.1",
            "cargo_llvm_cov": {"version": "0.9.1", "manifest_path": "/home/runner/work/sdk/sdk/Cargo.toml"},
        })
        .to_string()
    }

    /// One file for each entry of the pure set, each with `covered` of 100 lines run; the
    /// `handles/` entry gets `handles/mod.rs`.
    fn pure_files(covered: u64) -> Vec<(String, u64, u64)> {
        PURE_SET
            .iter()
            .map(|entry| {
                let path = if entry.ends_with('/') {
                    format!("agentcat-sdk/src/{entry}mod.rs")
                } else {
                    format!("agentcat-sdk/src/{entry}")
                };
                (path, covered, 100)
            })
            .collect()
    }

    /// [`check`] on the summary of `pure` and `rest`.
    fn check_files(pure: &[(String, u64, u64)], rest: &[(&str, u64, u64)]) -> Outcome {
        let mut files: Vec<(&str, u64, u64)> = pure
            .iter()
            .map(|(path, covered, count)| (path.as_str(), *covered, *count))
            .collect();
        files.extend_from_slice(rest);
        check(&summary(&files)).unwrap()
    }

    #[test]
    fn crate_floor_fails_at_89_9() {
        // The pure set: 1000 of 1000 lines; with client.rs, 1798 of 2000 lines are 89.9 %.
        let outcome = check_files(
            &pure_files(100),
            &[("agentcat-sdk/src/client.rs", 798, 1000)],
        );
        assert_eq!(
            outcome.problems,
            [
                "coverage: the crate runs 89.9 % of its lines (1798 of 2000), below its floor of 90 %"
            ]
        );
        // 1800 of 2000 lines are 90.0 %: the floor passes.
        let outcome = check_files(
            &pure_files(100),
            &[("agentcat-sdk/src/client.rs", 800, 1000)],
        );
        assert_eq!(outcome.problems, Vec::<String>::new());
    }

    #[test]
    fn crate_floor_compares_exact_counts() {
        // 8999 of 10000 lines are 89.99 %: below the floor, though one decimal place rounds it up.
        let outcome = check_files(
            &pure_files(100),
            &[("agentcat-sdk/src/client.rs", 7999, 9000)],
        );
        assert_eq!(
            outcome.problems,
            [
                "coverage: the crate runs 89.9 % of its lines (8999 of 10000), below its floor of 90 %"
            ]
        );
    }

    #[test]
    fn pure_floor_fails_at_94_9() {
        // The pure set: nine files at 100 of 100 and js.rs at 49 of 100, 949 of 1000 lines; the
        // crate is at 1949 of 2000 lines (97.4 %).
        let mut pure = pure_files(100);
        js_lines(&mut pure, 49);
        let outcome = check_files(&pure, &[("agentcat-sdk/src/client.rs", 1000, 1000)]);
        assert_eq!(
            outcome.problems,
            [
                "coverage: the pure set runs 94.9 % of its lines (949 of 1000), below its floor of 95 %; its files below 95 %: js.rs (49.0 %)"
            ]
        );
        // 950 of 1000 lines are 95.0 %: the floor passes, though js.rs alone is below it.
        js_lines(&mut pure, 50);
        let outcome = check_files(&pure, &[("agentcat-sdk/src/client.rs", 1000, 1000)]);
        assert_eq!(outcome.problems, Vec::<String>::new());
    }

    /// Sets the lines run of js.rs in `pure`.
    fn js_lines(pure: &mut [(String, u64, u64)], covered: u64) {
        for file in pure
            .iter_mut()
            .filter(|(path, _, _)| path.ends_with("/js.rs"))
        {
            file.1 = covered;
        }
    }

    #[test]
    fn test_files_are_ignored() {
        // Test files that ran none of their lines, one of them under the pure directory handles/.
        let outcome = check_files(
            &pure_files(100),
            &[
                ("agentcat-sdk/src/js/tests.rs", 0, 1000),
                ("agentcat-sdk/src/handles/tests.rs", 0, 1000),
                ("agentcat-sdk/src/handles/ksuid/tests.rs", 0, 1000),
                ("agentcat-sdk/src/test_support/oracle.rs", 0, 1000),
                ("agentcat-sdk/src/pipeline/truncate/tests/cut.rs", 0, 1000),
                ("agentcat-sdk/tests/fake.rs", 0, 1000),
            ],
        );
        assert_eq!(outcome.problems, Vec::<String>::new());
        assert!(!outcome.table.contains("tests"), "{}", outcome.table);
        assert!(!outcome.table.contains("test_support"), "{}", outcome.table);
        assert_eq!(row(&outcome.table, "crate"), ["1000", "1000", "100.0", "%"]);
        assert_eq!(
            row(&outcome.table, "handles/mod.rs (pure)"),
            ["100", "100", "100.0", "%"]
        );
    }

    /// The fields after `name` on the table row of `name`.
    fn row<'a>(table: &'a str, name: &str) -> Vec<&'a str> {
        let line = table
            .lines()
            .find(|line| {
                line.strip_prefix(name)
                    .is_some_and(|rest| rest.starts_with("  "))
            })
            .unwrap_or_else(|| panic!("no row {name} in\n{table}"));
        line[name.len()..].split_whitespace().collect()
    }

    #[test]
    fn files_outside_the_crate_are_ignored() {
        let outcome = check_files(
            &pure_files(100),
            &[
                ("xtask/src/main.rs", 0, 1000),
                ("harness/rust-server/src/main.rs", 0, 1000),
                (".cargo/registry/src/index/rmcp-3.1.0/src/lib.rs", 0, 1000),
                ("my-agentcat-sdk/src/lib.rs", 0, 1000),
            ],
        );
        assert_eq!(outcome.problems, Vec::<String>::new());
        assert!(!outcome.table.contains("main.rs"), "{}", outcome.table);
        assert_eq!(row(&outcome.table, "crate"), ["1000", "1000", "100.0", "%"]);
    }

    #[test]
    fn windows_separators_are_read() {
        let outcome = check_files(
            &pure_files(100),
            &[("agentcat-sdk\\src\\client.rs", 0, 1000)],
        );
        assert_eq!(row(&outcome.table, "client.rs"), ["0", "1000", "0.0", "%"]);
    }

    #[test]
    fn table_lists_each_file_and_both_totals() {
        let outcome = check_files(&pure_files(100), &[("agentcat-sdk/src/client.rs", 2, 3)]);
        let expected = "\
file                          covered    total  percent
client.rs                           2        3   66.6 %
constants.rs (pure)               100      100  100.0 %
handles/mod.rs (pure)             100      100  100.0 %
inject.rs (pure)                  100      100  100.0 %
js.rs (pure)                      100      100  100.0 %
pipeline/normalize.rs (pure)      100      100  100.0 %
pipeline/redact.rs (pure)         100      100  100.0 %
pipeline/sanitize.rs (pure)       100      100  100.0 %
pipeline/tags.rs (pure)           100      100  100.0 %
pipeline/truncate.rs (pure)       100      100  100.0 %
tokens.rs (pure)                  100      100  100.0 %
crate                            1002     1003   99.9 %
pure set                         1000     1000  100.0 %
";
        assert_eq!(outcome.table, expected);
    }

    #[test]
    fn file_without_lines_shows_no_percent() {
        let outcome = check_files(&pure_files(100), &[("agentcat-sdk/src/error.rs", 0, 0)]);
        assert_eq!(row(&outcome.table, "error.rs"), ["0", "0", "-"]);
        assert_eq!(outcome.problems, Vec::<String>::new());
    }

    #[test]
    fn pure_entry_without_a_file_is_a_problem() {
        let pure: Vec<_> = pure_files(100)
            .into_iter()
            .filter(|(path, _, _)| !path.ends_with("/tags.rs"))
            .collect();
        let outcome = check_files(&pure, &[]);
        assert_eq!(
            outcome.problems,
            ["coverage: the pure set entry pipeline/tags.rs matches no file of the report"]
        );
    }

    #[test]
    fn report_without_crate_files_is_an_error() {
        let error = check(&summary(&[("xtask/src/main.rs", 1, 1)])).unwrap_err();
        assert_eq!(error, "the report holds no file of agentcat-sdk/src/");
        assert!(check("{}").is_err());
    }

    #[test]
    fn run_takes_one_path() {
        let (code, _, problems) = run(&[]);
        assert_eq!(code, 2);
        assert_eq!(
            problems,
            ["usage: cargo run -p xtask -- coverage <llvm-cov summary json>"]
        );
        let (code, _, problems) = run(&["/nonexistent/coverage.json".to_owned()]);
        assert_eq!(code, 1);
        assert!(
            problems[0].starts_with("xtask coverage: cannot read /nonexistent/coverage.json: "),
            "{problems:?}"
        );
    }
}

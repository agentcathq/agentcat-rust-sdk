//! `scripts/mutants-base.sh`, the base commit of the mutation diff gate. Each test copies the
//! script into a scratch git repository under a temporary directory and runs it there: in CI mode
//! with a fake `gh` first on `PATH` (it answers from JSON files through `jq`, as `gh api --jq`
//! does), or in local mode. No test contacts the network or reads this checkout's history, and no
//! setting of the developer, the runner or a real CI run reaches git or the script: each command
//! starts from an empty environment.
#![cfg(unix)]
#![expect(clippy::expect_used, reason = "test helpers")]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

/// The run ID of the CI run the script runs in.
const RUN_ID: u64 = 999;

/// A commit that is in no scratch repository.
const UNKNOWN: &str = "0123456789abcdef0123456789abcdef01234567";

/// The fake `gh`: logs its arguments, fails when `FAKE_GH_FAIL` is set, else prints jq's output of
/// `$FAKE_GH_DIR/page-<page>.json` (an empty run list when the page has no file).
const FAKE_GH: &str = r#"#!/usr/bin/env bash
printf '%s\n' "$*" >> "$FAKE_GH_DIR/calls.log"
if [ -n "${FAKE_GH_FAIL:-}" ]; then
  echo "gh: HTTP 403: API rate limit exceeded" >&2
  exit 1
fi
page=1
expr=.
while [ $# -gt 0 ]; do
  case "$1" in
    -f) case "$2" in page=*) page="${2#page=}" ;; esac; shift 2 ;;
    --jq) expr="$2"; shift 2 ;;
    *) shift ;;
  esac
done
file="$FAKE_GH_DIR/page-$page.json"
if [ ! -f "$file" ]; then
  file="$FAKE_GH_DIR/empty.json"
  echo '{"workflow_runs":[]}' > "$file"
fi
jq -r "$expr" "$file"
"#;

/// A temporary directory, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static COUNT: AtomicUsize = AtomicUsize::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos());
        let path = std::env::temp_dir().join(format!(
            "mutants-base-{}-{}-{nanos}",
            std::process::id(),
            COUNT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("the scratch directory is created");
        // The canonical path: macOS reaches its temporary directory through a symbolic link.
        Self(path.canonicalize().expect("the scratch directory exists"))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The `PATH` of the test process, which finds git, bash and jq.
#[expect(
    clippy::disallowed_methods,
    reason = "the scripts under test need the tools of the test process"
)]
fn system_path() -> std::ffi::OsString {
    std::env::var_os("PATH").unwrap_or_default()
}

/// A command with an empty environment but `PATH`, a `HOME` in the scratch directory, an empty
/// global git configuration, no system one, and a fixed author.
fn clean_command(program: impl AsRef<std::ffi::OsStr>, scratch: &Path) -> Command {
    let config = scratch.join("no-gitconfig");
    if !config.exists() {
        fs::write(&config, "").expect("the empty git configuration is written");
    }
    let mut command = Command::new(program);
    command
        .env_clear()
        .env("PATH", system_path())
        .env("HOME", scratch)
        .env("GIT_CONFIG_GLOBAL", config)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com");
    command
}

/// A scratch repository: root A, then B and C on branch `work` (HEAD = C), and X on branch `side`
/// off A, which HEAD does not descend from. The script sits in its `scripts/` directory.
struct Repo {
    scratch: Scratch,
    dir: PathBuf,
    a: String,
    b: String,
    c: String,
    x: String,
}

impl Repo {
    fn new() -> Self {
        let scratch = Scratch::new();
        let dir = scratch.0.join("repo");
        fs::create_dir_all(dir.join("scripts")).expect("the repository directory is created");
        let git = |args: &[&str]| run_git(&scratch.0, &dir, args);
        let commit = |name: &str| {
            fs::write(dir.join(format!("{name}.txt")), name).expect("the file is written");
            git(&["add", &format!("{name}.txt")]);
            git(&["commit", "-q", "-m", name]);
            git(&["rev-parse", "HEAD"])
        };
        git(&["init", "-q", "-b", "work"]);
        let a = commit("A");
        git(&["checkout", "-q", "-b", "side"]);
        let x = commit("X");
        git(&["checkout", "-q", "work"]);
        let b = commit("B");
        let c = commit("C");
        let script = dir.join("scripts/mutants-base.sh");
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/mutants-base.sh");
        fs::copy(source, &script).expect("the script is copied");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755))
            .expect("the script is executable");
        Self {
            scratch,
            dir,
            a,
            b,
            c,
            x,
        }
    }

    /// Runs git in the repository; returns its trimmed stdout.
    fn git(&self, args: &[&str]) -> String {
        run_git(&self.scratch.0, &self.dir, args)
    }

    /// Runs the script in local mode.
    fn run_local(&self) -> Run {
        let output = clean_command(self.dir.join("scripts/mutants-base.sh"), &self.scratch.0)
            .current_dir(&self.dir)
            .output()
            .expect("the script runs");
        Run::new(&output, Vec::new())
    }

    /// Runs the script in CI mode; `pages` holds the runs of each page of the API answer, and
    /// `fail` makes the fake `gh` fail.
    fn run_ci(&self, pages: &[Vec<Value>], fail: bool) -> Run {
        let bin = self.scratch.0.join("bin");
        fs::create_dir_all(&bin).expect("the bin directory is created");
        fs::write(bin.join("gh"), FAKE_GH).expect("the fake gh is written");
        fs::set_permissions(bin.join("gh"), fs::Permissions::from_mode(0o755))
            .expect("the fake gh is executable");
        let gh_dir = self.dir.join(".gh");
        fs::create_dir_all(&gh_dir).expect("the gh directory is created");
        for (index, runs) in pages.iter().enumerate() {
            let page = json!({ "workflow_runs": runs });
            fs::write(
                gh_dir.join(format!("page-{}.json", index + 1)),
                page.to_string(),
            )
            .expect("the page is written");
        }
        let mut path = bin.into_os_string();
        path.push(":");
        path.push(system_path());
        let mut command = clean_command(self.dir.join("scripts/mutants-base.sh"), &self.scratch.0);
        command
            .current_dir(&self.dir)
            .env("PATH", path)
            .env("FAKE_GH_DIR", &gh_dir)
            .env("GITHUB_ACTIONS", "true")
            .env("GITHUB_REPOSITORY", "owner/repo")
            .env("GITHUB_RUN_ID", RUN_ID.to_string())
            .env("GITHUB_EVENT_NAME", "push")
            .env("GITHUB_REF_NAME", "work");
        if fail {
            command.env("FAKE_GH_FAIL", "1");
        }
        let output = command.output().expect("the script runs");
        let calls = fs::read_to_string(gh_dir.join("calls.log"))
            .map(|log| log.lines().map(str::to_owned).collect())
            .unwrap_or_default();
        Run::new(&output, calls)
    }
}

/// Runs git in `dir`; returns its trimmed stdout.
fn run_git(scratch: &Path, dir: &Path, args: &[&str]) -> String {
    let output = clean_command("git", scratch)
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

/// A run of the script.
struct Run {
    code: Option<i32>,
    stdout: String,
    stderr: String,
    /// The argument lines the fake `gh` logged.
    calls: Vec<String>,
}

impl Run {
    fn new(output: &Output, calls: Vec<String>) -> Self {
        Self {
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).trim().to_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            calls,
        }
    }
}

/// A run of the workflow, as the API lists it.
fn run(id: u64, sha: &str) -> Value {
    json!({"id": id, "head_sha": sha})
}

#[test]
fn ci_skips_this_run_a_non_ancestor_and_a_missing_commit_and_picks_the_next_ancestor() {
    let repo = Repo::new();
    // Newest first: this run (on HEAD itself), a green commit HEAD does not descend from, one the
    // clone lacks, then B.
    let page = vec![
        run(RUN_ID, &repo.c),
        run(5, &repo.x),
        run(4, UNKNOWN),
        run(3, &repo.b),
        run(2, &repo.a),
    ];
    let result = repo.run_ci(&[page], false);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(result.stdout, repo.b);
    assert_eq!(result.stderr, "");
    // Every branch counts: the query filters on success only, 100 runs a page.
    assert_eq!(result.calls.len(), 1, "{:?}", result.calls);
    let call = &result.calls[0];
    assert!(
        call.contains("repos/owner/repo/actions/workflows/ci.yml/runs"),
        "{call}"
    );
    assert!(call.contains("status=success"), "{call}");
    assert!(call.contains("per_page=100"), "{call}");
    assert!(!call.contains("branch="), "{call}");
}

#[test]
fn ci_reads_the_next_page_when_a_page_holds_no_ancestor() {
    let repo = Repo::new();
    let pages = [
        vec![run(5, &repo.x), run(4, UNKNOWN)],
        vec![run(3, &repo.a)],
    ];
    let result = repo.run_ci(&pages, false);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(result.stdout, repo.a);
    assert_eq!(result.calls.len(), 2, "{:?}", result.calls);
    assert!(result.calls[1].contains("page=2"), "{}", result.calls[1]);
}

#[test]
fn ci_with_no_green_ancestor_falls_back_to_the_parent_commit_with_a_message() {
    let repo = Repo::new();
    let page = vec![run(5, &repo.x), run(4, UNKNOWN), run(RUN_ID, &repo.c)];
    let result = repo.run_ci(&[page], false);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    // HEAD^
    assert_eq!(result.stdout, repo.b);
    assert!(
        result
            .stderr
            .starts_with("mutants-base: no successful ci.yml run "),
        "{}",
        result.stderr
    );
    assert!(
        result
            .stderr
            .contains(" HEAD descends from; using the parent commit"),
        "{}",
        result.stderr
    );
}

#[test]
fn ci_with_no_green_run_at_all_falls_back_to_the_parent_commit() {
    let repo = Repo::new();
    let result = repo.run_ci(&[], false);
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(result.stdout, repo.b);
    assert!(
        result.stderr.contains("using the parent commit"),
        "{}",
        result.stderr
    );
}

#[test]
fn ci_stops_with_exit_1_and_no_base_when_gh_fails() {
    let repo = Repo::new();
    let result = repo.run_ci(&[vec![run(3, &repo.b)]], true);
    assert_eq!(result.code, Some(1));
    assert_eq!(result.stdout, "");
    assert!(
        result
            .stderr
            .contains("mutants-base: the gh api call failed; stopping"),
        "{}",
        result.stderr
    );
}

#[test]
fn local_takes_the_upstream_else_origin_main_else_the_parent_commit() {
    let repo = Repo::new();
    // No remote at all: the parent commit, with a message.
    let result = repo.run_local();
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(result.stdout, repo.b);
    assert!(
        result
            .stderr
            .contains("no upstream and origin/main is missing; using the parent commit"),
        "{}",
        result.stderr
    );

    // origin/main exists (at X here, so that each answer differs), the branch has no upstream:
    // origin/main.
    let origin = repo.scratch.0.join("origin.git");
    let origin_text = origin.to_str().expect("the scratch path is text");
    run_git(
        &repo.scratch.0,
        &repo.scratch.0,
        &["init", "-q", "--bare", origin_text],
    );
    repo.git(&["remote", "add", "origin", origin_text]);
    repo.git(&[
        "push",
        "-q",
        "origin",
        &format!("{}:refs/heads/main", repo.x),
    ]);
    repo.git(&["fetch", "-q", "origin"]);
    let result = repo.run_local();
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(result.stdout, repo.x);
    assert_eq!(result.stderr, "");

    // The branch tracks origin/work, which is at A: the upstream wins over origin/main.
    repo.git(&[
        "push",
        "-q",
        "origin",
        &format!("{}:refs/heads/work", repo.a),
    ]);
    repo.git(&["fetch", "-q", "origin"]);
    repo.git(&["branch", "-q", "--set-upstream-to=origin/work", "work"]);
    assert_eq!(
        repo.git(&[
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}"
        ]),
        "origin/work"
    );
    let result = repo.run_local();
    assert_eq!(result.code, Some(0), "{}", result.stderr);
    assert_eq!(result.stdout, repo.a);
    assert_eq!(result.stderr, "");
}

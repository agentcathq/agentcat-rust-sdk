// scripts/mutants-base.sh, the base commit of the mutation diff gate (C53). Each test copies the script into a
// scratch git repository under a temporary directory and runs it there, in CI mode with a fake `gh` first on
// PATH (it answers from JSON files through jq, as `gh api --jq` does), or in local mode. No test contacts the
// network or reads this checkout's history.
import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { chmodSync, copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { after, test } from "node:test";

const SCRIPT = join(dirname(fileURLToPath(import.meta.url)), "mutants-base.sh");
const RUN_ID = 999;

const scratch = mkdtempSync(join(tmpdir(), "mutants-base-"));
after(() => rmSync(scratch, { recursive: true, force: true }));

// No developer or runner setting reaches git, and no GitHub variable of a real CI run reaches the script.
const baseEnv = Object.fromEntries(
  Object.entries(process.env).filter(([name]) => !name.startsWith("GITHUB_") && !name.startsWith("GIT_") && name !== "GH_TOKEN"),
);
Object.assign(baseEnv, {
  GIT_CONFIG_GLOBAL: join(scratch, "no-gitconfig"),
  GIT_CONFIG_NOSYSTEM: "1",
  GIT_AUTHOR_NAME: "t",
  GIT_AUTHOR_EMAIL: "t@example.com",
  GIT_COMMITTER_NAME: "t",
  GIT_COMMITTER_EMAIL: "t@example.com",
});
writeFileSync(baseEnv.GIT_CONFIG_GLOBAL, "");

// The fake gh: logs its arguments, fails when FAKE_GH_FAIL is set, else prints jq's output of
// $FAKE_GH_DIR/page-<page>.json (an empty run list when the page has no file).
const FAKE_BIN = join(scratch, "bin");
mkdirSync(FAKE_BIN);
writeFileSync(
  join(FAKE_BIN, "gh"),
  `#!/usr/bin/env bash
printf '%s\\n' "$*" >> "$FAKE_GH_DIR/calls.log"
if [ -n "\${FAKE_GH_FAIL:-}" ]; then
  echo "gh: HTTP 403: API rate limit exceeded" >&2
  exit 1
fi
page=1
expr=.
while [ $# -gt 0 ]; do
  case "$1" in
    -f) case "$2" in page=*) page="\${2#page=}" ;; esac; shift 2 ;;
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
`,
);
chmodSync(join(FAKE_BIN, "gh"), 0o755);

let repoCount = 0;

/**
 * A scratch repository: root A, then B and C on branch `work` (HEAD = C), and X on branch `side` off A, which
 * HEAD does not descend from. The script sits in its scripts/ directory. Returns the path and the SHAs.
 */
function makeRepo() {
  const dir = join(scratch, `repo-${repoCount++}`);
  mkdirSync(join(dir, "scripts"), { recursive: true });
  const git = (...args) => execFileSync("git", args, { cwd: dir, env: baseEnv, encoding: "utf8" }).trim();
  git("init", "-q", "-b", "work");
  const commit = (name) => {
    writeFileSync(join(dir, `${name}.txt`), name);
    git("add", `${name}.txt`);
    git("commit", "-q", "-m", name);
    return git("rev-parse", "HEAD");
  };
  const a = commit("A");
  git("checkout", "-q", "-b", "side");
  const x = commit("X");
  git("checkout", "-q", "work");
  const b = commit("B");
  const c = commit("C");
  copyFileSync(SCRIPT, join(dir, "scripts", "mutants-base.sh"));
  chmodSync(join(dir, "scripts", "mutants-base.sh"), 0o755);
  return { dir, git, a, b, c, x };
}

/** Runs the script of `repo`; `ci` holds the run pages for CI mode (undefined: local mode). */
function runScript(repo, { ci, fail = false } = {}) {
  const env = { ...baseEnv };
  if (ci !== undefined) {
    const ghDir = join(repo.dir, ".gh");
    mkdirSync(ghDir, { recursive: true });
    ci.forEach((runs, index) => writeFileSync(join(ghDir, `page-${index + 1}.json`), JSON.stringify({ workflow_runs: runs })));
    Object.assign(env, {
      PATH: `${FAKE_BIN}:${process.env.PATH}`,
      FAKE_GH_DIR: ghDir,
      GITHUB_ACTIONS: "true",
      GITHUB_REPOSITORY: "owner/repo",
      GITHUB_RUN_ID: String(RUN_ID),
      GITHUB_EVENT_NAME: "push",
      GITHUB_REF_NAME: "work",
    });
    if (fail) env.FAKE_GH_FAIL = "1";
  }
  const result = spawnSync(join(repo.dir, "scripts", "mutants-base.sh"), [], { cwd: repo.dir, env, encoding: "utf8" });
  const calls = ci === undefined ? [] : readCalls(join(repo.dir, ".gh", "calls.log"));
  return { code: result.status, stdout: result.stdout.trim(), stderr: result.stderr, calls };
}

function readCalls(path) {
  try {
    return readFileSync(path, "utf8").trim().split("\n");
  } catch {
    return [];
  }
}

const run = (id, sha) => ({ id, head_sha: sha });
const UNKNOWN = "0123456789abcdef0123456789abcdef01234567";

test("CI: skips this run, a non-ancestor commit and a commit missing from the clone; picks the next ancestor", () => {
  const repo = makeRepo();
  // Newest first: this run (on HEAD itself), a green commit HEAD does not descend from, one the clone lacks, then B.
  const result = runScript(repo, { ci: [[run(RUN_ID, repo.c), run(5, repo.x), run(4, UNKNOWN), run(3, repo.b), run(2, repo.a)]] });
  assert.equal(result.code, 0, result.stderr);
  assert.equal(result.stdout, repo.b);
  assert.equal(result.stderr, "");
  // Every branch counts: the query filters on success only, 100 runs a page.
  assert.equal(result.calls.length, 1);
  assert.match(result.calls[0], /repos\/owner\/repo\/actions\/workflows\/ci\.yml\/runs/);
  assert.match(result.calls[0], /status=success/);
  assert.match(result.calls[0], /per_page=100/);
  assert.doesNotMatch(result.calls[0], /branch=/);
});

test("CI: reads the next page when a page holds no ancestor", () => {
  const repo = makeRepo();
  const result = runScript(repo, { ci: [[run(5, repo.x), run(4, UNKNOWN)], [run(3, repo.a)]] });
  assert.equal(result.code, 0, result.stderr);
  assert.equal(result.stdout, repo.a);
  assert.equal(result.calls.length, 2);
  assert.match(result.calls[1], /page=2/);
});

test("CI: no green ancestor falls back to the parent commit, with a message", () => {
  const repo = makeRepo();
  const result = runScript(repo, { ci: [[run(5, repo.x), run(4, UNKNOWN), run(RUN_ID, repo.c)]] });
  assert.equal(result.code, 0, result.stderr);
  assert.equal(result.stdout, repo.b); // HEAD^
  assert.match(result.stderr, /mutants-base: no successful ci\.yml run .* HEAD descends from; using the parent commit/);
});

test("CI: no green run at all falls back to the parent commit", () => {
  const repo = makeRepo();
  const result = runScript(repo, { ci: [] });
  assert.equal(result.code, 0, result.stderr);
  assert.equal(result.stdout, repo.b);
  assert.match(result.stderr, /using the parent commit/);
});

test("CI: a gh error stops the script with exit 1 and prints no base", () => {
  const repo = makeRepo();
  const result = runScript(repo, { ci: [[run(3, repo.b)]], fail: true });
  assert.equal(result.code, 1);
  assert.equal(result.stdout, "");
  assert.match(result.stderr, /mutants-base: the gh api call failed; stopping/);
});

test("local: the upstream of the current branch, else origin/main, else the parent commit", () => {
  const repo = makeRepo();
  // No remote at all: the parent commit, with a message.
  let result = runScript(repo);
  assert.equal(result.code, 0, result.stderr);
  assert.equal(result.stdout, repo.b);
  assert.match(result.stderr, /no upstream and origin\/main is missing; using the parent commit/);

  // origin/main exists (at X here, so that each answer differs), the branch has no upstream: origin/main.
  const origin = `${repo.dir}-origin.git`;
  execFileSync("git", ["init", "-q", "--bare", origin], { env: baseEnv });
  repo.git("remote", "add", "origin", origin);
  repo.git("push", "-q", "origin", `${repo.x}:refs/heads/main`);
  repo.git("fetch", "-q", "origin");
  result = runScript(repo);
  assert.equal(result.code, 0, result.stderr);
  assert.equal(result.stdout, repo.x);
  assert.equal(result.stderr, "");

  // The branch tracks origin/work, which is at A: the upstream wins over origin/main.
  repo.git("push", "-q", "origin", `${repo.a}:refs/heads/work`);
  repo.git("fetch", "-q", "origin");
  repo.git("branch", "-q", "--set-upstream-to=origin/work", "work");
  assert.equal(repo.git("rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{upstream}"), "origin/work");
  result = runScript(repo);
  assert.equal(result.code, 0, result.stderr);
  assert.equal(result.stdout, repo.a);
  assert.equal(result.stderr, "");
});

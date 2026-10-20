// Clones the TS SDK at the locked commit into oracle/.cache, checks that the
// tree is clean, then installs and builds it with the pnpm version the TS repo
// pins (through corepack). A marker file records a finished build, so later
// calls only re-check the checkout.
//
// CLI: node oracle/src/fetch-ts.mjs [--force]
import { spawn } from "node:child_process";
import {
  appendFileSync,
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  realpathSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { basename, dirname, join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { fileURLToPath } from "node:url";

import { lock, tsDir, tsDist } from "./paths.mjs";

/** Holds the SHA once install and build have succeeded; git-excluded in the clone. */
const MARKER = ".oracle-ok";
const LOCK_WAIT_MS = 30 * 60_000;

let ready = null; // this process's successful check, shared by later non-forced calls
let queue = Promise.resolve(); // runs in this process one at a time

/**
 * Makes `tsDir()` a clean checkout of the locked commit with a fresh build in
 * `dist/`. A no-op apart from checking HEAD and `git status` when the marker
 * holds the SHA; `force` ignores the marker and re-verifies, reinstalls and
 * rebuilds. Rejects with a "not clean" error when the clone has local changes.
 */
export async function ensureTs({ force = false } = {}) {
  if (!force && ready) return ready;
  if (force) ready = null;
  const attempt = queue.then(() => ensure(force));
  queue = attempt.catch(() => {});
  if (!force) {
    ready = attempt;
    attempt.catch(() => {
      if (ready === attempt) ready = null;
    });
  }
  return attempt;
}

async function ensure(force) {
  const { repo, sha } = lock();
  const dir = tsDir();
  if (!force && isBuilt(dir, sha)) {
    await verifyCheckout(dir, sha);
    return;
  }
  await withLock(`${dir}.lock`, async () => {
    // Another process may have finished the build while this one waited.
    if (!force && isBuilt(dir, sha)) {
      await verifyCheckout(dir, sha);
      return;
    }
    await cloneOrFetch(repo, sha, dir);
    excludeMarker(dir);
    await git(dir, ["-c", "advice.detachedHead=false", "checkout", "--quiet", "--detach", sha]);
    await verifyCheckout(dir, sha);

    rmSync(join(dir, MARKER), { force: true });
    const pnpm = pnpmArgs(dir);
    progress(`installing the TS SDK dependencies in ${dir}`);
    await run("corepack", [...pnpm, "install", "--frozen-lockfile"], dir);
    progress("building the TS SDK");
    await run("corepack", [...pnpm, "build"], dir);
    if (!existsSync(tsDist())) {
      throw new Error(`oracle: the TS build did not produce ${tsDist()}`);
    }
    await verifyCheckout(dir, sha);
    const tmp = join(dir, `${MARKER}.tmp-${process.pid}`);
    writeFileSync(tmp, `${sha}\n`);
    renameSync(tmp, join(dir, MARKER));
  });
}

function isBuilt(dir, sha) {
  let marker;
  try {
    marker = readFileSync(join(dir, MARKER), "utf8").trim();
  } catch {
    return false;
  }
  return marker === sha && existsSync(tsDist());
}

async function verifyCheckout(dir, sha) {
  const head = (await git(dir, ["rev-parse", "HEAD"])).trim();
  if (head !== sha) {
    throw new Error(`oracle: the TS clone ${dir} is at ${head}, expected ${sha}`);
  }
  const status = (await git(dir, ["status", "--porcelain", "--untracked-files=all"])).trimEnd();
  if (status !== "") {
    throw new Error(
      `oracle: the TS clone ${dir} is not clean (delete the directory to re-clone it):\n${status}`,
    );
  }
}

async function cloneOrFetch(repo, sha, dir) {
  const cache = dirname(dir);
  mkdirSync(cache, { recursive: true });
  // Leftovers of interrupted clones; safe to remove while holding the lock.
  for (const entry of readdirSync(cache)) {
    if (entry.startsWith(`${basename(dir)}.tmp-`)) {
      rmSync(join(cache, entry), { recursive: true, force: true });
    }
  }
  if (!existsSync(join(dir, ".git"))) {
    rmSync(dir, { recursive: true, force: true });
    const tmp = `${dir}.tmp-${process.pid}`;
    progress(`cloning ${repo} into ${dir}`);
    await git(null, ["clone", "--quiet", "--no-checkout", repo, tmp]);
    renameSync(tmp, dir);
  }
  try {
    await git(dir, ["cat-file", "-e", `${sha}^{commit}`]);
  } catch {
    progress(`fetching ${sha} from ${repo}`);
    await git(dir, ["fetch", "--quiet", repo, sha]);
  }
}

/** Keeps the marker (and its temp file) out of `git status`. */
function excludeMarker(dir) {
  const file = join(dir, ".git", "info", "exclude");
  const line = `/${MARKER}*`;
  const text = existsSync(file) ? readFileSync(file, "utf8") : "";
  if (text.split(/\r?\n/).includes(line)) return;
  mkdirSync(dirname(file), { recursive: true });
  appendFileSync(file, `${text === "" || text.endsWith("\n") ? "" : "\n"}${line}\n`);
}

/**
 * The clone sits inside this repository's pnpm workspace; without its own
 * pnpm-workspace.yaml, pnpm would take ours and install the wrong project.
 */
function pnpmArgs(dir) {
  return existsSync(join(dir, "pnpm-workspace.yaml"))
    ? ["pnpm"]
    : ["pnpm", "--ignore-workspace"];
}

/** Serializes builds of the same clone across processes (mkdir is atomic). */
async function withLock(lockDir, fn) {
  mkdirSync(dirname(lockDir), { recursive: true });
  const deadline = Date.now() + LOCK_WAIT_MS;
  for (;;) {
    try {
      mkdirSync(lockDir);
      break;
    } catch (err) {
      if (err.code !== "EEXIST") throw err;
      if (lockIsStale(lockDir)) {
        rmSync(lockDir, { recursive: true, force: true });
        continue;
      }
      if (Date.now() > deadline) {
        throw new Error(`oracle: timed out waiting for ${lockDir}; delete it if no build is running`);
      }
      await sleep(250);
    }
  }
  writeFileSync(join(lockDir, "pid"), String(process.pid));
  try {
    return await fn();
  } finally {
    rmSync(lockDir, { recursive: true, force: true });
  }
}

function lockIsStale(lockDir) {
  let pid;
  try {
    pid = Number(readFileSync(join(lockDir, "pid"), "utf8"));
  } catch {
    // The owner writes its pid right after mkdir; a pid-less lock older than a
    // minute belongs to a process that died in between.
    try {
      return Date.now() - statSync(lockDir).mtimeMs > 60_000;
    } catch {
      return false;
    }
  }
  if (!Number.isInteger(pid) || pid <= 0) return true;
  try {
    process.kill(pid, 0);
    return false;
  } catch (err) {
    return err.code === "ESRCH";
  }
}

// Variables that point git at another repository (set inside git hooks) and
// npm/pnpm/node:test settings inherited from the caller's own pnpm script.
const DROPPED_ENV = new Set([
  "GIT_DIR",
  "GIT_WORK_TREE",
  "GIT_INDEX_FILE",
  "GIT_OBJECT_DIRECTORY",
  "GIT_ALTERNATE_OBJECT_DIRECTORIES",
  "GIT_COMMON_DIR",
  "GIT_PREFIX",
  "INIT_CWD",
  "PNPM_SCRIPT_SRC_DIR",
  "PNPM_PACKAGE_NAME",
  "NODE_TEST_CONTEXT",
]);

function childEnv() {
  const env = {};
  for (const [key, value] of Object.entries(process.env)) {
    if (/^npm_/i.test(key) || DROPPED_ENV.has(key.toUpperCase())) continue;
    env[key] = value;
  }
  env.COREPACK_ENABLE_DOWNLOAD_PROMPT = "0";
  env.GIT_TERMINAL_PROMPT = "0";
  env.HUSKY = "0"; // the TS repo's `prepare` script would install git hooks
  return env;
}

function git(dir, args) {
  return run("git", dir === null ? args : ["-C", dir, ...args], dir ?? process.cwd());
}

/** Runs a command; resolves with its stdout, rejects with its output on failure. */
function run(cmd, args, cwd) {
  return new Promise((resolve, reject) => {
    // corepack is a .cmd shim on Windows, which only starts through a shell.
    const shell = process.platform === "win32" && cmd === "corepack";
    const child = spawn(cmd, args, { cwd, env: childEnv(), shell, stdio: ["ignore", "pipe", "pipe"] });
    let stdout = "";
    let output = "";
    const keep = (text) => {
      output = (output + text).slice(-32_768);
    };
    child.stdout.setEncoding("utf8").on("data", (text) => {
      stdout += text;
      keep(text);
    });
    child.stderr.setEncoding("utf8").on("data", keep);
    child.on("error", (err) => reject(new Error(`oracle: cannot run ${cmd}: ${err.message}`)));
    child.on("close", (code, signal) => {
      if (code === 0) {
        resolve(stdout);
        return;
      }
      const why = signal === null ? `exit ${code}` : signal;
      reject(new Error(`oracle: \`${[cmd, ...args].join(" ")}\` failed (${why}) in ${cwd}:\n${output}`));
    });
  });
}

// stdout may be a protocol channel (the property-test server), so progress goes to stderr.
function progress(message) {
  process.stderr.write(`oracle: ${message}\n`);
}

function isMain() {
  if (!process.argv[1]) return false;
  try {
    return realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url));
  } catch {
    return false;
  }
}

if (isMain()) {
  const args = process.argv.slice(2);
  if (args.some((arg) => arg !== "--force")) {
    process.stderr.write("usage: node oracle/src/fetch-ts.mjs [--force]\n");
    process.exit(2);
  }
  ensureTs({ force: args.includes("--force") }).then(
    () => process.stdout.write(`oracle: TS SDK ${lock().sha} ready in ${tsDir()}\n`),
    (err) => {
      process.stderr.write(`${err.message}\n`);
      process.exitCode = 1;
    },
  );
}

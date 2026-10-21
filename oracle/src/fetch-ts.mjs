// Clones the TS SDK at the locked commit into oracle/.cache, checks that the
// tree is clean, then installs and builds it with the pnpm version the TS repo
// pins (through corepack). A marker file records a finished build, so later
// calls only re-check the checkout (every call: the clone can change while a
// long-running process uses it).
//
// CLI: node oracle/src/fetch-ts.mjs [--force]
import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import {
  appendFileSync,
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  realpathSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { basename, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { isAlive, withLock } from "./lock.mjs";
import { lock, traceOverride, tsDir, tsDist } from "./paths.mjs";

/** Holds the SHA once install and build have succeeded; git-excluded in the clone. */
const MARKER = ".oracle-ok";

let queue = Promise.resolve(); // runs in this process one at a time

/**
 * Makes `tsDir()` a clean checkout of the locked commit with a fresh build in
 * `dist/`. A no-op apart from checking HEAD and `git status` when the marker
 * holds the SHA; `force` ignores the marker and re-verifies, reinstalls and
 * rebuilds. Rejects with a "not clean" error when the clone has local changes.
 */
export async function ensureTs({ force = false } = {}) {
  return serial(() => ensure(force));
}

/**
 * Checks, without cloning or building, that `tsDir()` is a built, clean
 * checkout of the locked commit. Rejects otherwise ("not clean" for local
 * changes). Run it right before persisting anything derived from the clone.
 */
export async function verifyTs() {
  return serial(async () => {
    const { sha } = lock();
    const dir = tsDir();
    if (!isBuilt(dir, sha)) {
      throw new Error(`oracle: the TS SDK in ${dir} is not built; run ensureTs() first`);
    }
    await verifyCheckout(dir, sha);
  });
}

function serial(task) {
  const attempt = queue.then(task);
  queue = attempt.catch(() => {});
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
  // Leftovers of interrupted clones, named <dir>.tmp-<pid>-<id>. Only a dead
  // process's leftover goes: a live one may still be cloning.
  const prefix = `${basename(dir)}.tmp-`;
  for (const entry of readdirSync(cache)) {
    if (!entry.startsWith(prefix)) continue;
    const pid = Number.parseInt(entry.slice(prefix.length), 10);
    if (!(pid > 0 && isAlive(pid))) rmSync(join(cache, entry), { recursive: true, force: true });
  }
  if (!existsSync(join(dir, ".git"))) {
    rmSync(dir, { recursive: true, force: true });
    const tmp = `${dir}.tmp-${process.pid}-${randomUUID().slice(0, 8)}`;
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
 * The corepack arguments that run the clone's own pnpm in `dir`:
 * `run("corepack", [...pnpmArgs(dir), "exec", ...], dir)`. The clone sits
 * inside this repository's pnpm workspace; without its own
 * pnpm-workspace.yaml, pnpm would take ours and install the wrong project.
 */
export function pnpmArgs(dir) {
  return existsSync(join(dir, "pnpm-workspace.yaml"))
    ? ["pnpm"]
    : ["pnpm", "--ignore-workspace"];
}

/**
 * NODE_OPTIONS for a process that runs TS code and needs no network: `existing` (by default the
 * caller's own NODE_OPTIONS), then `--import` of the egress guard (oracle/src/egress-guard.mjs).
 * Node applies NODE_OPTIONS to every Node process the child starts in turn (vitest's forks), but
 * not to worker threads.
 */
export function guardedNodeOptions(existing = process.env.NODE_OPTIONS ?? "") {
  const guard = `--import=${new URL("./egress-guard.mjs", import.meta.url).href}`;
  return existing.trim() === "" ? guard : `${existing} ${guard}`;
}

/**
 * vitest in the clone `dir`, under the egress guard: `run(cmd, args, dir, env)`. node runs vitest's
 * own entry (the `bin` of its package.json), so corepack and pnpm, which may download, stay out of
 * the guarded process. `nodeOptions` goes before the guard (see guardedNodeOptions).
 */
export function vitestCommand(dir, vitestArgs, nodeOptions = process.env.NODE_OPTIONS ?? "") {
  const pkgDir = join(dir, "node_modules", "vitest");
  const { bin } = JSON.parse(readFileSync(join(pkgDir, "package.json"), "utf8"));
  const entry = typeof bin === "string" ? bin : bin?.vitest;
  if (typeof entry !== "string") throw new Error(`oracle: ${join(pkgDir, "package.json")} has no vitest bin`);
  return {
    cmd: process.execPath,
    args: [join(pkgDir, entry), ...vitestArgs],
    env: { NODE_OPTIONS: guardedNodeOptions(nodeOptions) },
  };
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

/**
 * Runs a command with the scrubbed environment (plus `extraEnv`); resolves
 * with its stdout, rejects with its output on failure.
 */
export function run(cmd, args, cwd, extraEnv = {}) {
  return new Promise((resolve, reject) => {
    // corepack is a .cmd shim on Windows, which only starts through a shell.
    const shell = process.platform === "win32" && cmd === "corepack";
    const env = { ...childEnv(), ...extraEnv };
    const child = spawn(cmd, args, { cwd, env, shell, stdio: ["ignore", "pipe", "pipe"] });
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
  try {
    traceOverride(); // the clone this CLI builds follows ORACLE_SHA
  } catch (err) {
    process.stderr.write(`${err.message}\n`);
    process.exit(1);
  }
  ensureTs({ force: args.includes("--force") }).then(
    () => process.stdout.write(`oracle: TS SDK ${lock().sha} ready in ${tsDir()}\n`),
    (err) => {
      process.stderr.write(`${err.message}\n`);
      process.exitCode = 1;
    },
  );
}

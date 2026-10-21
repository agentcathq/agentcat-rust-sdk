// Lists every test of the TS SDK at the locked commit with `vitest list`, for the parity map.
//   node oracle/src/parity/list-tests.mjs                   write parity/ts-tests.json
//   node oracle/src/parity/list-tests.mjs --out <dir>       write <dir>/ts-tests.json instead
//   node oracle/src/parity/list-tests.mjs --check           regenerate to a temp dir; exit 1 on any byte difference
//   node oracle/src/parity/list-tests.mjs --init-manifest   write parity/manifest.toml (one "todo" entry per key)
//                                                           from parity/ts-tests.json; refuses to overwrite
// --init-manifest with --out <dir> reads and writes in <dir> instead of parity/.
//
// ts-tests.json is {"ts_sha", "count", "tests": [{"key", "project", "file", "title"}]}, sorted by key,
// where key = "<project>|<file relative to the clone>|<title>". The title is vitest's full name:
// the describe blocks and the test name joined by " > ". vitest leaves out skipped and todo tests.
import { existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

import { ensureTs, run, verifyTs, vitestCommand } from "../fetch-ts.mjs";
import { lock, traceOverride, tsDir } from "../paths.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const DEFAULT_OUT = resolve(HERE, "../../../parity");
const LIST_FILE = "ts-tests.json";
const MANIFEST_FILE = "manifest.toml";

/** One `vitest list` run per project; the label is the `project` field and the key prefix. */
const PROJECTS = [
  { project: "v1", args: ["--project", "v1"] },
  { project: "v2", args: ["--project", "v2"] },
  { project: "e2e-http", args: ["--project", "e2e-http"] },
  // The Workers tests run under their own config (its project name is "workers").
  { project: "workers", args: ["--config", "vitest.workers.config.ts"] },
];

/** Byte order on UTF-16 code units: stable across locales and Node versions. */
function byKey(a, b) {
  if (a.key < b.key) return -1;
  return a.key > b.key ? 1 : 0;
}

/** Runs `vitest list --json` for every project in the clone; returns the sorted test list. */
async function listTests() {
  await ensureTs();
  const dir = tsDir();
  const root = realpathSync(dir);
  // vitest writes the JSON to a file: test files may print to stdout while they are collected.
  // The file sits next to the clone and is passed as a relative path with no spaces.
  const tmp = mkdtempSync(join(dirname(dir), "parity-list-"));
  try {
    const tests = [];
    for (const { project, args } of PROJECTS) {
      const out = join(tmp, `${project}.json`);
      const jsonArg = `--json=${relative(dir, out).split(sep).join("/")}`;
      // Collecting runs the top level of every TS test file (unreviewed code at TS main), so vitest
      // and every Node process it starts load the egress guard.
      const vitest = vitestCommand(dir, ["list", jsonArg, ...args]);
      const stdout = await run(vitest.cmd, vitest.args, dir, {
        ...vitest.env,
        DISABLE_DIAGNOSTICS: "true",
        WRANGLER_SEND_METRICS: "false",
      });
      if (!existsSync(out)) {
        throw new Error(
          `parity: \`vitest list ${args.join(" ")}\` wrote no JSON (collection errors?) in ${dir}:\n${stdout.slice(-8_192)}`,
        );
      }
      for (const entry of JSON.parse(readFileSync(out, "utf8"))) {
        tests.push(toTest(project, root, entry));
      }
    }
    tests.sort(byKey);
    for (let i = 1; i < tests.length; i++) {
      if (tests[i].key === tests[i - 1].key) throw new Error(`parity: duplicate key ${JSON.stringify(tests[i].key)}`);
    }
    // Nothing derived from the clone is persisted unless the clone is still clean.
    await verifyTs();
    return tests;
  } finally {
    rmSync(tmp, { recursive: true, force: true });
  }
}

function toTest(project, root, entry) {
  const { name, file, projectName } = entry;
  if (projectName !== project) {
    throw new Error(`parity: vitest reported project ${JSON.stringify(projectName)} for a ${project} run`);
  }
  if (typeof name !== "string" || name === "" || !name.isWellFormed()) {
    throw new Error(`parity: bad test name ${JSON.stringify(name)} in ${file}`);
  }
  const rel = relative(root, realpathSync(file));
  if (rel === "" || rel.startsWith("..") || isAbsolute(rel)) {
    throw new Error(`parity: test file ${file} is outside the clone ${root}`);
  }
  const posix = rel.split(sep).join("/");
  return { key: `${project}|${posix}|${name}`, project, file: posix, title: name };
}

function render(tests) {
  return JSON.stringify({ ts_sha: lock().sha, count: tests.length, tests }, null, 2) + "\n";
}

async function writeList(dir) {
  const tests = await listTests();
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, LIST_FILE), render(tests));
  return tests;
}

async function check() {
  const tmp = mkdtempSync(join(tmpdir(), "parity-"));
  try {
    await writeList(tmp);
    const fresh = readFileSync(join(tmp, LIST_FILE));
    let have = null;
    try { have = readFileSync(join(DEFAULT_OUT, LIST_FILE)); } catch { have = null; }
    if (have === null || !fresh.equals(have)) {
      process.stderr.write(`parity/${LIST_FILE} out of date: run node oracle/src/parity/list-tests.mjs\n`);
      return 1;
    }
    process.stdout.write(`parity/${LIST_FILE} up to date\n`);
    return 0;
  } finally {
    rmSync(tmp, { recursive: true, force: true });
  }
}

/** A TOML basic string: escapes `"`, `\` and control characters. */
function tomlString(text) {
  let out = '"';
  for (const ch of text) {
    const code = ch.codePointAt(0);
    if (ch === '"' || ch === "\\") out += `\\${ch}`;
    else if (code < 0x20 || code === 0x7f) out += `\\u${code.toString(16).toUpperCase().padStart(4, "0")}`;
    else out += ch;
  }
  return `${out}"`;
}

const MANIFEST_HEADER = `# Parity map: every TS test in ts-tests.json maps to Rust tests, harness scenarios, or n/a.
# Generated once by \`node oracle/src/parity/list-tests.mjs --init-manifest\`, then maintained by hand.
# Checked by \`cargo run -p xtask -- parity\`; \`parity --final\` also rejects "todo".
#
# [[test]]
# key = "<project>|<file>|<title>"   # from ts-tests.json
# status = "todo"                    # "todo" | "ported" | "n/a"
# rust = ["<binary-id>::<test name>", "harness:<scenario>"]   # ported only
# category = "ts-host-only"          # n/a only: ts-host-only | second-pass | ruled-divergence:RD-xx | covered-by-vector:<file>.json
# reason = "..."                     # n/a only
`;

function initManifest(dir) {
  const target = join(dir, MANIFEST_FILE);
  if (existsSync(target)) {
    process.stderr.write(`parity: ${target} exists; refusing to overwrite it\n`);
    return 1;
  }
  const { tests } = JSON.parse(readFileSync(join(dir, LIST_FILE), "utf8"));
  const entries = tests.map(({ key }) => `[[test]]\nkey = ${tomlString(key)}\nstatus = "todo"\n`);
  writeFileSync(target, `${MANIFEST_HEADER}\n${entries.join("\n")}`, { flag: "wx" });
  process.stdout.write(`parity: ${tests.length} todo entries -> ${target}\n`);
  return 0;
}

const args = process.argv.slice(2);
const outAt = args.indexOf("--out");
if (outAt >= 0 && !args[outAt + 1]) {
  process.stderr.write("--out needs a directory\n");
  process.exit(2);
}
const known = new Set(["--check", "--init-manifest", "--out"]);
const unknown = args.filter((arg, i) => !known.has(arg) && !(outAt >= 0 && i === outAt + 1));
if (unknown.length > 0 || (args.includes("--check") && args.length > 1)) {
  process.stderr.write("usage: node oracle/src/parity/list-tests.mjs [--check | [--init-manifest] [--out <dir>]]\n");
  process.exit(2);
}
const out = outAt >= 0 ? resolve(args[outAt + 1]) : DEFAULT_OUT;
try {
  traceOverride();
  if (args.includes("--check")) {
    process.exitCode = await check();
  } else if (args.includes("--init-manifest")) {
    process.exitCode = initManifest(out);
  } else {
    const tests = await writeList(out);
    const perProject = PROJECTS.map(({ project }) => `${project} ${tests.filter((t) => t.project === project).length}`);
    process.stdout.write(`parity: ${tests.length} TS tests (${perProject.join(", ")}) -> ${join(out, LIST_FILE)}\n`);
  }
} catch (err) {
  process.stderr.write(`${err.message}\n`);
  process.exitCode = 1;
}

// The oracle must be built from the pinned TS commit and nothing else: a clone
// that is dirty or at another SHA fails instead of producing vectors.
import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";

import { loadModule } from "../src/bundle.mjs";
import { DROPPED_ENV, ensureTs, verifyTs } from "../src/fetch-ts.mjs";
import { bundleDir, lock, tsDir, tsDist } from "../src/paths.mjs";

function git(args) {
  const env = { ...process.env };
  for (const key of DROPPED_ENV) delete env[key]; // the variables run() drops: inside a git hook they point git at this repository
  return execFileSync("git", ["-C", tsDir(), ...args], { encoding: "utf8", env }).trim();
}

test("the git() helper ignores GIT_DIR, as run() does", async () => {
  await ensureTs(); // the clone must exist, so a failure means GIT_DIR, not a missing clone
  const before = process.env.GIT_DIR;
  process.env.GIT_DIR = "/nonexistent";
  try {
    assert.equal(git(["rev-parse", "HEAD"]), lock().sha);
  } finally {
    if (before === undefined) delete process.env.GIT_DIR;
    else process.env.GIT_DIR = before;
  }
});

test("the clone is clean and at the locked SHA", async () => {
  await ensureTs();
  assert.equal(git(["rev-parse", "HEAD"]), lock().sha);
  assert.equal(git(["status", "--porcelain"]), "");
  assert.ok(existsSync(tsDist()));
});

test("a dirty clone is rejected", async () => {
  writeFileSync(join(tsDir(), "src", "dirty.txt"), "x");
  try {
    await assert.rejects(ensureTs({ force: true }), /not clean/);
  } finally {
    rmSync(join(tsDir(), "src", "dirty.txt"));
  }
});

test("a forced run removes only dead processes' temp clones", async () => {
  const live = `${tsDir()}.tmp-${process.pid}-live`;
  const dead = `${tsDir()}.tmp-${spawnSync(process.execPath, ["-e", ""]).pid}-dead`;
  mkdirSync(live);
  mkdirSync(dead);
  // The dirty file stops the forced run right after the cleanup, before a rebuild.
  writeFileSync(join(tsDir(), "src", "dirty.txt"), "x");
  try {
    await assert.rejects(ensureTs({ force: true }), /not clean/);
    assert.equal(existsSync(live), true, "a live process's temp clone was removed");
    assert.equal(existsSync(dead), false, "a dead process's temp clone was kept");
  } finally {
    rmSync(join(tsDir(), "src", "dirty.txt"));
    rmSync(live, { recursive: true, force: true });
    rmSync(dead, { recursive: true, force: true });
  }
});

test("modules load from the pinned source", async () => {
  const tokens = await loadModule("tokenEstimate");
  assert.equal(typeof tokens.estimateInputTokens, "function");
  assert.equal(tokens.estimateInputTokens({ n: 1.0 }), 2);
});

test("import.meta.url in a bundled module is its source file's URL", async () => {
  // runtime-versions resolves packages with createRequire(import.meta.url):
  // from the bundle directory, that walk would miss the clone's node_modules.
  const versions = await loadModule("runtime-versions");
  const manifest = join(tsDir(), "node_modules", "@modelcontextprotocol", "sdk", "package.json");
  const { version } = JSON.parse(readFileSync(manifest, "utf8"));
  assert.equal(versions.readInstalledPackageVersion("@modelcontextprotocol/sdk"), version);
});

test("a cached clone at another commit is rejected", async () => {
  await ensureTs();
  git(["-c", "advice.detachedHead=false", "checkout", "--quiet", "--detach", "HEAD~1"]);
  try {
    await assert.rejects(ensureTs(), /is at [0-9a-f]{40}, expected/);
  } finally {
    git(["-c", "advice.detachedHead=false", "checkout", "--quiet", "--detach", lock().sha]);
  }
});

test("a clone edited after an earlier check is never bundled", async () => {
  await ensureTs(); // a long-running process checked the clone earlier
  const name = "truncation";
  const out = join(bundleDir(), `${name}.mjs`);
  rmSync(out, { force: true });
  const source = join("src", "modules", `${name}.ts`);
  writeFileSync(join(tsDir(), source), "export const edited = true;\n", { flag: "a" });
  try {
    await assert.rejects(loadModule(name), /not clean/);
    assert.equal(existsSync(out), false, "a bundle was written from the edited clone");
  } finally {
    git(["checkout", "--", source]);
  }
});

test("verifyTs checks the clone without cloning or building", async () => {
  await verifyTs();
  writeFileSync(join(tsDir(), "src", "dirty.txt"), "x");
  try {
    await assert.rejects(verifyTs(), /not clean/);
  } finally {
    rmSync(join(tsDir(), "src", "dirty.txt"));
  }
});

test("ORACLE_SHA overrides the locked SHA and everything keyed by it", () => {
  const other = "0123456789abcdef0123456789abcdef01234567";
  const before = process.env.ORACLE_SHA;
  process.env.ORACLE_SHA = other;
  try {
    assert.equal(lock().sha, other);
    assert.ok(tsDir().endsWith(`ts-${other}`));
    assert.ok(bundleDir().endsWith(`bundle-${other}`));
    process.env.ORACLE_SHA = "main";
    assert.throws(() => lock(), /ORACLE_SHA/);
    process.env.ORACLE_SHA = "";
    assert.equal(lock().sha, JSON.parse(readFileSync(new URL("../oracle.lock", import.meta.url), "utf8")).sha);
  } finally {
    if (before === undefined) delete process.env.ORACLE_SHA;
    else process.env.ORACLE_SHA = before;
  }
});

test("corepack goes through the Windows shell as one command string", async () => {
  // corepack is a .cmd shim on Windows, which starts only through a shell; an argument array with
  // `shell: true` is deprecated (DEP0190, Node 24 and later).
  const { spawnCommand } = await import("../src/fetch-ts.mjs");
  const install = ["pnpm", "--ignore-workspace", "install", "--frozen-lockfile"];
  assert.deepEqual(spawnCommand("corepack", install, "win32"), {
    command: "corepack pnpm --ignore-workspace install --frozen-lockfile",
    args: [],
    shell: true,
  });
  assert.deepEqual(spawnCommand("corepack", install, "linux"), { command: "corepack", args: install, shell: false });
  assert.deepEqual(spawnCommand("git", ["status"], "win32"), { command: "git", args: ["status"], shell: false });
  // The shell would read these characters: such a word never reaches it.
  for (const word of ["a b", "a&b", "a|b", "%PATH%", "a^b", '"a"', "a>b", "!a", "a=b", "a,b", "a;b", ""]) {
    assert.throws(() => spawnCommand("corepack", ["pnpm", word], "win32"), /through the Windows shell/, word);
  }
});

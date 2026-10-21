// The oracle must be built from the pinned TS commit and nothing else: a clone
// that is dirty or at another SHA fails instead of producing vectors.
import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";

import { loadModule } from "../src/bundle.mjs";
import { ensureTs, verifyTs } from "../src/fetch-ts.mjs";
import { bundleDir, lock, tsDir, tsDist } from "../src/paths.mjs";

function git(args) {
  return execFileSync("git", ["-C", tsDir(), ...args], { encoding: "utf8" }).trim();
}

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

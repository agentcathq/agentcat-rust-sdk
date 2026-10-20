// The oracle must be built from the pinned TS commit and nothing else: a clone
// that is dirty or at another SHA fails instead of producing vectors.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";

import { loadModule } from "../src/bundle.mjs";
import { ensureTs } from "../src/fetch-ts.mjs";
import { lock, tsDir, tsDist } from "../src/paths.mjs";

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

test("modules load from the pinned source", async () => {
  const tokens = await loadModule("tokenEstimate");
  assert.equal(typeof tokens.estimateInputTokens, "function");
  assert.equal(tokens.estimateInputTokens({ n: 1.0 }), 2);
});

// run.mjs rejects arguments it does not know: a misspelled --check must not regenerate and exit 0.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

const RUN = join(import.meta.dirname, "..", "src", "vectors", "run.mjs");
const USAGE = "usage: node oracle/src/vectors/run.mjs [--check | --out <dir>]\n";

function runVectors(args) {
  return spawnSync(process.execPath, [RUN, ...args], { encoding: "utf8", timeout: 120_000 });
}

test("run.mjs exits 2 with the usage line on an unknown or misplaced argument, and writes nothing", (t) => {
  const out = mkdtempSync(join(tmpdir(), "vectors-args-"));
  t.after(() => rmSync(out, { recursive: true, force: true }));
  for (const args of [
    ["--chekc", "--out", out],
    ["--out", out, "extra"],
    ["--check", "--out", out],
    ["--out", out, "--out", out],
    ["--check", "--check"],
    ["-c"],
  ]) {
    const res = runVectors(args);
    assert.equal(res.status, 2, `${args.join(" ")}: ${res.stderr}`);
    assert.equal(res.stderr, USAGE, args.join(" "));
    assert.equal(res.stdout, "", args.join(" "));
  }
  assert.deepEqual(readdirSync(out), []);
});

test("run.mjs exits 2 when --out has no directory", () => {
  const res = runVectors(["--out"]);
  assert.equal(res.status, 2, res.stderr);
  assert.equal(res.stderr, "--out needs a directory\n");
});

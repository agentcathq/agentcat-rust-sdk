// run.mjs rejects arguments it does not know: a misspelled --check must not regenerate and exit 0.
// checkSet holds every vector set to the rules the Rust tests and the drift comparison rely on.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { lock } from "../src/paths.mjs";
import { checkSet } from "../src/vectors/write.mjs";

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

const GEN = "oracle/src/vectors/gen/t.mjs";

test("checkSet rejects an empty set", () => {
  assert.throws(() => checkSet("t", []), { message: `${GEN}: the set has no cases` });
});

test("checkSet rejects a case that is not an object or has no name", () => {
  assert.throws(() => checkSet("t", [{ name: "a" }, { input: 1 }]), {
    message: `${GEN}: case #1: "name" must be a non-empty, well-formed string`,
  });
  for (const name of ["", "a\ud83d", 1]) {
    assert.throws(() => checkSet("t", [{ name }]), {
      message: `${GEN}: case #0: "name" must be a non-empty, well-formed string`,
    });
  }
  assert.throws(() => checkSet("t", ["a"]), { message: `${GEN}: case #0: not an object` });
});

test("checkSet rejects a duplicate case name and names it", () => {
  assert.throws(() => checkSet("t", [{ name: "a" }, { name: "b" }, { name: "a" }]), {
    message: `${GEN}: case "a" (#2): the name is already used by case #0`,
  });
});

test("checkSet rejects a ts_test that is not a non-empty array of strings", () => {
  for (const ts_test of ["x", [], [""], ["k", 1]]) {
    assert.throws(() => checkSet("t", [{ name: "a", ts_test }]), {
      message: `${GEN}: case "a" (#0): "ts_test" must be a non-empty array of non-empty strings`,
    }, JSON.stringify(ts_test));
  }
  checkSet("t", [{ name: "a", ts_test: ["v1|src/tests/x.test.ts|y"] }, { name: "b" }]);
});

test("checkSet names the generator and the case when a value cannot be encoded", () => {
  assert.throws(() => checkSet("t", [{ name: "a" }, { name: "b", input: { x: [undefined] } }]), {
    message: `${GEN}: case "b" (#1): strict-json: $.cases[1].input.x[0]: undefined`,
  });
});

test("checkSet accepts a file of exactly 2 MiB and rejects one byte more", () => {
  const LIMIT = 2 * 1024 * 1024;
  const file = (value) =>
    JSON.stringify({ ts_sha: lock().sha, generator: GEN, cases: [{ name: "big", value }] }, null, 2) + "\n";
  const fits = "x".repeat(LIMIT - Buffer.byteLength(file("")));
  assert.equal(Buffer.byteLength(file(fits)), LIMIT);
  checkSet("t", [{ name: "big", value: fits }]);
  assert.throws(() => checkSet("t", [{ name: "big", value: `${fits}x` }]), {
    message: `${GEN}: the file is ${LIMIT + 1} bytes, over the limit of ${LIMIT}; the largest case is "big" (#0)`,
  });
});

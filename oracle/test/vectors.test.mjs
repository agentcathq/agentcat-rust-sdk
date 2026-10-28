// run.mjs rejects arguments it does not know: a misspelled --check must not regenerate and exit 0.
// checkSet holds every vector set to the rules the Rust tests and the drift comparison rely on.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { lock } from "../src/paths.mjs";
import { checkSet, writeAll } from "../src/vectors/write.mjs";

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

test("checkSet names the set and the index of a hole in cases", () => {
  // eslint-disable-next-line no-sparse-arrays
  const holey = [{ name: "a" }, , { name: "c" }];
  assert.equal(1 in holey, false);
  assert.throws(() => checkSet("t", holey), { message: `${GEN}: case #1: a hole in the set (no case there)` });
  assert.throws(() => checkSet("t", [{ name: "a" }, undefined]), { message: `${GEN}: case #1: not an object` });
});

test("checkSet names the path of a cyclic value", () => {
  const input = { list: [] };
  input.list.push(input);
  assert.throws(() => checkSet("t", [{ name: "a" }, { name: "loop", input }]), {
    message: `${GEN}: case "loop" (#1): strict-json: $.cases[1].input.list[0]: cyclic reference to $.cases[1].input`,
  });
});

test("checkSet rejects an array with a property that is not an index", () => {
  const list = [1];
  list.note = "x";
  assert.throws(() => checkSet("t", [{ name: "a", input: { list } }]), {
    message: `${GEN}: case "a" (#0): strict-json: $.cases[0].input.list: array property "note"`,
  });
});

test("checkSet accepts a $repeat macro in a case input only", () => {
  checkSet("t", [{ name: "a", input: { s: { $repeat: ["ab", 3] }, nested: [{ $repeat: ["x", 1] }] } }]);
  for (const field of ["output", "event", "logs"]) {
    assert.throws(() => checkSet("t", [{ name: "a" }, { name: "b", [field]: { s: { $repeat: ["ab", 3] } } }]), {
      message: `${GEN}: case "b" (#1): strict-json: $.cases[1].${field}.s: $repeat outside "input"`,
    }, field);
  }
  assert.throws(() => checkSet("t", [{ name: "a", input: 1, output: [{ $repeat: ["", 0] }] }]), {
    message: `${GEN}: case "a" (#0): strict-json: $.cases[0].output[0]: $repeat outside "input"`,
  });
});

/** A generator directory holding one file per `{name: casesSource}` entry. */
function genDirWith(t, files) {
  const dir = mkdtempSync(join(tmpdir(), "vectors-gen-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  for (const [name, cases] of Object.entries(files)) {
    writeFileSync(join(dir, `${name}.mjs`), `export async function generate() { return { cases: ${cases} }; }\n`);
  }
  return dir;
}

test("writeAll reads the generators from genDir and names that directory in each file", async (t) => {
  const genDir = genDirWith(t, { alpha: '[{ name: "a" }]', beta: '[{ name: "b" }, { name: "c" }]' });
  const outDir = join(mkdtempSync(join(tmpdir(), "vectors-out-")), "out");
  t.after(() => rmSync(join(outDir, ".."), { recursive: true, force: true }));
  assert.deepEqual(await writeAll({ outDir, genDir }), { alpha: 1, beta: 2 });
  const file = JSON.parse(readFileSync(join(outDir, "beta.json"), "utf8"));
  assert.equal(file.generator, `${genDir.split("\\").join("/")}/beta.mjs`);
  assert.equal(file.ts_sha, lock().sha);
  // Inside the repository, the path is relative to its root, as in every committed file.
  const committed = JSON.parse(readFileSync(join(import.meta.dirname, "..", "..", "agentcat-sdk", "tests", "vectors", "queue.json"), "utf8"));
  assert.equal(committed.generator, "oracle/src/vectors/gen/queue.mjs");
});

test("writeAll checks every set before it writes any file", async (t) => {
  // "alpha" is valid and sorts first; "beta" has a duplicate name. Nothing may be written.
  const genDir = genDirWith(t, { alpha: '[{ name: "a" }]', beta: '[{ name: "x" }, { name: "x" }]' });
  const outDir = join(mkdtempSync(join(tmpdir(), "vectors-out-")), "out");
  t.after(() => rmSync(join(outDir, ".."), { recursive: true, force: true }));
  await assert.rejects(writeAll({ outDir, genDir }), /beta\.mjs: case "x" \(#1\): the name is already used by case #0/);
  assert.equal(existsSync(outDir), false, "no output directory");
});

test("a generator that sets a deviation other than RD-24 does not round-trip its output through JSON", () => {
  const dir = join(import.meta.dirname, "..", "src", "vectors", "gen");
  let checked = 0;
  for (const file of readdirSync(dir).filter((f) => f.endsWith(".mjs"))) {
    const lines = readFileSync(join(dir, file), "utf8").split("\n");
    const sets = lines.some((line) => /deviation\W+"RD-(\d+)"/.exec(line)?.[1] !== undefined && !line.includes('"RD-24"') && !line.trimStart().startsWith("//"));
    if (!sets) continue;
    checked += 1;
    lines.forEach((line, i) => {
      const where = `${file}:${i + 1}: ${line.trim()}`;
      // The JSON round trip (JSON.parse(JSON.stringify(x))) drops a __proto__ key: only RD-24 rules that.
      const helper = /^\s*const (plain|copyJson) =/.test(line);
      if (line.includes("JSON.parse(JSON.stringify(") && !helper) {
        assert.ok(line.includes('"RD-24"'), `a round trip that RD-24 does not guard: ${where}`);
      }
      // A helper that does the round trip, on an output, must be skipped for a case with a deviation.
      if (/\b(plain|copyJson)\(\s*output\b/.test(line)) {
        assert.ok(line.includes("deviation"), `the round trip of an output ignores the deviation: ${where}`);
      }
    });
  }
  assert.ok(checked >= 3, `only ${checked} generators set a deviation`);
});

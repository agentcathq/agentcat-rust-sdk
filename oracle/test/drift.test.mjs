// The nightly drift comparison looks only at the `cases` and `tests` arrays (R-6).
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

import { compare } from "../src/drift/compare.mjs";

function setup(t, { committed, fresh }) {
  const dir = mkdtempSync(join(tmpdir(), "drift-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const put = (sub, file, doc) => {
    mkdirSync(join(dir, sub), { recursive: true });
    writeFileSync(join(dir, sub, file), JSON.stringify(doc));
  };
  put("cv", "constants.json", committed.vectors);
  put("cp", "ts-tests.json", committed.list);
  put("fv", "constants.json", fresh.vectors);
  put("fp", "ts-tests.json", fresh.list);
  return {
    vectorsDir: join(dir, "fv"),
    parityDir: join(dir, "fp"),
    committedVectors: join(dir, "cv"),
    committedParity: join(dir, "cp", "ts-tests.json"),
  };
}

const t1 = { key: "v1|a.test.ts|one", project: "v1", file: "a.test.ts", title: "one" };

test("ts_sha, count and generator differences are not drift", (t) => {
  const dirs = setup(t, {
    committed: { vectors: { ts_sha: "a", cases: [{ name: "x", v: 1 }] }, list: { ts_sha: "a", count: 1, tests: [t1] } },
    fresh: { vectors: { ts_sha: "b", cases: [{ name: "x", v: 1 }] }, list: { ts_sha: "b", count: 9, tests: [t1] } },
  });
  assert.deepEqual(compare(dirs), []);
});

test("a changed case and a new TS test are drift", (t) => {
  const t2 = { ...t1, key: "v1|a.test.ts|two", title: "two" };
  const dirs = setup(t, {
    committed: { vectors: { cases: [{ name: "x", v: 1 }] }, list: { tests: [t1] } },
    fresh: { vectors: { cases: [{ name: "x", v: 2 }] }, list: { tests: [t1, t2] } },
  });
  const out = compare(dirs).join("\n");
  assert.match(out, /cases that changed \(1\)/);
  assert.match(out, /tests added to TS \(1\)/);
  assert.match(out, /v1\|a\.test\.ts\|two/);
});

test("the CLI exits 2 with a one-line message on a malformed file", (t) => {
  const dir = mkdtempSync(join(tmpdir(), "drift-cli-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  mkdirSync(join(dir, "v"));
  mkdirSync(join(dir, "p"));
  writeFileSync(join(dir, "v", "constants.json"), "{ not json");
  const cli = fileURLToPath(new URL("../src/drift/compare.mjs", import.meta.url));
  const res = spawnSync(process.execPath, [cli, join(dir, "v"), join(dir, "p")], { encoding: "utf8" });
  assert.equal(res.status, 2);
  assert.match(res.stdout, /^- The drift comparison could not run: .+\n$/);
});

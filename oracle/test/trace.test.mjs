// Every entry point that builds or runs the oracle calls traceOverride(): with ORACLE_SHA set it writes one
// trace line to stderr; without ORACLE_SHA it writes none.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";

import { lock } from "../src/paths.mjs";

const SRC = join(import.meta.dirname, "..", "src");
const RUN = join(SRC, "vectors", "run.mjs");
const LIST_TESTS = join(SRC, "parity", "list-tests.mjs");

const { sha } = lock();
const TRACE = `oracle: ORACLE_SHA=${sha} overrides oracle/oracle.lock (${sha})\n`;

/** The entry points, each with an argument list that returns quickly and reaches its traceOverride() call. */
function entries(t) {
  // list-tests.mjs: --init-manifest in a directory with no ts-tests.json fails after the trace, before any vitest run.
  const empty = mkdtempSync(join(tmpdir(), "trace-"));
  t.after(() => rmSync(empty, { recursive: true, force: true }));
  return [
    ["run.mjs --check", RUN, ["--check"]],
    ["list-tests.mjs --init-manifest", LIST_TESTS, ["--init-manifest", "--out", empty]],
  ];
}

function withEnv(set) {
  const env = { ...process.env };
  delete env.ORACLE_SHA;
  if (set !== undefined) env.ORACLE_SHA = set;
  return env;
}

test("with ORACLE_SHA set, each entry point writes the trace line once", { timeout: 60_000 }, (t) => {
  for (const [label, script, args] of entries(t)) {
    const res = spawnSync(process.execPath, [script, ...args], { encoding: "utf8", env: withEnv(sha), timeout: 55_000 });
    assert.equal(res.stderr.split(TRACE).length - 1, 1, `${label}: ${res.stderr}`);
  }
});

test("with ORACLE_SHA absent or empty, no entry point writes a trace line", { timeout: 60_000 }, (t) => {
  for (const [label, script, args] of entries(t)) {
    for (const value of [undefined, ""]) {
      const res = spawnSync(process.execPath, [script, ...args], { encoding: "utf8", env: withEnv(value), timeout: 55_000 });
      assert.doesNotMatch(res.stderr, /ORACLE_SHA/, `${label} (ORACLE_SHA ${JSON.stringify(value)}): ${res.stderr}`);
    }
  }
});

test("a malformed ORACLE_SHA stops an entry point before it does any work", { timeout: 60_000 }, (t) => {
  for (const [label, script, args] of entries(t)) {
    const res = spawnSync(process.execPath, [script, ...args], { encoding: "utf8", env: withEnv("abc"), timeout: 55_000 });
    assert.notEqual(res.status, 0, label);
    assert.match(res.stderr, /ORACLE_SHA must be a full 40-character lowercase commit SHA/, label);
  }
});

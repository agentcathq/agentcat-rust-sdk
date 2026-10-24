// The reference servers' Task 13 additions: the redactEvent profile, the paged tools/list
// (HARNESS_PAGE_SIZE) and the Rust server's HARNESS_CANARY values.
import assert from "node:assert/strict";
import { join } from "node:path";
import { before, test } from "node:test";

import { ensureTs } from "oracle/fetch-ts";

import { BASIC, CONTRACT, RUST_SERVER, TS_SERVER, TS_V2_SERVER, driver, outFile, readJson, run, tempDir, writeJson } from "./support.mjs";

before(async () => {
  await ensureTs();
});

const PROJECT_ID = readJson(BASIC).projectId;

/** A server's environment for a run that must stop before serving (a refused profile or variable). */
function refusedEnv(t, extra) {
  const env = { ...process.env, AGENTCAT_API_URL: "http://127.0.0.1:9", DISABLE_DIAGNOSTICS: "true", HOME: tempDir(t), HARNESS_PROJECT_ID: PROJECT_ID };
  for (const key of ["HARNESS_NO_TRACK", "HARNESS_OPTIONS", "HARNESS_CANARY", "HARNESS_PAGE_SIZE"]) delete env[key];
  return { ...env, ...extra };
}

/** Starts `server` over stdio with `env`; resolves its exit code and output. */
function start(server, env) {
  const [cmd, ...args] = server;
  return run(cmd, [...args, "stdio"], { env, timeoutMs: 60_000 });
}

const BAD_REDACT_EVENT = [
  [{ returns: { resourceName: "x" } }, /redactEvent: "returns" must be null/],
  [{ merge: { resourceName: "x", tags: {} } }, /redactEvent: unknown merge field "tags"/],
  [{ merge: ["resourceName"] }, /redactEvent: "merge" must be an object/],
  [{ merge: {}, returns: null }, /redactEvent: needs exactly one of returns, throws, merge/],
  [{ replace: "x" }, /redactEvent: unknown key "replace"/],
];

for (const [name, server] of [
  ["ts-v1", TS_SERVER],
  ["ts-v2", TS_V2_SERVER],
  ["rust", RUST_SERVER],
]) {
  test(`${name} exits 2 on an invalid redactEvent behavior`, { timeout: 120_000 }, async (t) => {
    for (const [behavior, message] of BAD_REDACT_EVENT) {
      const res = await start(server, refusedEnv(t, { HARNESS_OPTIONS: JSON.stringify({ redactEvent: behavior }) }));
      assert.equal(res.code, 2, `${JSON.stringify(behavior)}: ${res.stderr}`);
      assert.match(res.stderr, /HARNESS_OPTIONS: /);
      assert.match(res.stderr, message);
      assert.equal(res.stdout, "");
    }
  });

  test(`${name} exits 2 on a HARNESS_PAGE_SIZE that is not a positive integer`, { timeout: 120_000 }, async (t) => {
    for (const size of ["0", "-1", "1.5", "x", "03"]) {
      const res = await start(server, refusedEnv(t, { HARNESS_PAGE_SIZE: size }));
      assert.equal(res.code, 2, `${size}: ${res.stderr}`);
      assert.match(res.stderr, /HARNESS_PAGE_SIZE: must be a positive integer/);
    }
  });
}

test("the Rust server exits 2 on a HARNESS_CANARY other than 1, call or event", async (t) => {
  for (const canary of ["2", "calls", "EVENT", " 1"]) {
    const res = await start(RUST_SERVER, refusedEnv(t, { HARNESS_CANARY: canary }));
    assert.equal(res.code, 2, `${JSON.stringify(canary)}: ${res.stderr}`);
    assert.match(res.stderr, /HARNESS_CANARY: must be 1, call or event/);
  }
});

test("each server accepts a redactEvent profile of every kind (an untracked run serves it)", { timeout: 120_000 }, async (t) => {
  const merge = { resourceName: "renamed", sessionId: "merged", eventType: "x", userIntent: "why", properties: { a: 1 } };
  for (const redactEvent of [{ returns: null }, { throws: "no" }, { merge, sleepMs: 1 }]) {
    const path = writeJson(join(tempDir(t), "redact.json"), {
      name: "redactok",
      projectId: PROJECT_ID,
      options: { redactEvent },
      expectPublishes: 0,
      sessions: [{ steps: [{ op: "callTool", name: "echo", args: { msg: "hi" } }] }],
    });
    for (const [label, server] of [["v1", TS_SERVER], ["v2", TS_V2_SERVER], ["rust", RUST_SERVER]]) {
      const res = await driver(path, `redactok-${label}`, "stdio", [], { server, env: { HARNESS_NO_TRACK: "1" } });
      assert.equal(res.code, 0, `${label} ${JSON.stringify(redactEvent)}: ${res.stderr}`);
    }
  }
});

// tools/list in pages of 3: cursor = the decimal start index, nextCursor while tools remain.
const NAMES = readJson(CONTRACT).map((tool) => tool.name);
const PAGES = [
  [undefined, NAMES.slice(0, 3), "3"],
  ["3", NAMES.slice(3, 6), "6"],
  ["9", NAMES.slice(9), undefined],
];

for (const [name, server, transport, lane] of [
  ["ts-v1", TS_SERVER, "stdio", "2025-11-25"],
  ["ts-v2", TS_V2_SERVER, "stdio", "2025-11-25"],
  ["ts-v2", TS_V2_SERVER, "http", "2026-07-28"],
  ["rust", RUST_SERVER, "stdio", "2025-11-25"],
  ["rust", RUST_SERVER, "http", "2026-07-28"],
]) {
  test(`${name} pages tools/list by HARNESS_PAGE_SIZE (${transport} ${lane}, untracked)`, { timeout: 120_000 }, async (t) => {
    assert.equal(NAMES.length, 10, "the contract has 10 tools: pages of 3, 3, 3 and 1");
    const path = writeJson(join(tempDir(t), "pages.json"), {
      name: "pages",
      projectId: PROJECT_ID,
      options: {},
      pageSize: 3,
      expectPublishes: 0,
      sessions: [{ steps: PAGES.map(([cursor]) => (cursor === undefined ? { op: "listTools" } : { op: "listTools", cursor })) }],
    });
    const label = `pages-${name}-${lane}`;
    const res = await driver(path, label, transport, ["--lane", lane], { server, env: { HARNESS_NO_TRACK: "1" } });
    assert.equal(res.code, 0, res.stderr);
    const lists = readJson(outFile("pages", label, transport, "transcript")).filter((e) => e.op === "listTools");
    assert.deepEqual(
      lists.map((e) => [e.request?.cursor, e.result.tools.map((tool) => tool.name), e.result.nextCursor]),
      PAGES,
    );
  });
}

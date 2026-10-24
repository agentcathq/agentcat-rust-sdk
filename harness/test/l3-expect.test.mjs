// l3-expect.mjs: the A/A and canary checks of the L3 gate, on hand-made matrix.json files.
import assert from "node:assert/strict";
import { join } from "node:path";
import { test } from "node:test";

import { HARNESS, run, tempDir, writeJson } from "./support.mjs";

const L3 = join(HARNESS, "src", "l3-expect.mjs");
const TOOL_PATH = "$.delta.transcript[1].result.tools[0].inputSchema.properties.context.description";

const NO_EVENTS = { total: 0, ruled: 0, unexpected: 0 };
const EVENT_PATH = "$.events[3].body.error.type";

const combo = (transport, { exitCodes, delta, differences, events, eventDifferences } = {}) => {
  const c = {
    scenario: "basic",
    transport,
    lane: "2025-11-25",
    servers: ["rust", "rust"],
    exitCodes: exitCodes ?? { "rust-2025-11-25-u": 0, "rust-2025-11-25-t": 0, "rust-2025-11-25-u-b": 0, "rust-2025-11-25-t-b": 0 },
  };
  if (delta !== null) c.delta = delta ?? { total: 0, ruled: 0, unexpected: 0 };
  if (differences) c.differences = differences;
  if (events !== null) c.events = events ?? NO_EVENTS;
  if (eventDifferences) c.eventDifferences = eventDifferences;
  return c;
};
const caught = (transport, paths = [TOOL_PATH]) =>
  combo(transport, {
    delta: { total: paths.length, ruled: 0, unexpected: paths.length },
    differences: paths.map((path) => ({ path, ts: 'add "a"', rust: 'add "canary"', ruledBy: [] })),
  });

async function check(t, mode, doc) {
  const file = writeJson(join(tempDir(t), "matrix.json"), doc);
  return run(process.execPath, [L3, mode, file]);
}

test("none: passes when every combination has no difference and every driver exited 0", async (t) => {
  const res = await check(t, "none", [combo("stdio"), combo("http")]);
  assert.equal(res.code, 0, res.stdout + res.stderr);
  assert.match(res.stdout, /L3 A\/A ok: 2 combination/);
});

test("none: a ruled difference, a failed driver, a missing delta or no combination at all fails", async (t) => {
  for (const [doc, why] of [
    [[combo("stdio"), combo("http", { delta: { total: 1, ruled: 1, unexpected: 0 } })], /basic http 2025-11-25: delta/],
    [[combo("stdio", { exitCodes: { a: 0, b: 1 } })], /basic stdio 2025-11-25: driver exit/],
    [[combo("stdio", { delta: null })], /no delta/],
    [[], /no combination ran/],
  ]) {
    const res = await check(t, "none", doc);
    assert.equal(res.code, 1, res.stdout + res.stderr);
    assert.match(res.stderr, why);
  }
});

test("caught: passes when every combination has unruled differences at the canary paths only", async (t) => {
  const res = await check(t, "caught", [caught("stdio"), caught("http", [TOOL_PATH, "$.delta.transcript[1].result.tools[7].description"])]);
  assert.equal(res.code, 0, res.stdout + res.stderr);
  assert.match(res.stdout, /L3 canary ok: 2 combination/);
});

test("caught: a combination without an unruled difference, one outside the canary paths, or a failed driver fails", async (t) => {
  for (const [doc, why] of [
    [[caught("stdio"), combo("http")], /basic http 2025-11-25: no unruled difference/],
    [[caught("stdio"), combo("http", { delta: { total: 2, ruled: 2, unexpected: 0 } })], /no unruled difference/],
    [[caught("stdio", [TOOL_PATH, "$.delta.transcript[4].result"])], /outside the canary paths: \$\.delta\.transcript\[4\]\.result/],
    [[{ ...caught("stdio"), exitCodes: { a: 1 } }], /driver exit/],
    [[], /no combination ran/],
  ]) {
    const res = await check(t, "caught", doc);
    assert.equal(res.code, 1, res.stdout + res.stderr);
    assert.match(res.stderr, why);
  }
});

test("a bad mode, an extra argument or an unreadable file exits 2", async (t) => {
  const dir = tempDir(t);
  const file = writeJson(join(dir, "matrix.json"), [combo("stdio")]);
  for (const args of [[], ["both", file], ["none", file, "extra"], ["none", join(dir, "missing.json")]]) {
    const res = await run(process.execPath, [L3, ...args]);
    assert.equal(res.code, 2, `${JSON.stringify(args)}: ${res.stdout}${res.stderr}`);
  }
  const notArray = writeJson(join(dir, "object.json"), { scenario: "basic" });
  assert.equal((await run(process.execPath, [L3, "none", notArray])).code, 2);
});

test("none: an event difference, ruled or not, or a combination without an event comparison fails", async (t) => {
  for (const [doc, why] of [
    [[combo("stdio"), combo("http", { events: { total: 1, ruled: 1, unexpected: 0 } })], /basic http 2025-11-25: events \{"total":1/],
    [[combo("stdio", { events: null })], /basic stdio 2025-11-25: no event comparison/],
  ]) {
    const res = await check(t, "none", doc);
    assert.equal(res.code, 1, res.stdout + res.stderr);
    assert.match(res.stderr, why);
  }
});

test("caught: an event difference, or a combination without an event comparison, fails", async (t) => {
  const withEvent = {
    ...caught("http"),
    events: { total: 1, ruled: 0, unexpected: 1 },
    eventDifferences: [{ path: EVENT_PATH, a: "x", b: "y", ruledBy: [] }],
  };
  const noEvents = { ...caught("http") };
  delete noEvents.events;
  for (const [doc, why] of [
    [[caught("stdio"), withEvent], /basic http 2025-11-25: an event difference: \$\.events\[3\]\.body\.error\.type/],
    [[caught("stdio"), noEvents], /basic http 2025-11-25: no event comparison/],
  ]) {
    const res = await check(t, "caught", doc);
    assert.equal(res.code, 1, res.stdout + res.stderr);
    assert.match(res.stderr, why);
  }
});

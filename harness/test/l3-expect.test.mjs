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
  for (const args of [[], ["both", file], ["caught-calls", file], ["none", file, "extra"], ["none", join(dir, "missing.json")]]) {
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

// ---------------------------------------------------------------------------
// Task 13: the call canary (HARNESS_CANARY=call) and the event canary (HARNESS_CANARY=event).
// ---------------------------------------------------------------------------
const MINT_PATH = "$.delta.transcript[2].result.content[0]";
const RESOURCE_PATH = "$.events[0].body.resource_name";

const MINT_TS = 'add {"type":"text","text":"[session_id issued — see this tool\'s session_id parameter description]\\nsession_id: <ses#1>\\nT...';
const MINT_RUST = MINT_TS.replace("[session_id issued", "[session_id lssued");
// The entries of a matrix run carry the whole text beside the clipped one (delta.mjs compareDeltas).
const MINT_TS_FULL = `${MINT_TS.slice(0, -3)}ail of the mint-back text, past the clip"}`;
const MINT_RUST_FULL = MINT_TS_FULL.replace("[session_id issued", "[session_id lssued");
const callCaught = (transport, paths = [MINT_PATH], extra = {}) =>
  combo(transport, {
    delta: { total: paths.length, ruled: 0, unexpected: paths.length },
    differences: paths.map((path) => ({ path, ts: MINT_TS, rust: MINT_RUST, tsText: MINT_TS_FULL, rustText: MINT_RUST_FULL, ruledBy: [], ...extra })),
  });
const eventCaught = (transport, paths = [RESOURCE_PATH]) =>
  combo(transport, {
    events: { total: paths.length, ruled: 0, unexpected: paths.length },
    eventDifferences: paths.map((path) => ({ path, a: "echo", b: "canary", ruledBy: [] })),
  });

test("caught-call: passes when every combination has unruled differences in the first content block of results only", async (t) => {
  // A call whose untracked answer was an error shows the whole tracked result as added.
  const paths = [MINT_PATH, "$.delta.transcript[9].result", "$.delta.transcript[11].result.content[0].text"];
  const res = await check(t, "caught-call", [callCaught("stdio"), callCaught("http", paths)]);
  assert.equal(res.code, 0, res.stdout + res.stderr);
  assert.match(res.stdout, /L3 call canary ok: 2 combination/);
});

test("caught-call: a difference elsewhere, an event difference, no unruled difference or a failed driver fails", async (t) => {
  const withEvent = { ...callCaught("http"), events: { total: 1, ruled: 0, unexpected: 1 }, eventDifferences: [{ path: RESOURCE_PATH, a: "x", b: "y", ruledBy: [] }] };
  for (const [doc, why] of [
    [[callCaught("stdio"), callCaught("http", [MINT_PATH, TOOL_PATH])], /basic http 2025-11-25: a difference outside the canary paths: \$\.delta\.transcript\[1\]\.result\.tools/],
    [[callCaught("stdio", ["$.delta.transcript[2].result.content[1]"])], /outside the canary paths: \$\.delta\.transcript\[2\]\.result\.content\[1\]/],
    [[caught("stdio", [MINT_PATH])], /stdio 2025-11-25: not the call canary's change at \$\.delta\.transcript\[2\]\.result\.content\[0\]/],
    [[callCaught("stdio", ["$.delta.transcript[9].structuredContent"])], /outside the canary paths: \$\.delta\.transcript\[9\]\.structuredContent/],
    [[callCaught("stdio"), withEvent], /basic http 2025-11-25: an event difference: \$\.events\[0\]\.body\.resource_name/],
    [[callCaught("stdio"), combo("http")], /basic http 2025-11-25: no unruled difference/],
    [[{ ...callCaught("stdio"), exitCodes: { a: 1 } }], /driver exit/],
    [[], /no combination ran/],
  ]) {
    const res = await check(t, "caught-call", doc);
    assert.equal(res.code, 1, res.stdout + res.stderr);
    assert.match(res.stderr, why);
  }
});

test("caught-call: compares the whole text, so a change past the clipped summary fails", async (t) => {
  // The clipped summaries look like the canary's change alone; the full text has a second change in its tail.
  const tail = callCaught("stdio", [MINT_PATH], { rustText: MINT_RUST_FULL.replace("past the clip", "past the clipped part") });
  const noText = callCaught("http", [MINT_PATH], { tsText: undefined, rustText: undefined });
  for (const doc of [[tail], [noText]]) {
    const res = await check(t, "caught-call", doc);
    assert.equal(res.code, 1, res.stdout + res.stderr);
    assert.match(res.stderr, /: not the call canary's change at \$\.delta\.transcript\[2\]\.result\.content\[0\]/);
  }
});

test("caught-event: passes when every combination has unruled event differences at resource_name only and no delta difference", async (t) => {
  const res = await check(t, "caught-event", [eventCaught("stdio"), eventCaught("http", [RESOURCE_PATH, "$.events[9].body.resource_name"])]);
  assert.equal(res.code, 0, res.stdout + res.stderr);
  assert.match(res.stdout, /L3 event canary ok: 2 combination/);
});

test("caught-event: a delta difference, an event difference elsewhere or with another value, no unruled event difference or a failed driver fails", async (t) => {
  const withDelta = { ...eventCaught("http"), delta: { total: 1, ruled: 1, unexpected: 0 } };
  for (const [doc, why] of [
    [[eventCaught("stdio"), withDelta], /basic http 2025-11-25: a delta difference: delta \{"total":1/],
    [[eventCaught("stdio", [RESOURCE_PATH, EVENT_PATH])], /an event difference outside the canary paths: \$\.events\[3\]\.body\.error\.type/],
    // The right path, but not the canary's value on the second (Rust) server.
    [
      [eventCaught("stdio"), combo("http", { events: { total: 1, ruled: 0, unexpected: 1 }, eventDifferences: [{ path: RESOURCE_PATH, a: "echo", b: "renamed", ruledBy: [] }] })],
      /basic http 2025-11-25: not the event canary's change at \$\.events\[0\]\.body\.resource_name: a "echo" b "renamed"/,
    ],
    [
      [combo("stdio", { events: { total: 1, ruled: 0, unexpected: 1 }, eventDifferences: [{ path: RESOURCE_PATH, a: "canary", b: "echo", ruledBy: [] }] })],
      /not the event canary's change at \$\.events\[0\]\.body\.resource_name: a "canary" b "echo"/,
    ],
    [[eventCaught("stdio"), combo("http")], /basic http 2025-11-25: no unruled event difference/],
    [[eventCaught("stdio"), combo("http", { events: { total: 1, ruled: 1, unexpected: 0 } })], /no unruled event difference/],
    [[{ ...eventCaught("stdio"), exitCodes: { a: 1 } }], /driver exit/],
    [[eventCaught("stdio", [RESOURCE_PATH]), { ...eventCaught("http"), events: undefined }], /no event comparison/],
    [[], /no combination ran/],
  ]) {
    const res = await check(t, "caught-event", doc);
    assert.equal(res.code, 1, res.stdout + res.stderr);
    assert.match(res.stderr, why);
  }
});

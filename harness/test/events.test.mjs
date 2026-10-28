// The event comparison and the pinned values of a ruling (ts, rust, rustKeys), on crafted documents (no
// driver, no server).
import assert from "node:assert/strict";
import { join } from "node:path";
import { test } from "node:test";

import { compareEvents } from "../src/events.mjs";
import { loadRulings, matchRulings, rulingLines, rulingsFor } from "../src/rulings.mjs";
import { HARNESS, diff, run, tempDir, writeJson } from "./support.mjs";

const KEYS_PATH = "\\$\\.events\\[\\d+\\]\\.body\\.parameters\\.extra\\.<keys>";
const META_PATH = "\\$\\.events\\[\\d+\\]\\.body\\.parameters\\.request\\.params\\._meta";

const failNow = (message) => {
  throw new Error(message);
};

/** The rulings of `list`, loaded from a file as matrix.mjs loads them. */
function rulingsOf(t, list) {
  return loadRulings(writeJson(join(tempDir(t), "rulings-events.json"), list), failNow);
}

/** A normalized document of one tool-call event with these `extra` keys and this request `_meta`. */
function doc(keys, meta) {
  const params = { name: "echo", arguments: { msg: "hi" } };
  if (meta !== undefined) params._meta = meta;
  return { events: [{ body: { parameters: { extra: { "<keys>": keys }, request: { method: "tools/call", params } } } }], nonPublishRequests: [] };
}

const ENVELOPE = {
  "io.modelcontextprotocol/protocolVersion": "2026-07-28",
  "io.modelcontextprotocol/clientInfo": { name: "harness-client-a", version: "1.0.0" },
};

/** The entries of compareEvents(ts, rust) as [path, ruledBy] pairs. */
const ruled = (ts, rust, rulings) => compareEvents(ts, rust, rulings).map((d) => [d.path, d.ruledBy]);

test("a ruling that pins ts and rust rules its exact two values only; another value on either side is unruled", (t) => {
  const rulings = rulingsOf(t, [{ path: KEYS_PATH, ts: "http,mcpReq", rust: "requestId,requestInfo", reason: "test", deviation: "RD-07" }]);
  const at = "$.events[0].body.parameters.extra.<keys>";
  assert.deepEqual(ruled(doc("http,mcpReq"), doc("requestId,requestInfo"), rulings), [[at, ["RD-07"]]]);
  assert.equal(rulings[0].matched, 1);
  // An extra key on the Rust side, a missing one, or another TS set: the same path, no ruling.
  assert.deepEqual(ruled(doc("http,mcpReq"), doc("requestId,requestInfo,surprise"), rulings), [[at, []]]);
  assert.deepEqual(ruled(doc("http,mcpReq"), doc("requestId"), rulings), [[at, []]]);
  assert.deepEqual(ruled(doc("mcpReq"), doc("requestId,requestInfo"), rulings), [[at, []]]);
  assert.equal(rulings[0].matched, 1);
});

test("a pinned value compares as JSON, whatever the key order; a ruling without pins rules any values", (t) => {
  const rulings = rulingsOf(t, [
    { path: "\\$\\.events\\[0\\]\\.body\\.x", ts: { a: 1, b: [1, 2] }, rust: null, reason: "pinned", deviation: "RD-90" },
    { path: "\\$\\.events\\[0\\]\\.body\\.y", reason: "not pinned", deviation: "RD-91" },
  ]);
  const ts = { events: [{ body: { x: { b: [1, 2], a: 1 }, y: 1 } }] };
  const rust = { events: [{ body: { x: null, y: "anything" } }] };
  assert.deepEqual(ruled(ts, rust, rulings), [
    ["$.events[0].body.x", ["RD-90"]],
    ["$.events[0].body.y", ["RD-91"]],
  ]);
  // An array in another order is another value.
  const reordered = { events: [{ body: { x: { a: 1, b: [2, 1] }, y: 1 } }] };
  assert.deepEqual(ruled(reordered, rust, rulings)[0], ["$.events[0].body.x", []]);
});

test("rustKeys: a Rust _meta of io.modelcontextprotocol/* keys alone is ruled; another key, or an empty _meta, is unruled", (t) => {
  const rulings = rulingsOf(t, [
    { path: META_PATH, ts: "<absent>", rustKeys: "io\\.modelcontextprotocol/.+", reason: "test", deviation: "RD-13" },
  ]);
  const at = "$.events[0].body.parameters.request.params._meta";
  const keys = "requestId";
  assert.deepEqual(ruled(doc(keys), doc(keys, ENVELOPE), rulings), [[at, ["RD-13"]]]);
  assert.deepEqual(ruled(doc(keys), doc(keys, { ...ENVELOPE, progressToken: 7 }), rulings), [[at, []]]);
  assert.deepEqual(ruled(doc(keys), doc(keys, { "io.modelcontextprotocol/x": 1, "io.modelcontextprotocolX": 2 }), rulings), [[at, []]]);
  assert.deepEqual(ruled(doc(keys), doc(keys, {}), rulings), [[at, []]]);
  assert.deepEqual(ruled(doc(keys), doc(keys, "io.modelcontextprotocol/x"), rulings), [[at, []]]);
  // TS with a _meta of its own: the difference is no longer "<absent>" against an object.
  assert.deepEqual(ruled(doc(keys, { "io.modelcontextprotocol/x": 0 }), doc(keys, { "io.modelcontextprotocol/x": 1 }), rulings), [
    [`${at}.io.modelcontextprotocol/x`, []],
  ]);
  assert.equal(rulings[0].matched, 1);
});

test("harness/rulings-events.json: extra.<keys> and params._meta pass with the known values only", () => {
  const shipped = loadRulings(join(HARNESS, "rulings-events.json"), failNow);
  const at = (combo) => rulingsFor(shipped, { scenario: "basic", ...combo });
  const keysAt = "$.events[0].body.parameters.extra.<keys>";
  const metaAt = "$.events[0].body.parameters.request.params._meta";
  const stdio2025 = at({ transport: "stdio", lane: "2025-11-25" });
  assert.deepEqual(ruled(doc("mcpReq"), doc("requestId"), stdio2025), [[keysAt, ["RD-07"]]]);
  // The key set of another transport or lane, or one key more, is unruled.
  assert.deepEqual(ruled(doc("mcpReq"), doc("_meta,requestId"), stdio2025), [[keysAt, []]]);
  assert.deepEqual(ruled(doc("mcpReq"), doc("requestId,requestInfo"), stdio2025), [[keysAt, []]]);
  assert.deepEqual(ruled(doc("mcpReq"), doc("requestId,surprise"), stdio2025), [[keysAt, []]]);
  assert.deepEqual(ruled(doc("http,mcpReq"), doc("requestId"), stdio2025), [[keysAt, []]]);
  const http2026 = at({ transport: "http", lane: "2026-07-28" });
  const tsKeys = doc("http,mcpReq");
  const rustKeys = "_meta,requestId,requestInfo";
  assert.deepEqual(ruled(tsKeys, doc(rustKeys, ENVELOPE), http2026), [
    [keysAt, ["RD-07"]],
    [metaAt, ["RD-13"]],
  ]);
  // A key outside io.modelcontextprotocol/* in Rust's _meta, or an empty _meta, is unruled.
  assert.deepEqual(ruled(tsKeys, doc(rustKeys, { ...ENVELOPE, progressToken: 7 }), http2026)[1], [metaAt, []]);
  assert.deepEqual(ruled(tsKeys, doc(rustKeys, {}), http2026)[1], [metaAt, []]);
  // The _meta ruling is scoped to 2026-07-28.
  assert.deepEqual(ruled(doc("mcpReq"), doc("requestId", ENVELOPE), stdio2025)[1], [metaAt, []]);
});

test("a pinned ruling never rules a difference without values (a delta difference)", (t) => {
  const [pinned, plain] = rulingsOf(t, [
    { path: "\\$\\.x", ts: 1, reason: "pinned", deviation: "RD-90" },
    { path: "\\$\\.x", reason: "plain", deviation: "RD-91" },
  ]);
  assert.deepEqual(matchRulings([pinned, plain], "$.x").map((r) => r.deviation), ["RD-91"]);
  assert.deepEqual(matchRulings([pinned, plain], "$.x", { a: 1, b: 2 }).map((r) => r.deviation), ["RD-90", "RD-91"]);
});

test("loadRulings rejects a rustKeys that is not a non-empty string or not a valid regex", (t) => {
  const base = { path: "\\$\\.x", reason: "test", deviation: "RD-90" };
  for (const [rustKeys, message] of [
    ["(", /\[0\]: bad rustKeys regex: /],
    ["", /\[0\]: "rustKeys" must be a non-empty regex string/],
    [["a"], /\[0\]: "rustKeys" must be a non-empty regex string/],
  ]) {
    assert.throws(() => rulingsOf(t, [{ ...base, rustKeys }]), message, JSON.stringify(rustKeys));
  }
});

test("loadRulings refuses pin keys when pins are off, as in a delta rulings file", (t) => {
  const base = { path: "\\$\\.x", reason: "test", deviation: "RD-90" };
  const file = (extra) => writeJson(join(tempDir(t), "rulings.json"), [{ ...base, ...extra }]);
  for (const extra of [{ ts: "a" }, { rust: "b" }, { rustKeys: "k.*" }]) {
    const key = Object.keys(extra)[0];
    assert.throws(() => loadRulings(file(extra), failNow, { pins: false }), new RegExp(`\\[0\\]: "${key}" pins a value, which only an event ruling can do`), key);
    assert.equal(loadRulings(file(extra), failNow).length, 1, `${key}: pins stay allowed by default`);
  }
  assert.equal(loadRulings(file({}), failNow, { pins: false }).length, 1);
});

test("delta.mjs and matrix.mjs exit 2 on a pin key in a delta rulings file; matrix.mjs takes it in an event rulings file", async (t) => {
  const dir = tempDir(t);
  const pinned = writeJson(join(dir, "rulings.json"), [{ path: "\\$\\.x", reason: "test", deviation: "RD-90", ts: "a" }]);
  const empty = writeJson(join(dir, "empty.normalized.json"), { transcript: [] });
  let res = await run(process.execPath, [join(HARNESS, "src", "delta.mjs"), empty, empty, empty, empty, "--rulings", pinned]);
  assert.equal(res.code, 2, res.stdout + res.stderr);
  assert.match(res.stderr, /"ts" pins a value/);
  const matrix = join(HARNESS, "src", "matrix.mjs");
  res = await run(process.execPath, [matrix, "--servers", "ts-v2,ts-v2", "--rulings", pinned]);
  assert.equal(res.code, 2, res.stdout + res.stderr);
  assert.match(res.stderr, /"ts" pins a value/);
  // The same file as event rulings loads (the run then fails at the missing scenario below, not at the pin).
  res = await run(process.execPath, [matrix, "--servers", "ts-v2,ts-v2", "--scenarios", "no-such", "--event-rulings", pinned]);
  assert.equal(res.code, 2, res.stdout + res.stderr);
  assert.match(res.stderr, /unknown scenario/);
});

test("the ruling line shows the pinned values", (t) => {
  const rulings = rulingsOf(t, [
    { path: KEYS_PATH, transports: ["stdio"], ts: "mcpReq", rust: "requestId", reason: "keys", deviation: "RD-07" },
    { path: META_PATH, ts: "<absent>", rustKeys: "io\\.modelcontextprotocol/.+", reason: "meta", deviation: "RD-13" },
  ]);
  const [keys, meta] = rulingLines(rulings, "EVENT ");
  assert.match(keys, /^EVENT UNUSED RULING RD-07 matched 0 on stdio ts="mcpReq" rust="requestId": /);
  assert.match(meta, /^EVENT UNUSED RULING RD-13 matched 0 ts="<absent>" rustKeys="io\\\\.modelcontextprotocol\/\.\+": /);
});

test("diff.mjs holds the pinned values too", async (t) => {
  const dir = tempDir(t);
  const a = writeJson(join(dir, "a.normalized.json"), doc("http,mcpReq"));
  const pinned = (rust) => writeJson(join(dir, "rulings.json"), [{ path: KEYS_PATH, ts: "http,mcpReq", rust, reason: "test", deviation: "RD-07" }]);
  const b = writeJson(join(dir, "b.normalized.json"), doc("requestId,requestInfo"));
  let res = await diff(a, b, ["--rulings", pinned("requestId,requestInfo")]);
  assert.equal(res.code, 0, res.stdout + res.stderr);
  assert.match(res.stdout, /^RULED RD-07 \$\.events\[0\]\.body\.parameters\.extra\.<keys>: /m);
  res = await diff(a, b, ["--rulings", pinned("requestId")]);
  assert.equal(res.code, 1, res.stdout + res.stderr);
  assert.match(res.stdout, /^DIFF {2}\$\.events\[0\]\.body\.parameters\.extra\.<keys>: "http,mcpReq" != "requestId,requestInfo"$/m);
  assert.match(res.stdout, /^UNUSED RULING RD-07 matched 0 ts="http,mcpReq" rust="requestId": /m);
});

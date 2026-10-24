// The normalizer rules of the event comparison, each on a crafted run (no driver, no server).
import assert from "node:assert/strict";
import { before, test } from "node:test";

import { loadModule } from "oracle/bundle";
import { ensureTs } from "oracle/fetch-ts";

import { RULES, normalizeRun } from "../src/normalize.mjs";
import { BASIC, readJson } from "./support.mjs";

const PROJECT = readJson(BASIC).projectId;

before(async () => {
  await ensureTs();
});

/** A recorded publish whose raw text is `rawBody` (the mock keeps both). */
const publish = (rawBody) => ({
  method: "POST",
  path: "/v1/public/events/",
  query: "",
  status: 202,
  headers: { "content-type": "application/json" },
  rawBody,
  body: JSON.parse(rawBody),
});
const publishOf = (body) => publish(JSON.stringify(body));

const event = (extra = {}) => ({
  event_type: "mcp:tools/call",
  resource_name: "echo",
  timestamp: "2026-01-01T00:00:00.100Z",
  duration: 1,
  ...extra,
});

/** The normalized events of a run of `requests`. */
async function events(requests, scenario) {
  return (await normalizeRun({ requests, transcript: [], scenario })).normalized.events;
}

test("the rule list names the event-comparison rules and no longer extra-headers-only", () => {
  for (const rule of ["extra-projection", "number-source", "timestamp-class", "id-placeholders"]) assert.ok(RULES.includes(rule), rule);
  assert.equal(RULES.includes("extra-headers-only"), false);
});

test("extra-projection: the sorted keys, every header by lower-case name with volatile values masked, the URL without its port", async () => {
  // TS v2's handler context (http.req) and Rust's (requestInfo), as each server publishes them.
  const headers = {
    Accept: "application/json, text/event-stream",
    "content-length": "396",
    host: "127.0.0.1:45013",
    "mcp-session-id": "0b5d2c3e-0000-4000-8000-000000000000",
    "user-agent": "harness-driver/1.0",
    "x-test-header": "harness",
  };
  const ts = { mcpReq: { id: 1, method: "tools/call", signal: {} }, http: { req: { method: "POST", url: "http://127.0.0.1:45013/mcp", headers } } };
  const rust = { sessionId: "s", requestId: 1, requestInfo: { headers, url: "http://[::1]:8080/mcp?x=1" } };
  const [a, b, stdio] = await events([
    publishOf(event({ parameters: { extra: ts } })),
    publishOf(event({ timestamp: "2026-01-01T00:00:00.200Z", parameters: { extra: rust } })),
    publishOf(event({ timestamp: "2026-01-01T00:00:00.300Z", parameters: { extra: { requestId: 2, signal: {} } } })),
  ]);
  const masked = {
    accept: "application/json, text/event-stream",
    "content-length": "<content-length>",
    host: "<host>",
    "mcp-session-id": "<mcp-session-id>",
    "user-agent": "harness-driver/1.0",
    "x-test-header": "harness",
  };
  assert.deepEqual(a.body.parameters.extra, { "<keys>": "http,mcpReq", "<headers>": masked, "<url>": "http://127.0.0.1:<port>/mcp" });
  assert.deepEqual(b.body.parameters.extra, { "<keys>": "requestId,requestInfo,sessionId", "<headers>": masked, "<url>": "http://[::1]:<port>/mcp?x=1" });
  // stdio: no headers and no URL, so only the keys.
  assert.deepEqual(stdio.body.parameters.extra, { "<keys>": "requestId,signal" });
});

test("number-source: an integer JavaScript would write otherwise becomes a token; 1e21 and 0.5 compare by value", async () => {
  const raw =
    '{"event_type":"mcp:tools/call","resource_name":"echo","timestamp":"2026-01-01T00:00:00.100Z",' +
    '"parameters":{"one":1.0,"negZero":-0,"hundred":1e2,"big":9007199254740993,"plain":7,"neg":-3,' +
    '"huge":1e21,"half":0.5,"tiny":1e-6}}';
  const [e] = await events([publish(raw)]);
  assert.deepEqual(e.body.parameters, {
    one: "<non-js-number:1.0>",
    negZero: "<non-js-number:-0>",
    hundred: "<non-js-number:1e2>",
    big: "<non-js-number:9007199254740993>",
    plain: 7,
    neg: -3,
    huge: 1e21,
    half: 0.5,
    tiny: 0.000001,
  });
});

test("number-source: a request without raw text (an older raw.json) is read from its parsed body", async () => {
  const { rawBody, ...parsed } = publish('{"event_type":"x","timestamp":"2026-01-01T00:00:00.100Z","parameters":{"one":1.0}}');
  const [e] = await events([parsed]);
  assert.deepEqual(e.body.parameters, { one: 1 });
});

test("timestamp-class: milliseconds or whole seconds with Z are one class (RD-04); anything else stays visible", async () => {
  const stamps = [
    "2026-01-01T00:00:00.100Z",
    "2026-01-01T00:00:01Z",
    "2026-01-01T00:00:02.1Z",
    "2026-01-01T00:00:03.000+00:00",
    "2026-01-01T00:00:04.000000Z",
  ];
  const out = await events(stamps.map((timestamp, i) => publishOf(event({ timestamp, resource_name: `r${i}` }))));
  assert.deepEqual(
    out.map((e) => e.body.timestamp),
    [
      "<iso8601-Z>",
      "<iso8601-Z>",
      "<bad-timestamp:2026-01-01T00:00:02.1Z>",
      "<bad-timestamp:2026-01-01T00:00:03.000+00:00>",
      "<bad-timestamp:2026-01-01T00:00:04.000000Z>",
    ],
  );
});

test("id-placeholders: a derived ses_ (KSUID time before 2024-03-01) and a minted one get different placeholders", async () => {
  const { deriveSessionId, newSessionId } = await loadModule("handles");
  const derived = deriveSessionId("customer-abc", PROJECT);
  const minted = newSessionId();
  const out = await events([
    publishOf(event({ session_id: derived })),
    publishOf(event({ timestamp: "2026-01-01T00:00:00.200Z", session_id: minted })),
    publishOf(event({ timestamp: "2026-01-01T00:00:00.300Z", session_id: derived })),
  ]);
  assert.deepEqual(
    out.map((e) => e.body.session_id),
    ["<ses-derived#1>", "<ses#1>", "<ses-derived#1>"],
  );
  // literal-derived-ids still keeps the canned hook value's ID.
  const scenario = { projectId: PROJECT, options: { resolveSessionId: { returns: "customer-abc" } } };
  const kept = await events([publishOf(event({ session_id: derived }))], scenario);
  assert.equal(kept[0].body.session_id, derived);
});

test("event order: a whole-second timestamp sorts before later milliseconds of the same second", async () => {
  const late = publishOf(event({ timestamp: "2026-01-01T00:00:29.005Z", resource_name: "late" }));
  const early = publishOf(event({ timestamp: "2026-01-01T00:00:29Z", resource_name: "early" }));
  for (const requests of [[late, early], [early, late]]) {
    assert.deepEqual((await events(requests)).map((e) => e.body.resource_name), ["early", "late"]);
  }
});

test("event order: in the same millisecond the normalized body decides, not the raw duration", async () => {
  // Same timestamp, both timed, same resource: the tag decides. The raw durations would order them the other way.
  const slow = publishOf(event({ duration: 5, tags: { k: "a" } }));
  const fast = publishOf(event({ duration: 3, tags: { k: "b" } }));
  for (const requests of [[slow, fast], [fast, slow]]) {
    assert.deepEqual((await events(requests)).map((e) => e.body.tags.k), ["a", "b"]);
  }
});

test('event order: "eventOrder": "multiset" sorts on the normalized body alone', async () => {
  const one = (resource, at) => publishOf(event({ resource_name: resource, timestamp: `2026-01-01T00:00:00.${at}Z` }));
  // Two runs whose events started in opposite orders.
  const first = [one("b", "100"), one("a", "200")];
  const second = [one("a", "100"), one("b", "200")];
  const multiset = { eventOrder: "multiset" };
  assert.deepEqual(await events(first, multiset), await events(second, multiset));
  assert.deepEqual((await events(first, multiset)).map((e) => e.body.resource_name), ["a", "b"]);
  // By time, they differ.
  assert.notDeepEqual(await events(first), await events(second));
});

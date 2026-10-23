// A/A runs of the TS v1 server, and the diff / normalizer rules that a
// TS-vs-Rust comparison relies on.
import assert from "node:assert/strict";
import { join } from "node:path";
import { before, test } from "node:test";

import { loadModule } from "oracle/bundle";
import { ensureTs } from "oracle/fetch-ts";

import { SERVER_INFO_META_KEY, normalizeRun } from "../src/normalize.mjs";
import { BASIC, HARNESS, diff, driver, expectedFor, outFile, readJson, tempDir, writeJson } from "./support.mjs";

const basic = readJson(BASIC);

before(async () => {
  await ensureTs();
});

test("(a) two stdio runs of basic.json diff to zero", { timeout: 120_000 }, async () => {
  for (const label of ["aa1", "aa2"]) {
    const res = await driver(BASIC, label, "stdio");
    assert.equal(res.code, 0, `${label}: ${res.stderr}`);
  }
  const res = await diff(
    outFile("basic", "aa1", "stdio", "normalized"),
    outFile("basic", "aa2", "stdio", "normalized"),
    ["--rulings", join(HARNESS, "rulings-ts.json")],
  );
  assert.equal(res.code, 0, res.stdout + res.stderr);
  assert.match(res.stdout, /"total":0/);
});

test("(b) one stateful HTTP run publishes 11", { timeout: 120_000 }, async () => {
  const res = await driver(BASIC, "aa", "http");
  assert.equal(res.code, 0, res.stderr);
  assert.equal(readJson(outFile("basic", "aa", "http", "report")).publishes, 11);
});

test("(c) one stateless HTTP run matches the http-stateless count in basic.json", { timeout: 120_000 }, async () => {
  const res = await driver(BASIC, "aa", "http-stateless");
  assert.equal(res.code, 0, res.stderr);
  const report = readJson(outFile("basic", "aa", "http-stateless", "report"));
  assert.equal(report.expected, expectedFor(basic, "http-stateless"));
  assert.equal(report.publishes, report.expected);
});

const mockPublish = (body) => ({ method: "POST", path: "/v1/public/events/", query: "", status: 200, headers: {}, body });

/** A normalized file built here from two synthetic publishes and two transcript entries (no driver run). */
async function ownNormalized(dir, name = "own") {
  const transcript = [
    { session: 0, op: "initialize", result: { protocolVersion: "2025-11-25" } },
    { session: 0, op: "listTools", result: { tools: [] } },
  ];
  const requests = [
    mockPublish({ event_type: "mcp:tools/list", resource_name: "tools/list", timestamp: "2026-01-01T00:00:00.100Z" }),
    mockPublish({ event_type: "mcp:tools/call", resource_name: "echo", timestamp: "2026-01-01T00:00:00.200Z", duration: 3 }),
  ];
  const { normalized } = await normalizeRun({ requests, transcript, scenario: basic });
  return writeJson(join(dir, `${name}.normalized.json`), normalized);
}

/** An own normalized file with one event's resource_name changed; returns {original, changed, i}. */
async function changedResourceName(dir) {
  const original = await ownNormalized(dir);
  const doc = readJson(original);
  const i = doc.events.findIndex((e) => typeof e.body.resource_name === "string");
  assert.ok(i >= 0, "no event with a resource_name");
  doc.events[i].body.resource_name = "changed";
  return { original, changed: writeJson(join(dir, "changed.normalized.json"), doc), i };
}

test("(d) one changed resource_name makes diff.mjs exit 1, and a ruling for it makes it exit 0", async (t) => {
  const dir = tempDir(t);
  const { original, changed, i } = await changedResourceName(dir);

  let res = await diff(original, changed);
  assert.equal(res.code, 1, res.stdout);
  assert.match(res.stdout, new RegExp(`\\$\\.events\\[${i}\\]\\.body\\.resource_name`));

  const rulings = writeJson(join(dir, "rulings.json"), [
    { path: "^\\$\\.events\\[\\d+\\]\\.body\\.resource_name$", reason: "test ruling", deviation: "RD-99" },
  ]);
  res = await diff(original, changed, ["--rulings", rulings]);
  assert.equal(res.code, 0, res.stdout + res.stderr);
  assert.match(res.stdout, /RULED/);
});

test("a pending ruling is a ruling; a pending value other than a boolean makes diff.mjs exit 2", async (t) => {
  const dir = tempDir(t);
  const { original, changed } = await changedResourceName(dir);
  const ruling = { path: "\\$\\.events\\[\\d+\\]\\.body\\.resource_name", reason: "awaits the owner", deviation: "RD-01" };

  const pending = writeJson(join(dir, "pending.json"), [{ ...ruling, pending: true }]);
  let res = await diff(original, changed, ["--rulings", pending]);
  assert.equal(res.code, 0, res.stdout + res.stderr);
  assert.match(res.stdout, /^RULING \(pending\) RD-01 matched 1:/m);
  // A ruling that is not pending keeps the plain marker.
  const plain = writeJson(join(dir, "plain.json"), [ruling]);
  res = await diff(original, changed, ["--rulings", plain]);
  assert.match(res.stdout, /^RULING RD-01 matched 1:/m);
  assert.doesNotMatch(res.stdout, /pending/);

  const notBoolean = writeJson(join(dir, "not-boolean.json"), [{ ...ruling, pending: "yes" }]);
  res = await diff(original, changed, ["--rulings", notBoolean]);
  assert.equal(res.code, 2, res.stdout + res.stderr);
  assert.match(res.stderr, /\[0\]: .*"pending"/);
});

test("(e) a ruling that matches nothing makes diff.mjs exit 1", async (t) => {
  const rulings = writeJson(join(tempDir(t), "rulings.json"), [
    { path: "^\\$\\.nothing\\.here$", reason: "matches nothing", deviation: "RD-98" },
  ]);
  const a = await ownNormalized(tempDir(t));
  const res = await diff(a, a, ["--rulings", rulings]);
  assert.equal(res.code, 1, res.stdout);
  assert.match(res.stdout + res.stderr, /RD-98/);
});

test("ruling regexes match whole paths only, and the output gives each ruling's match count", async (t) => {
  const dir = tempDir(t);
  const { original, changed, i } = await changedResourceName(dir);
  const substrings = writeJson(join(dir, "substrings.json"), [
    { path: "resource_name", reason: "a bare name", deviation: "RD-97" },
    { path: "\\.body\\.resource_name", reason: "a path tail", deviation: "RD-96" },
  ]);
  let res = await diff(original, changed, ["--rulings", substrings]);
  assert.equal(res.code, 1, res.stdout);
  assert.match(res.stdout, new RegExp(`^DIFF  \\$\\.events\\[${i}\\]\\.body\\.resource_name:`, "m"));
  assert.match(res.stdout, /^UNUSED RULING RD-97 matched 0:/m);
  assert.match(res.stdout, /^UNUSED RULING RD-96 matched 0:/m);

  const whole = writeJson(join(dir, "whole.json"), [
    { path: "\\$\\.events\\[\\d+\\]\\.body\\.resource_name", reason: "the whole path, no anchors", deviation: "RD-95" },
  ]);
  res = await diff(original, changed, ["--rulings", whole]);
  assert.equal(res.code, 0, res.stdout);
  assert.match(res.stdout, /^RULING RD-95 matched 1:/m);
});

test("arrays of different length give one entry per element, with <missing> on the short side", async (t) => {
  const dir = tempDir(t);
  const long = await ownNormalized(dir, "long");
  const doc = readJson(long);
  const short = writeJson(join(dir, "short.normalized.json"), { ...doc, transcript: doc.transcript.slice(0, 1) });
  const longer = writeJson(join(dir, "longer.normalized.json"), {
    ...doc,
    transcript: [...doc.transcript, { session: 0, op: "listTools", result: { tools: [] } }, { session: 0, op: "listTools" }],
  });

  let res = await diff(long, short);
  assert.equal(res.code, 1, res.stdout);
  assert.match(res.stdout, /^DIFF  \$\.transcript\[1\]: \{.*\} != "<missing>"$/m);
  assert.doesNotMatch(res.stdout, /\.length/);
  assert.equal(JSON.parse(res.stdout.trim().split("\n").at(-1)).total, 1);

  res = await diff(long, longer);
  assert.equal(res.code, 1, res.stdout);
  assert.match(res.stdout, /^DIFF  \$\.transcript\[2\]: "<missing>" != \{.*\}$/m);
  assert.match(res.stdout, /^DIFF  \$\.transcript\[3\]: "<missing>" != \{.*\}$/m);
  assert.equal(JSON.parse(res.stdout.trim().split("\n").at(-1)).total, 2);

  // A ruling can allow one missing element by its index.
  const rulings = writeJson(join(dir, "rulings.json"), [
    { path: "\\$\\.transcript\\[1\\]", reason: "test ruling", deviation: "RD-94" },
  ]);
  res = await diff(long, short, ["--rulings", rulings]);
  assert.equal(res.code, 0, res.stdout);
  assert.match(res.stdout, /^RULED RD-94 \$\.transcript\[1\]:/m);
});

/** aa1's raw requests and transcript, and a normalizer for variants of them. */
function aa1Run() {
  return {
    raw: readJson(outFile("basic", "aa1", "stdio", "raw")),
    transcript: readJson(outFile("basic", "aa1", "stdio", "transcript")),
  };
}

async function normalizedVariant(dir, name, requests, transcript) {
  const { normalized } = await normalizeRun({ requests, transcript, scenario: basic });
  return writeJson(join(dir, `${name}.normalized.json`), normalized);
}

test("an extra request, or another publish content-type or query, makes the diff fail", async (t) => {
  const dir = tempDir(t);
  const { raw, transcript } = aa1Run();
  const base = await normalizedVariant(dir, "base", raw, transcript);
  assert.deepEqual(readJson(base), readJson(outFile("basic", "aa1", "stdio", "normalized")));
  assert.deepEqual(readJson(base).nonPublishRequests, []);

  const extra = structuredClone(raw);
  extra.push({ ...structuredClone(raw[0]), seq: raw.length, path: "/v1/public/events", status: 404, body: {} });
  let res = await diff(base, await normalizedVariant(dir, "extra", extra, transcript));
  assert.equal(res.code, 1, res.stdout);
  assert.match(res.stdout, /\$\.nonPublishRequests/);

  const contentType = structuredClone(raw);
  contentType[0].headers["content-type"] = "text/plain";
  res = await diff(base, await normalizedVariant(dir, "content-type", contentType, transcript));
  assert.equal(res.code, 1, res.stdout);
  assert.match(res.stdout, /\.content_type: "application\/json" != "text\/plain"/);

  const query = structuredClone(raw);
  query[0].query = "?retry=1";
  res = await diff(base, await normalizedVariant(dir, "query", query, transcript));
  assert.equal(res.code, 1, res.stdout);
  assert.match(res.stdout, /\.query: "" != "\?retry=1"/);
});

test("error frames of another shape, an empty stack and an unknown sdk_language show in the diff", async (t) => {
  const dir = tempDir(t);
  const { raw, transcript } = aa1Run();
  const base = await normalizedVariant(dir, "base", raw, transcript);
  const at = raw.findIndex((r) => Array.isArray(r.body?.error?.frames) && r.body.error.frames.length > 0);
  assert.ok(at >= 0, "no event with error frames");
  const doc = readJson(base);
  const boom = doc.events.find((e) => typeof e.body.error?.frames === "string" && e.body.error.frames !== "<frames:array:>");
  assert.match(boom.body.error.frames, /^<frames:array:.*\bfilename\b.*\bin_app\b.*>$/);
  assert.equal(boom.body.error.stack, "<stack:string>");
  assert.equal(boom.body.error.platform, "<platform:string>");
  assert.ok(doc.events.every((e) => e.body.sdk_language === undefined || e.body.sdk_language === "<sdk_language>"));

  const variants = {
    "frames-without-in_app": (b) => (b.error.frames = b.error.frames.map(({ in_app, ...frame }) => frame)),
    "frames-as-string": (b) => (b.error.frames = "at boom (file.rs:1)"),
    "frames-empty": (b) => (b.error.frames = []),
    "stack-empty": (b) => (b.error.stack = ""),
    "sdk_language-empty": (b) => (b.sdk_language = ""),
  };
  for (const [name, change] of Object.entries(variants)) {
    const requests = structuredClone(raw);
    change(requests[at].body);
    const res = await diff(base, await normalizedVariant(dir, name, requests, transcript));
    assert.equal(res.code, 1, `${name}: ${res.stdout}`);
    const field = name.split("-")[0];
    assert.match(res.stdout, new RegExp(`\\.body\\.(error\\.)?${field}: `), `${name}: ${res.stdout}`);
  }
});

const SES = "ses_2bWv1nVbHvnL4lsnp1tqXjqGWQz";

function callRun(structuredContent) {
  return {
    requests: [],
    transcript: [
      {
        session: 0,
        op: "callTool",
        request: { name: "structured", arguments: { msg: "s" } },
        result: { content: [{ type: "text", text: "s" }], structuredContent },
      },
    ],
  };
}

async function normalizedFile(dir, name, run) {
  const { normalized } = await normalizeRun(run);
  return writeJson(join(dir, `${name}.normalized.json`), normalized);
}

test("(f) a structuredContent whose first key is not mcp_session produces a diff entry", async (t) => {
  const dir = tempDir(t);
  const good = await normalizedFile(dir, "good", callRun({ mcp_session: { session_id: SES }, echoed: "s" }));
  const bad = await normalizedFile(dir, "bad", callRun({ echoed: "s", mcp_session: { session_id: SES } }));

  let res = await diff(good, good);
  assert.equal(res.code, 0, res.stdout);
  res = await diff(good, bad);
  assert.equal(res.code, 1, res.stdout);
  assert.match(res.stdout, /\$\.transcript\[0\]\.result\.structuredContent.*mcp-session-first/);
  // The violation is a diff entry even when both sides share it.
  res = await diff(bad, bad);
  assert.equal(res.code, 1, res.stdout);
  assert.match(res.stdout, /mcp-session-first/);
});

function listRun(properties) {
  return {
    requests: [],
    transcript: [
      {
        session: 0,
        op: "listTools",
        result: { tools: [{ name: "echo", inputSchema: { type: "object", properties } }] },
      },
    ],
  };
}

test("(g) two tool schemas that differ only in property order produce a diff entry", async (t) => {
  const dir = tempDir(t);
  const msg = { type: "string" };
  const sid = { type: "string", description: "session handle" };
  const a = await normalizedFile(dir, "a", listRun({ msg, session_id: sid }));
  const b = await normalizedFile(dir, "b", listRun({ session_id: sid, msg }));
  const res = await diff(a, b);
  assert.equal(res.code, 1, res.stdout);
  assert.match(res.stdout, /\$\.transcript\[0\]\.result\.tools\[0\]\.inputSchema\.<properties-order>/);
});

test("server-info-meta-drop drops only the host's serverInfo key from transcript results, and a _meta it empties", async () => {
  const serverInfo = { name: "harness-ref-server", version: "1.0.0" };
  const call = (msg, result) => ({ session: 0, op: "callTool", request: { name: "echo", arguments: { msg } }, result });
  const transcript = [
    call("only", { _meta: { [SERVER_INFO_META_KEY]: serverInfo }, content: [] }),
    call("and another key", { _meta: { [SERVER_INFO_META_KEY]: serverInfo, "example.com/other": 1 }, content: [] }),
    call("no serverInfo", { _meta: { "example.com/other": 2 }, content: [] }),
    { session: 0, op: "callTool", request: { name: "nope", arguments: {} }, error: { code: -32602, message: "x", _meta: { [SERVER_INFO_META_KEY]: serverInfo } } },
  ];
  // Event bodies are not transcript results: the rule leaves them alone.
  const body = { event_type: "mcp:tools/call", resource_name: "echo", parameters: { request: { _meta: { [SERVER_INFO_META_KEY]: serverInfo } } } };
  const { normalized, report } = await normalizeRun({ requests: [mockPublish(body)], transcript });
  assert.ok(report.rules.includes("server-info-meta-drop"));
  assert.deepEqual(
    normalized.transcript.map((e) => e.result ?? e.error),
    [
      { content: [] },
      { _meta: { "example.com/other": 1 }, content: [] },
      { _meta: { "example.com/other": 2 }, content: [] },
      { code: -32602, message: "x", _meta: { [SERVER_INFO_META_KEY]: serverInfo } },
    ],
  );
  assert.deepEqual(normalized.events[0].body.parameters.request, body.parameters.request);
});

test("a custom event published during a tool call sorts after the call event, in the same millisecond or the next", async () => {
  const publish = (body) => ({ method: "POST", path: "/v1/public/events/", query: "", status: 200, headers: {}, body });
  const call = publish({ event_type: "mcp:tools/call", resource_name: "emit_custom", timestamp: "2026-01-01T00:00:00.100Z", duration: 3 });
  const order = async (requests) => (await normalizeRun({ requests, transcript: [] })).normalized.events.map((e) => e.body.event_type);
  for (const at of ["100", "101"]) {
    const custom = publish({ event_type: "agentcat:custom", resource_name: "checkout", timestamp: `2026-01-01T00:00:00.${at}Z` });
    // Arrival order is noise: both orders must normalize alike.
    assert.deepEqual(await order([call, custom]), ["mcp:tools/call", "agentcat:custom"], `custom at .${at}`);
    assert.deepEqual(await order([custom, call]), ["mcp:tools/call", "agentcat:custom"], `custom at .${at}, arrived first`);
  }
});

test("(h) a resolveSessionId hook run keeps the derived ses_ ID literal", { timeout: 120_000 }, async (t) => {
  const scenario = {
    name: "hook",
    projectId: basic.projectId,
    options: { resolveSessionId: { returns: "customer-session-1" } },
    expectPublishes: 2,
    sessions: [
      {
        client: { name: "hook-client", version: "1.0.0" },
        steps: [
          { op: "listTools" },
          { op: "callTool", name: "echo", args: { msg: "one" } },
          { op: "callTool", name: "echo", args: { msg: "two" } },
        ],
      },
    ],
  };
  const path = writeJson(join(tempDir(t), "hook.json"), scenario);
  const res = await driver(path, "aa", "stdio");
  assert.equal(res.code, 0, res.stderr);

  const { deriveSessionId } = await loadModule("handles");
  const derived = deriveSessionId("customer-session-1", basic.projectId);
  const normalized = readJson(outFile("hook", "aa", "stdio", "normalized"));
  assert.ok(normalized.events.length > 0);
  for (const event of normalized.events) assert.equal(event.body.session_id, derived);
  const report = readJson(outFile("hook", "aa", "stdio", "report"));
  assert.ok(report.report.rules.includes("literal-derived-ids"));
  assert.deepEqual(report.report.literalIds, [derived]);
});

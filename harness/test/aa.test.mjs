// A/A runs of the TS v1 server, and the diff / normalizer rules that a
// TS-vs-Rust comparison relies on.
import assert from "node:assert/strict";
import { join } from "node:path";
import { before, test } from "node:test";

import { loadModule } from "oracle/bundle";
import { ensureTs } from "oracle/fetch-ts";

import { normalizeRun } from "../src/normalize.mjs";
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
    ["--rulings", join(HARNESS, "rulings.json")],
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

/** aa1's normalized file with one event's resource_name changed; returns {original, changed, i}. */
function changedResourceName(dir) {
  const original = outFile("basic", "aa1", "stdio", "normalized");
  const doc = readJson(original);
  const i = doc.events.findIndex((e) => typeof e.body.resource_name === "string");
  assert.ok(i >= 0, "no event with a resource_name");
  doc.events[i].body.resource_name = "changed";
  return { original, changed: writeJson(join(dir, "changed.normalized.json"), doc), i };
}

test("(d) one changed resource_name makes diff.mjs exit 1, and a ruling for it makes it exit 0", async (t) => {
  const dir = tempDir(t);
  const { original, changed, i } = changedResourceName(dir);

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

test("(e) a ruling that matches nothing makes diff.mjs exit 1", async (t) => {
  const rulings = writeJson(join(tempDir(t), "rulings.json"), [
    { path: "^\\$\\.nothing\\.here$", reason: "matches nothing", deviation: "RD-98" },
  ]);
  const a = outFile("basic", "aa1", "stdio", "normalized");
  const res = await diff(a, a, ["--rulings", rulings]);
  assert.equal(res.code, 1, res.stdout);
  assert.match(res.stdout + res.stderr, /RD-98/);
});

test("ruling regexes match whole paths only, and the output gives each ruling's match count", async (t) => {
  const dir = tempDir(t);
  const { original, changed, i } = changedResourceName(dir);
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

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

test("(d) one changed resource_name makes diff.mjs exit 1, and a ruling for it makes it exit 0", async (t) => {
  const dir = tempDir(t);
  const original = outFile("basic", "aa1", "stdio", "normalized");
  const doc = readJson(original);
  const i = doc.events.findIndex((e) => typeof e.body.resource_name === "string");
  assert.ok(i >= 0, "no event with a resource_name");
  doc.events[i].body.resource_name = "changed";
  const changed = writeJson(join(dir, "changed.normalized.json"), doc);

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

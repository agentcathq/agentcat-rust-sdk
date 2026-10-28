// The MCP-wire delta: ops between an untracked and a tracked transcript, and the TS-vs-Rust comparison.
import assert from "node:assert/strict";
import { join } from "node:path";
import { test } from "node:test";

import { compareDeltas, transcriptDelta } from "../src/delta.mjs";
import { HARNESS, run, tempDir, writeJson } from "./support.mjs";

const DELTA = join(HARNESS, "src", "delta.mjs");
const entry = (result, extra = {}) => ({ session: 0, op: "callTool", result, ...extra });

test("a prepended content block is one add at index 0", () => {
  const a = { type: "text", text: "hi" };
  const block = { type: "text", text: "mint-back" };
  const ops = transcriptDelta([entry({ content: [a] })], [entry({ content: [block, a] })]);
  assert.deepEqual(ops, [{ op: "add", path: "$.delta.transcript[0].result.content[0]", value: block }]);
});

test("an added key is an add with its value", () => {
  const ops = transcriptDelta([entry({ content: [] })], [entry({ content: [], structuredContent: { mcp_session: "ses_x" } })]);
  assert.deepEqual(ops, [
    { op: "add", path: "$.delta.transcript[0].result.structuredContent", value: { mcp_session: "ses_x" } },
  ]);
});

test("a removed additionalProperties is a remove without a value", () => {
  const u = { session: 0, op: "listTools", result: { tools: [{ name: "t", inputSchema: { type: "object", additionalProperties: false } }] } };
  const t = { session: 0, op: "listTools", result: { tools: [{ name: "t", inputSchema: { type: "object" } }] } };
  const ops = transcriptDelta([u], [t]);
  assert.deepEqual(ops, [{ op: "remove", path: "$.delta.transcript[0].result.tools[0].inputSchema.additionalProperties" }]);
  assert.equal(Object.hasOwn(ops[0], "value"), false);
});

test("an untracked error that becomes a tracked result is remove error plus add result", () => {
  const ops = transcriptDelta([{ session: 0, op: "callTool", error: { code: -32602 } }], [entry({ content: [] })]);
  assert.deepEqual(ops, [
    { op: "remove", path: "$.delta.transcript[0].error" },
    { op: "add", path: "$.delta.transcript[0].result", value: { content: [] } },
  ]);
});

test("a changed scalar carries the tracked value", () => {
  const ops = transcriptDelta([entry({ isError: false })], [entry({ isError: true })]);
  assert.deepEqual(ops, [{ op: "change", path: "$.delta.transcript[0].result.isError", value: true }]);
});

test("arrays of equal length compare by index", () => {
  const ops = transcriptDelta([entry({ content: [{ text: "a" }, { text: "b" }] })], [entry({ content: [{ text: "a" }, { text: "c" }] })]);
  assert.deepEqual(ops, [{ op: "change", path: "$.delta.transcript[0].result.content[1].text", value: "c" }]);
});

/** A normalized document with one tools/call transcript entry. */
const doc = (result, violations = []) => ({ transcript: [entry(result)], events: [], nonPublishRequests: [], violations });

function files(t, ...docs) {
  const dir = tempDir(t);
  return docs.map((d, i) => writeJson(join(dir, `${i}.json`), d));
}
const cli = (paths, extra = []) => run(process.execPath, [DELTA, ...paths, ...extra]);

test("equal deltas over different untracked values compare equal", async (t) => {
  const paths = files(
    t,
    doc({ content: [], n: 1 }),
    doc({ content: [], n: 1, structuredContent: { mcp_session: "ses_1" } }),
    doc({ content: [], n: 2 }),
    doc({ content: [], n: 2, structuredContent: { mcp_session: "ses_1" } }),
  );
  const res = await cli(paths);
  assert.equal(res.code, 0, res.stdout + res.stderr);
  assert.match(res.stdout, /"total":0/);
});

const different = (t) =>
  files(
    t,
    doc({ content: [] }),
    doc({ content: [], note: "ts" }),
    doc({ content: [] }),
    doc({ content: [], note: "rust" }),
  );

test("a different added value is a DELTA line and exit 1", async (t) => {
  const res = await cli(different(t));
  assert.equal(res.code, 1);
  assert.match(res.stdout, /^DELTA \$\.delta\.transcript\[0\]\.result\.note: ts add "ts" != rust add "rust"$/m);
});

test("a ruling for it gives exit 0 and a RULED line", async (t) => {
  const paths = different(t);
  const rulings = writeJson(join(tempDir(t), "r.json"), [
    { path: "\\$\\.delta\\.transcript\\[0\\]\\.result\\.note", reason: "test", deviation: "RD-99" },
  ]);
  const res = await cli(paths, ["--rulings", rulings]);
  assert.equal(res.code, 0, res.stdout);
  assert.match(res.stdout, /^RULED RD-99 \$\.delta\.transcript\[0\]\.result\.note: /m);
  assert.match(res.stdout, /^RULING RD-99 matched 1: /m);
});

test("delta.mjs knows no scenario: a scenario-scoped ruling matches by path alone; a non-string scenario exits 2", async (t) => {
  const paths = different(t);
  const ruling = { path: "\\$\\.delta\\.transcript\\[0\\]\\.result\\.note", reason: "test", deviation: "RD-99" };
  const scoped = writeJson(join(tempDir(t), "r.json"), [{ ...ruling, scenario: "tracing-off" }]);
  let res = await cli(paths, ["--rulings", scoped]);
  assert.equal(res.code, 0, res.stdout);
  assert.match(res.stdout, /^RULED RD-99 \$\.delta\.transcript\[0\]\.result\.note: /m);
  assert.match(res.stdout, /^RULING RD-99 matched 1 in tracing-off: /m);

  const bad = writeJson(join(tempDir(t), "bad.json"), [{ ...ruling, scenario: ["tracing-off"] }]);
  res = await cli(paths, ["--rulings", bad]);
  assert.equal(res.code, 2, res.stdout + res.stderr);
  assert.match(res.stderr, /\[0\]: "scenario" must be a scenario name/);
});

test("delta.mjs knows no transport or lane: a ruling scoped to them matches by path alone; an unknown lane exits 2", async (t) => {
  const paths = different(t);
  const ruling = { path: "\\$\\.delta\\.transcript\\[0\\]\\.result\\.note", reason: "test", deviation: "RD-99" };
  const scoped = writeJson(join(tempDir(t), "r.json"), [{ ...ruling, transports: ["http"], lanes: ["2026-07-28"] }]);
  let res = await cli(paths, ["--rulings", scoped]);
  assert.equal(res.code, 0, res.stdout);
  assert.match(res.stdout, /^RULED RD-99 \$\.delta\.transcript\[0\]\.result\.note: /m);
  assert.match(res.stdout, /^RULING RD-99 matched 1 on http at 2026-07-28: /m);

  const bad = writeJson(join(tempDir(t), "bad.json"), [{ ...ruling, lanes: ["2099-01-01"] }]);
  res = await cli(paths, ["--rulings", bad]);
  assert.equal(res.code, 2, res.stdout + res.stderr);
  assert.match(res.stderr, /\[0\]: "lanes" names an unknown lane "2099-01-01"/);
});

test("an unused ruling exits 1", async (t) => {
  const paths = files(t, doc({ content: [] }), doc({ content: [] }), doc({ content: [] }), doc({ content: [] }));
  const rulings = writeJson(join(tempDir(t), "r.json"), [{ path: "nothing", reason: "test", deviation: "RD-98", pending: true }]);
  const res = await cli(paths, ["--rulings", rulings]);
  assert.equal(res.code, 1);
  assert.match(res.stdout, /^UNUSED RULING \(pending\) RD-98 matched 0: /m);
});

test("a tracked-document violation is an op the other side lacks", async (t) => {
  const v = { rule: "mcp-session-first", path: "$.transcript[0].result" };
  const paths = files(t, doc({ content: [] }), doc({ content: [] }, [v]), doc({ content: [] }), doc({ content: [] }));
  const res = await cli(paths);
  assert.equal(res.code, 1);
  assert.match(res.stdout, /^DELTA \$\.delta\.violations\[mcp-session-first\]@\$\.transcript\[0\]\.result: ts add /m);
});

test("bad arguments exit 2", async (t) => {
  assert.equal((await cli(["only-one"])).code, 2);
  const paths = files(t, doc({}), doc({}), doc({}), doc({}));
  assert.equal((await cli(paths, ["--rulings"])).code, 2);
});

test("a delta entry keeps the whole text of each side beside the clipped summary", () => {
  const long = "x".repeat(300);
  const ts = [{ op: "add", path: "$.delta.p", value: { text: long } }];
  const rust = [{ op: "add", path: "$.delta.p", value: { text: `${long}!` } }];
  const [e] = compareDeltas(ts, rust);
  assert.ok(e.ts.length < 150 && e.ts.endsWith("..."), e.ts);
  assert.equal(e.tsText, `add ${JSON.stringify({ text: long })}`);
  assert.equal(e.rustText, `add ${JSON.stringify({ text: `${long}!` })}`);
  assert.equal(compareDeltas([], rust)[0].tsText, "<none>");
});

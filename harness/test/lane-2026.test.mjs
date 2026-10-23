// The 2026-07-28 lane (design decision C18): the TS v2 reference server
// (@modelcontextprotocol/server 2.x) driven by the v2 client with
// versionNegotiation "auto". MCP 2026-07-28 has no initialize handshake, so a
// v1 McpServer cannot serve it.
import assert from "node:assert/strict";
import { join } from "node:path";
import { before, test } from "node:test";

import { ensureTs } from "oracle/fetch-ts";
import { tsDir } from "oracle/paths";

import {
  BASIC,
  CONTRACT,
  HARNESS,
  RUST_SERVER,
  TS_SERVER,
  TS_V2_SERVER,
  diff,
  driver,
  expectedFor,
  outFile,
  readJson,
  run,
  tempDir,
  writeJson,
} from "./support.mjs";

const LANE = "2026-07-28";
const basic = readJson(BASIC);
const v2 = { server: TS_V2_SERVER };

before(async () => {
  await ensureTs();
});

/** The protocol version each session of a run negotiated (the transcript's session-start entries). */
const negotiated = (transcript) => transcript.filter((e) => e.op === "initialize").map((e) => e.result.protocolVersion);

/** Name and input schema of each listed tool, in list order. */
const namesAndInputs = (tools) => tools.map(({ name, inputSchema }) => ({ name, inputSchema }));

test("2026-07-28: two HTTP runs of basic.json against ts-v2 publish the expected count and diff to zero", { timeout: 180_000 }, async () => {
  const expected = expectedFor(basic, "http", LANE);
  for (const label of ["v2lane1", "v2lane2"]) {
    const res = await driver(BASIC, label, "http", ["--lane", LANE], v2);
    assert.equal(res.code, 0, `${label}: ${res.stderr}`);
    const report = readJson(outFile("basic", label, "http", "report"));
    assert.equal(report.lane, LANE);
    assert.equal(report.publishes, expected);
    // "auto" falls back to initialize when the probe fails; the lane must really be 2026-07-28.
    assert.deepEqual(negotiated(readJson(outFile("basic", label, "http", "transcript"))), basic.sessions.map(() => LANE));
  }
  const res = await diff(
    outFile("basic", "v2lane1", "http", "normalized"),
    outFile("basic", "v2lane2", "http", "normalized"),
    ["--rulings", join(HARNESS, "rulings.json")],
  );
  assert.equal(res.code, 0, res.stdout + res.stderr);
  assert.match(res.stdout, /"total":0/);
});

test("2026-07-28: untracked, ts-v2 lists the tool contract's tools whole (R20) and publishes nothing", { timeout: 120_000 }, async (t) => {
  const scenario = writeJson(join(tempDir(t), "basic.json"), { ...basic, expectPublishes: 0 });
  const res = await driver(scenario, "v2contract", "http", ["--lane", LANE], { ...v2, env: { HARNESS_NO_TRACK: "1" } });
  assert.equal(res.code, 0, res.stderr);
  assert.doesNotMatch(res.stderr, /egress-guard/);
  assert.deepEqual(readJson(outFile("basic", "v2contract", "http", "raw")), []);
  const transcript = readJson(outFile("basic", "v2contract", "http", "transcript"));
  assert.deepEqual(negotiated(transcript), basic.sessions.map(() => LANE));
  const tools = transcript.find((e) => e.op === "listTools").result.tools;
  // The contract is ts-v2's own tools/list: names, descriptions, schemas, and no other field.
  assert.deepEqual(tools, readJson(CONTRACT));
  const listed = namesAndInputs(tools);
  const contract = namesAndInputs(readJson(CONTRACT));
  assert.deepEqual(listed, contract);
  // deepEqual ignores key order; the diff compares the order of each tool's
  // inputSchema.properties (normalizer rule property-order), so check it too.
  const propertyOrder = (tools) => tools.map((tool) => Object.keys(tool.inputSchema.properties));
  assert.deepEqual(propertyOrder(listed), propertyOrder(contract));
  const emit = transcript.find((e) => e.request?.name === "emit_custom");
  assert.deepEqual(emit.result.content, [{ type: "text", text: "published" }]);
});

test("over stdio, a <transport>@<lane> count wins over the <transport> count, and a missing count exits 2", { timeout: 120_000 }, async (t) => {
  // 2026-07-28 over stdio: the v2 client probes on a short-lived sibling process, then serveStdio serves the era.
  const keyed = writeJson(join(tempDir(t), "basic.json"), { ...basic, expectPublishes: { stdio: 99, [`stdio@${LANE}`]: 11 } });
  let res = await driver(keyed, "v2lanestdio", "stdio", ["--lane", LANE], v2);
  assert.equal(res.code, 0, res.stderr);
  const report = readJson(outFile("basic", "v2lanestdio", "stdio", "report"));
  assert.equal(report.expected, 11);
  assert.equal(report.publishes, 11);
  assert.deepEqual(negotiated(readJson(outFile("basic", "v2lanestdio", "stdio", "transcript"))), basic.sessions.map(() => LANE));

  const missing = writeJson(join(tempDir(t), "missing.json"), { ...basic, expectPublishes: { [`stdio@${LANE}`]: 11 } });
  res = await driver(missing, "v2missing", "stdio", [], v2);
  assert.equal(res.code, 2, res.stderr);
  assert.match(res.stderr, /expectPublishes has no count for stdio@2025-11-25 or stdio/);
});

test("ts-v2 over HTTP on the 2025-11-25 lane publishes the basic.json count", { timeout: 120_000 }, async () => {
  const res = await driver(BASIC, "v2legacy", "http", [], v2);
  assert.equal(res.code, 0, res.stderr);
  const report = readJson(outFile("basic", "v2legacy", "http", "report"));
  assert.equal(report.lane, "2025-11-25");
  assert.equal(report.publishes, expectedFor(basic, "http"));
  assert.deepEqual(negotiated(readJson(outFile("basic", "v2legacy", "http", "transcript"))), basic.sessions.map(() => "2025-11-25"));
});

// The TS SDK marks each tool function it wraps in a module-wide WeakSet, so a
// function shared by two servers is wrapped only on the first, and the thrown
// error of a later server loses its stack. Each server needs its own handlers.
test("over http-stateless, ts-v1 and ts-v2 publish the boom error with type, stack and frames", { timeout: 120_000 }, async () => {
  for (const [label, server] of [["v1fresh", TS_SERVER], ["v2fresh", TS_V2_SERVER]]) {
    const res = await driver(BASIC, label, "http-stateless", [], { server });
    assert.equal(res.code, 0, `${label}: ${res.stderr}`);
    const boom = readJson(outFile("basic", label, "http-stateless", "normalized")).events.find((e) => e.body.resource_name === "boom");
    assert.equal(boom.body.error.type, "Error", label);
    assert.equal(boom.body.error.stack, "<stack:string>", label);
    assert.match(boom.body.error.frames, /^<frames:array:.+>$/, label);
  }
});

// rmcp serves server/discover and the per-request 2026-07-28 envelope itself; the driver fails a
// session that negotiates another version. The Rust SDK publishes nothing yet (plan 4).
test("2026-07-28: the tracked Rust server negotiates the lane on stdio and HTTP", { timeout: 180_000 }, async () => {
  for (const transport of ["stdio", "http"]) {
    const res = await driver(BASIC, "rustlane", transport, ["--lane", LANE, "--expect-publishes", "0"], { server: RUST_SERVER });
    assert.equal(res.code, 0, `${transport}: ${res.stderr}`);
    const report = readJson(outFile("basic", "rustlane", transport, "report"));
    assert.equal(report.lane, LANE);
    assert.deepEqual(negotiated(readJson(outFile("basic", "rustlane", transport, "transcript"))), basic.sessions.map(() => LANE));
  }
});

test("ts-v2 refuses a non-local AGENTCAT_API_URL with exit 2", async (t) => {
  const [cmd, ...args] = TS_V2_SERVER;
  for (const mode of [["stdio"], ["http", "0"], ["http-stateless", "0"]]) {
    for (const url of ["https://api.agentcat.com", "http://localhost.example.com"]) {
      const res = await run(cmd, [...args, ...mode], {
        env: {
          ...process.env,
          AGENTCAT_API_URL: url,
          DISABLE_DIAGNOSTICS: "true",
          HOME: tempDir(t),
          HARNESS_PROJECT_ID: basic.projectId,
        },
      });
      assert.equal(res.code, 2, `${mode[0]} ${url}: ${res.stderr}`);
      assert.match(res.stderr, /refusing to start/);
      assert.equal(res.stdout, "");
    }
  }
});

test("the harness and the TS clone resolve the same @modelcontextprotocol server, node and client 2.x", () => {
  const version = (dir, name) => readJson(join(dir, "node_modules", "@modelcontextprotocol", name, "package.json")).version;
  const pins = { server: "2.0.0", node: "2.0.0", client: "2.3.1" };
  for (const [name, pin] of Object.entries(pins)) {
    assert.equal(version(HARNESS, name), pin, name);
    assert.equal(version(tsDir(), name), pin, `the TS clone's ${name}`);
  }
});

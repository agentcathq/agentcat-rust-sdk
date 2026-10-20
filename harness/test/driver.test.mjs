// The driver fails a run whose publish count differs from expectPublishes,
// and rejects bad arguments; the TS server rejects a bad options profile.
import assert from "node:assert/strict";
import { join } from "node:path";
import { before, test } from "node:test";

import { ensureTs } from "oracle/fetch-ts";
import { tsDir } from "oracle/paths";

import {
  BASIC,
  DRIVER,
  DUMP_CONTRACT,
  HARNESS,
  TS_SERVER,
  TS_SERVER_PLUS,
  driver,
  outFile,
  readJson,
  run,
  tempDir,
  writeJson,
} from "./support.mjs";

before(async () => {
  await ensureTs();
});

function basicWith(t, patch) {
  return writeJson(join(tempDir(t), "basic.json"), { ...readJson(BASIC), ...patch });
}

/** One session, one echo call: one publish from a tracked server. */
function oneCall(t, expectPublishes) {
  return writeJson(join(tempDir(t), "one.json"), {
    name: "one",
    projectId: readJson(BASIC).projectId,
    options: {},
    expectPublishes,
    sessions: [{ steps: [{ op: "callTool", name: "echo", args: { msg: "hi" } }] }],
  });
}

test("basic.json with expectPublishes 12 exits 1", { timeout: 120_000 }, async (t) => {
  const res = await driver(basicWith(t, { expectPublishes: 12 }), "driver12", "stdio");
  assert.equal(res.code, 1, res.stderr);
  const report = readJson(outFile("basic", "driver12", "stdio", "report"));
  assert.equal(report.expected, 12);
  assert.equal(report.publishes, 11);
});

test("basic.json with expectPublishes 11 exits 0 and reports 11 publishes", { timeout: 120_000 }, async (t) => {
  const res = await driver(basicWith(t, { expectPublishes: 11 }), "driver11", "stdio");
  assert.equal(res.code, 0, res.stderr);
  const report = readJson(outFile("basic", "driver11", "stdio", "report"));
  assert.equal(report.publishes, 11);
  assert.equal(report.lane, "2025-11-25");
  for (const kind of ["raw", "transcript", "normalized"]) readJson(outFile("basic", "driver11", "stdio", kind));
});

test("basic.json with expectPublishes 10 exits 1 (too many publishes)", { timeout: 120_000 }, async (t) => {
  const res = await driver(basicWith(t, { expectPublishes: 10 }), "driver10", "stdio");
  assert.equal(res.code, 1, res.stderr);
  const report = readJson(outFile("basic", "driver10", "stdio", "report"));
  assert.equal(report.publishes, 11);
  assert.equal(report.expected, 10);
});

for (const transport of ["stdio", "http"]) {
  test(`a publish sent after the client closes is counted (${transport})`, { timeout: 120_000 }, async (t) => {
    const env = { FIXTURE_LATE_PUBLISH: "1" };
    const res = await driver(oneCall(t, 2), "late", transport, [], { server: TS_SERVER_PLUS, env });
    assert.equal(res.code, 0, res.stderr);
    const report = readJson(outFile("one", "late", transport, "report"));
    assert.equal(report.publishes, 2);
    const raw = readJson(outFile("one", "late", transport, "raw"));
    assert.ok(raw.some((r) => r.body?.event_type === "fixture:late"), "the late publish is in the snapshot");
  });
}

test("a request other than a publish makes the driver exit 1 and is in the report", { timeout: 120_000 }, async (t) => {
  const env = { FIXTURE_EXTRA_REQUEST: "1" };
  const res = await driver(oneCall(t, 1), "extra", "stdio", [], { server: TS_SERVER_PLUS, env });
  assert.equal(res.code, 1, res.stderr);
  const report = readJson(outFile("one", "extra", "stdio", "report"));
  assert.equal(report.publishes, 1);
  assert.equal(report.nonPublishRequests, 1);
  assert.deepEqual(report.report.nonPublishRequests, [{ method: "POST", path: "/v1/public/events", query: "", status: 404 }]);
  const normalized = readJson(outFile("one", "extra", "stdio", "normalized"));
  assert.deepEqual(normalized.nonPublishRequests, report.report.nonPublishRequests);
});

test("the driver exits 2 on a lane other than 2025-11-25", async () => {
  for (const lane of ["2026-07-28", "2024-11-05", ""]) {
    const res = await run(process.execPath, [DRIVER, BASIC, "lane", "stdio", "--lane", lane, "--", ...TS_SERVER]);
    assert.equal(res.code, 2, `lane ${JSON.stringify(lane)}: ${res.stderr}`);
  }
});

test("the driver exits 2 on an unknown transport or a missing server command", async () => {
  let res = await run(process.execPath, [DRIVER, BASIC, "x", "sse", "--", ...TS_SERVER]);
  assert.equal(res.code, 2, res.stderr);
  res = await run(process.execPath, [DRIVER, BASIC, "x", "stdio", "--"]);
  assert.equal(res.code, 2, res.stderr);
});

test("ts-v1 exits 2 on an invalid options profile", { timeout: 60_000 }, async (t) => {
  const [cmd, ...args] = TS_SERVER;
  const profiles = [
    { sessionHook: "x" }, // the spike's key: unknown now
    { identify: { returns: null, throws: "both" } },
    { eventTags: { returns: {}, sleepMs: -1 } },
    { eventProperties: { replace: "only for redaction" } },
    { redactSensitiveInformation: { replace: "" } },
    [],
  ];
  for (const profile of profiles) {
    const res = await run(cmd, [...args, "stdio"], {
      env: {
        ...process.env,
        AGENTCAT_API_URL: "http://127.0.0.1:9",
        DISABLE_DIAGNOSTICS: "true",
        HOME: tempDir(t),
        HARNESS_PROJECT_ID: "proj_HARNESS00000000000000000000",
        HARNESS_OPTIONS: JSON.stringify(profile),
      },
    });
    assert.equal(res.code, 2, `${JSON.stringify(profile)}: ${res.stderr}`);
    assert.match(res.stderr, /HARNESS_OPTIONS/);
  }
});

test("the committed tool contract matches the TS server (dump-contract --check)", { timeout: 120_000 }, async () => {
  const res = await run(process.execPath, [DUMP_CONTRACT, "--check"]);
  assert.equal(res.code, 0, res.stderr);
});

test("the harness and the TS clone resolve the same @modelcontextprotocol/sdk 1.31.x", () => {
  const version = (dir) => readJson(join(dir, "node_modules", "@modelcontextprotocol", "sdk", "package.json")).version;
  const ours = version(HARNESS);
  assert.match(ours, /^1\.31\.\d+$/);
  assert.equal(version(tsDir()), ours);
});

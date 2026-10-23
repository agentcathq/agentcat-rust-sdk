// The driver fails a run whose publish count differs from expectPublishes,
// and rejects bad arguments; the TS server rejects a bad options profile.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { before, test } from "node:test";

import { ensureTs } from "oracle/fetch-ts";
import { lock, tsDir } from "oracle/paths";

import {
  BASIC,
  DRIVER,
  DUMP_CONTRACT,
  HARNESS,
  TS_SERVER,
  TS_SERVER_PLUS,
  TS_V2_SERVER,
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
  assert.equal(report.ts_sha, lock().sha);
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
    assert.ok(raw.some((r) => r.body?.resource_name === "fixture:late"), "the late publish is in the snapshot");
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

test("the driver exits 2 on a lane other than 2025-11-25 and 2026-07-28", async () => {
  for (const lane of ["2026-07-29", "2024-11-05", ""]) {
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

test("the committed tool contract matches the TS v2 server (dump-contract --check)", { timeout: 120_000 }, async () => {
  const res = await run(process.execPath, [DUMP_CONTRACT, "--check"]);
  assert.equal(res.code, 0, res.stderr);
});

test("the harness and the TS clone resolve the same @modelcontextprotocol/sdk 1.31.x", () => {
  const version = (dir) => readJson(join(dir, "node_modules", "@modelcontextprotocol", "sdk", "package.json")).version;
  const ours = version(HARNESS);
  assert.match(ours, /^1\.31\.\d+$/);
  assert.equal(version(tsDir()), ours);
});

// ---------------------------------------------------------------------------
// Task 10: stricter driver and scenarios.
// ---------------------------------------------------------------------------
const SES = "ses_2bWv1nVbHvnL4lsnp1tqXjqGWQz";
const LANE_2026 = "2026-07-28";

/** A scenario of one session with the given steps; `extra` overrides top-level keys. */
function scenarioOf(t, name, steps, extra = {}) {
  return writeJson(join(tempDir(t), `${name}.json`), {
    name,
    projectId: readJson(BASIC).projectId,
    options: {},
    expectPublishes: 0,
    sessions: [{ steps }],
    ...extra,
  });
}

test("--expect-publishes overrides the scenario count: tracked exits 1 on a mismatch, untracked exits 0", { timeout: 120_000 }, async (t) => {
  const path = oneCall(t, 1);
  let res = await driver(path, "expect0", "stdio", ["--expect-publishes", "0"]);
  assert.equal(res.code, 1, res.stderr);
  assert.match(res.stderr, /1 publishes, expected exactly 0/);
  const report = readJson(outFile("one", "expect0", "stdio", "report"));
  assert.equal(report.expected, 0);
  assert.equal(report.publishes, 1);

  res = await driver(path, "expect0u", "stdio", ["--expect-publishes", "0"], { env: { HARNESS_NO_TRACK: "1" } });
  assert.equal(res.code, 0, res.stderr);
  assert.equal(readJson(outFile("one", "expect0u", "stdio", "report")).expected, 0);
});

test("--expect-publishes needs a non-negative integer", async () => {
  for (const value of ["-1", "x", "1.5", ""]) {
    const res = await run(process.execPath, [DRIVER, BASIC, "x", "stdio", "--expect-publishes", value, "--", ...TS_SERVER]);
    assert.equal(res.code, 2, `${JSON.stringify(value)}: ${res.stderr}`);
  }
  const res = await run(process.execPath, [DRIVER, BASIC, "x", "stdio", "--expect-publishes", "--", ...TS_SERVER]);
  assert.equal(res.code, 2, res.stderr);
});

test("a tracked saveHandleAs on a call that issues no handle exits 1 with the step and the name", { timeout: 120_000 }, async (t) => {
  // A valid session_id is accepted as given: the result carries no mint-back text and no structured content.
  const path = scenarioOf(t, "nohandle", [
    { op: "callTool", name: "echo", args: { msg: "hi", session_id: SES }, saveHandleAs: "S1" },
  ]);
  const res = await driver(path, "nohandle", "stdio");
  assert.equal(res.code, 1, res.stderr);
  assert.match(res.stderr, /^driver: step 0 of session 0: no session handle for S1$/m);
});

test("a tracked ${NAME} with no saved value exits 1 with the step and the name", { timeout: 120_000 }, async (t) => {
  const path = scenarioOf(t, "unsaved", [
    { op: "listTools" },
    { op: "callTool", name: "echo", args: { msg: "x", session_id: "${NOPE}" } },
  ]);
  const res = await driver(path, "unsaved", "stdio");
  assert.equal(res.code, 1, res.stderr);
  assert.match(res.stderr, /^driver: step 1 of session 0: no session handle for NOPE$/m);
});

test("an untracked run keeps ${NAME} literally", { timeout: 120_000 }, async (t) => {
  const path = scenarioOf(t, "literal", [
    { op: "callTool", name: "echo", args: { msg: "first" }, saveHandleAs: "S1" },
    { op: "callTool", name: "echo", args: { msg: "${S1}" } },
  ]);
  const res = await driver(path, "literal", "stdio", [], { env: { HARNESS_NO_TRACK: "1" } });
  assert.equal(res.code, 0, res.stderr);
  const transcript = readJson(outFile("literal", "literal", "stdio", "transcript"));
  const call = transcript.filter((e) => e.op === "callTool")[1];
  assert.equal(call.request.arguments.msg, "${S1}");
  assert.deepEqual(call.result.content, [{ type: "text", text: "${S1}" }]);
});

test("a v1 server on --lane 2026-07-28 falls back, and the driver exits 1", { timeout: 120_000 }, async (t) => {
  const path = scenarioOf(t, "fallback", [{ op: "listTools" }]);
  const res = await driver(path, "fallback", "stdio", ["--lane", LANE_2026], { env: { HARNESS_NO_TRACK: "1" } });
  assert.equal(res.code, 1, res.stderr);
  assert.match(res.stderr, /^driver: negotiated 2025-11-25, expected 2026-07-28$/m);
});

for (const [lane, extra] of [
  ["2025-11-25", []],
  [LANE_2026, ["--lane", LANE_2026]],
]) {
  test(`the initialize entry holds protocolVersion, serverInfo, capabilities and instructions (${lane})`, { timeout: 120_000 }, async (t) => {
    const path = scenarioOf(t, "init", [{ op: "listTools" }]);
    const res = await driver(path, `init${lane.slice(0, 4)}`, "stdio", extra, { server: TS_V2_SERVER, env: { HARNESS_NO_TRACK: "1" } });
    assert.equal(res.code, 0, res.stderr);
    const [entry] = readJson(outFile("init", `init${lane.slice(0, 4)}`, "stdio", "transcript"));
    assert.equal(entry.op, "initialize");
    assert.deepEqual(Object.keys(entry.result).sort(), ["capabilities", "instructions", "protocolVersion", "serverInfo"]);
    assert.equal(entry.result.protocolVersion, lane);
    assert.deepEqual(entry.result.serverInfo.name, "harness-ref-server");
    assert.equal(entry.result.serverInfo.version, "1.0.0");
    assert.ok(entry.result.capabilities.tools, "the tools capability");
  });
}

test("the scenario check rejects unknown keys and unknown expectPublishes keys (exit 2)", async (t) => {
  const steps = [{ op: "callTool", name: "echo", args: { msg: "hi" } }];
  const base = { name: "keys", projectId: readJson(BASIC).projectId, expectPublishes: 0, sessions: [{ steps }] };
  const bad = {
    "a top-level key": { ...base, expectPublishe: 1 },
    "a session key": { ...base, sessions: [{ steps, clients: {} }] },
    "a callTool step key": { ...base, sessions: [{ steps: [{ ...steps[0], saveAs: "S1" }] }] },
    "a listTools step key": { ...base, sessions: [{ steps: [{ op: "listTools", name: "echo" }] }] },
    "a transport key": { ...base, expectPublishes: { stdio: 0, sse: 0 } },
    "a lane in the key": { ...base, expectPublishes: { stdio: 0, "stdio@2026-07-29": 0 } },
    "an unknown transport with a lane": { ...base, expectPublishes: { stdio: 0, "sse@2026-07-28": 0 } },
    "an empty key": { ...base, expectPublishes: { stdio: 0, "": 0 } },
  };
  for (const [what, scenario] of Object.entries(bad)) {
    const path = writeJson(join(tempDir(t), "bad.json"), scenario);
    const res = await driver(path, "badkeys", "stdio");
    assert.equal(res.code, 2, `${what}: ${res.stderr}`);
  }
  const good = writeJson(join(tempDir(t), "good.json"), {
    ...base,
    expectPublishes: { stdio: 0, http: 0, "http-stateless": 0, "stdio@2025-11-25": 0, "http@2026-07-28": 0 },
  });
  const res = await driver(good, "goodkeys", "stdio", [], { env: { HARNESS_NO_TRACK: "1" } });
  assert.equal(res.code, 0, res.stderr);
});

test("expectPublishes falls back from <transport>@<lane> to <transport>", { timeout: 120_000 }, async (t) => {
  const path = scenarioOf(t, "fallbackcount", [{ op: "callTool", name: "echo", args: { msg: "hi" } }], {
    expectPublishes: { stdio: 1, http: 7, "http@2026-07-28": 9 },
  });
  const res = await driver(path, "fbcount", "stdio");
  assert.equal(res.code, 0, res.stderr);
  const report = readJson(outFile("fallbackcount", "fbcount", "stdio", "report"));
  assert.equal(report.expected, 1);
  assert.equal(report.publishes, 1);
});

test("on a driver error every connected client is closed, so an SDK-spawned stdio server ends", { timeout: 120_000 }, async (t) => {
  const pidFile = join(tempDir(t), "server.pid");
  // The wrapper execs the server (same pid) and drops its stderr, so the driver's stderr pipe does not
  // stay open while the server lives: the driver's exit is the moment to look.
  const server = ["sh", "-c", 'echo $$ > "$PIDFILE"; exec "$@" 2>/dev/null', "sh", ...TS_SERVER];
  const path = scenarioOf(t, "closeall", [
    { op: "callTool", name: "echo", args: { msg: "hi", session_id: SES }, saveHandleAs: "S1" },
  ]);
  const res = await driver(path, "closeall", "stdio", [], { server, env: { PIDFILE: pidFile } });
  assert.equal(res.code, 1, res.stderr);
  const pid = Number(readFileSync(pidFile, "utf8"));
  assert.ok(pid > 0);
  let alive = true;
  try {
    process.kill(pid, 0);
  } catch {
    alive = false;
  }
  assert.equal(alive, false, `server ${pid} still runs after the driver exited`);
});

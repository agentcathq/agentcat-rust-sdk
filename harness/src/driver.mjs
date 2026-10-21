// Language-neutral scenario driver. ONE client implementation (the official TS
// MCP client) drives ANY reference server named by a command line, over stdio
// or streamable HTTP, following a JSON scenario file. The server under test is
// a black box: the TS and Rust reference servers run under the same driver.
//
// usage: node harness/src/driver.mjs <scenario.json> <label> <stdio|http|http-stateless>
//          [--lane 2025-11-25|2026-07-28] -- <server cmd...>
//   label      output file tag, e.g. "ts" or "rust"
//   transport  the server is started as `<server cmd...> stdio`, `... http 0` or `... http-stateless 0`
//   --lane     the client's protocol lane: 2025-11-25 (default; the v1 client, initialize
//              handshake) or 2026-07-28 (the v2 client, versionNegotiation "auto")
// outputs: harness/out/<scenario>.<label>.<transport>.{raw,transcript,normalized,report}.json
// After the steps, the driver closes the clients, waits for the servers to exit
// and keeps the mock open for a final quiet window before it counts.
// Every publish body is then validated against harness/contract/publish-event.schema.json
// (src/contract/validate.mjs); report.json records the result as `contract` and the
// number of bodies checked as `contractChecked`. No body to check is a contract failure
// unless the scenario expects no publish (an untracked run).
// Exit 0: exactly expectPublishes publishes, no other request reached the mock,
//         and the contract holds (every publish body is valid).
// Exit 1: the count differs, another request arrived, the contract fails, or the
//         run failed.
// Exit 2: bad arguments or scenario.
import { spawn } from "node:child_process";
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";

import { Client as V2Client, StreamableHTTPClientTransport as V2StreamableHTTPClientTransport } from "@modelcontextprotocol/client";
import { StdioClientTransport as V2StdioClientTransport } from "@modelcontextprotocol/client/stdio";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";

import { NO_BODIES, contractResult, formatInvalid } from "./contract/validate.mjs";
import { normalizeRun } from "./normalize.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const OUT_DIR = join(HERE, "..", "out");
const TRANSPORTS = ["stdio", "http", "http-stateless"];
// 2025-11-25: the official TS client 1.31 (@modelcontextprotocol/sdk).
// 2026-07-28: the v2 client (@modelcontextprotocol/client) with versionNegotiation
// "auto", as connectV2Client in the TS repo's src/tests/e2e-http/lanes.ts.
const LANES = ["2025-11-25", "2026-07-28"];
const PUBLISH_PATH = "/v1/public/events/";
const USAGE =
  "usage: node harness/src/driver.mjs <scenario.json> <label> <stdio|http|http-stateless> [--lane 2025-11-25|2026-07-28] -- <server cmd...>";

function usage(message) {
  process.stderr.write(`driver: ${message}\n${USAGE}\n`);
  process.exit(2);
}

// ---------------------------------------------------------------------------
// Arguments and scenario.
// ---------------------------------------------------------------------------
const argv = process.argv.slice(2);
const sep = argv.indexOf("--");
if (sep < 0) usage("missing -- before the server command");
const serverCmd = argv.slice(sep + 1);
if (serverCmd.length === 0) usage("missing server command after --");
const head = argv.slice(0, sep);
let lane = LANES[0];
const laneAt = head.indexOf("--lane");
if (laneAt >= 0) {
  if (laneAt + 1 >= head.length) usage("--lane needs a value");
  lane = head[laneAt + 1];
  head.splice(laneAt, 2);
}
if (!LANES.includes(lane)) usage(`unsupported lane ${JSON.stringify(lane)} (supported: ${LANES.join(", ")})`);
if (head.length !== 3) usage("expected <scenario.json> <label> <transport>");
const [scenarioPath, label, transport] = head;
if (!/^[A-Za-z0-9_-]+$/.test(label)) usage(`bad label ${JSON.stringify(label)} (letters, digits, _ and - only)`);
if (!TRANSPORTS.includes(transport)) usage(`unknown transport ${JSON.stringify(transport)}`);

let scenario;
try {
  scenario = JSON.parse(readFileSync(scenarioPath, "utf8"));
} catch (err) {
  usage(`${scenarioPath}: ${err.message}`);
}
const expected = checkScenario(scenario);

/** Validates the scenario; returns the publish count expected on this transport and lane. */
function checkScenario(s) {
  if (!s || typeof s !== "object") usage("scenario must be a JSON object");
  if (typeof s.name !== "string" || !/^[A-Za-z0-9_-]+$/.test(s.name)) usage("scenario.name must be letters, digits, _ and -");
  if (typeof s.projectId !== "string" || s.projectId === "") usage("scenario.projectId must be a non-empty string");
  if (!Array.isArray(s.sessions) || s.sessions.length === 0) usage("scenario.sessions must be a non-empty array");
  for (const [si, session] of s.sessions.entries()) {
    if (!Array.isArray(session?.steps)) usage(`sessions[${si}].steps must be an array`);
    for (const [i, step] of session.steps.entries()) {
      if (step?.op === "listTools") continue;
      if (step?.op === "callTool" && typeof step.name === "string") continue;
      usage(`sessions[${si}].steps[${i}]: op must be listTools, or callTool with a name`);
    }
  }
  // A number, or an object keyed by `<transport>@<lane>` or `<transport>` (first match wins).
  const e = s.expectPublishes;
  const n = typeof e === "number" ? e : e && typeof e === "object" ? (e[`${transport}@${lane}`] ?? e[transport]) : undefined;
  if (!Number.isInteger(n) || n < 0) usage(`expectPublishes has no count for ${transport}@${lane} or ${transport}`);
  return n;
}

// ---------------------------------------------------------------------------
// Processes.
// ---------------------------------------------------------------------------
function readyLine(child, what) {
  return new Promise((resolve, reject) => {
    const rl = createInterface({ input: child.stdout });
    const onExit = (code, signal) => reject(new Error(`${what} exited before its ready line (${signal ?? code})`));
    child.once("exit", onExit);
    child.once("error", reject);
    rl.once("line", (line) => {
      child.off("exit", onExit);
      try {
        resolve(JSON.parse(line));
      } catch {
        reject(new Error(`${what}: bad ready line: ${line}`));
      }
    });
  });
}

const base = `${scenario.name}.${label}.${transport}`;
mkdirSync(OUT_DIR, { recursive: true });
// HOME for the server (the TS SDK writes ~/agentcat.log there); kept for debugging.
const home = join(OUT_DIR, "home", base);
rmSync(home, { recursive: true, force: true });
mkdirSync(home, { recursive: true });

const countPublishes = (requests) => requests.filter((r) => r.method === "POST" && r.path === PUBLISH_PATH).length;
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const isRunning = (child) => child.exitCode === null && child.signalCode === null;
const exited = (child) => (isRunning(child) ? new Promise((resolve) => child.once("exit", resolve)) : Promise.resolve());
/** Resolves true when `promise` settles before `deadline` (epoch ms), false otherwise. */
const within = (promise, deadline) =>
  Promise.race([promise.then(() => true), sleep(Math.max(0, deadline - Date.now())).then(() => false)]);

const children = [];
function killAll() {
  for (const child of children) if (isRunning(child)) child.kill("SIGTERM");
}

const mock = spawn(process.execPath, [join(HERE, "mock-api.mjs"), "0"], { stdio: ["ignore", "pipe", "inherit"] });
children.push(mock);

async function main() {
  const { port: mockPort } = await readyLine(mock, "mock API");
  const mockUrl = `http://127.0.0.1:${mockPort}`;
  const getRequests = async () => (await fetch(`${mockUrl}/__requests`)).json();

  const serverEnv = {
    ...process.env,
    HOME: home,
    USERPROFILE: home,
    AGENTCAT_API_URL: mockUrl,
    DISABLE_DIAGNOSTICS: "true",
    HARNESS_PROJECT_ID: scenario.projectId,
    HARNESS_OPTIONS: JSON.stringify(scenario.options ?? {}),
  };

  let httpUrl;
  let httpChild;
  if (transport !== "stdio") {
    httpChild = spawn(serverCmd[0], [...serverCmd.slice(1), transport, "0"], {
      env: serverEnv,
      stdio: ["ignore", "pipe", "inherit"],
    });
    children.push(httpChild);
    ({ url: httpUrl } = await readyLine(httpChild, "server"));
  }

  async function connect(clientInfo, negotiated) {
    if (lane === "2026-07-28") return connectV2(clientInfo, negotiated);
    const client = new Client(clientInfo, { capabilities: {} });
    const t =
      transport === "stdio"
        ? new StdioClientTransport({
            command: serverCmd[0],
            args: [...serverCmd.slice(1), "stdio"],
            env: serverEnv,
            stderr: "inherit",
          })
        : new StreamableHTTPClientTransport(new URL(httpUrl), { requestInit: { headers: scenario.httpHeaders ?? {} } });
    // The client hands the negotiated protocol version to transports that take it.
    const setVersion = t.setProtocolVersion?.bind(t);
    t.setProtocolVersion = (version) => {
      negotiated.protocolVersion = version;
      setVersion?.(version);
    };
    await client.connect(t);
    return client;
  }

  async function connectV2(clientInfo, negotiated) {
    const client = new V2Client(clientInfo, { capabilities: {}, versionNegotiation: { mode: "auto" } });
    const t =
      transport === "stdio"
        ? new V2StdioClientTransport({
            command: serverCmd[0],
            args: [...serverCmd.slice(1), "stdio"],
            env: serverEnv,
            stderr: "inherit",
          })
        : new V2StreamableHTTPClientTransport(new URL(httpUrl), { requestInit: { headers: scenario.httpHeaders ?? {} } });
    await client.connect(t);
    // "auto" falls back to initialize when the server does not prove the
    // 2026-07-28 era; the transcript keeps what was negotiated.
    negotiated.protocolVersion = client.getNegotiatedProtocolVersion();
    return client;
  }

  const vars = {};
  const subst = (v) =>
    typeof v === "string"
      ? v.replace(/\$\{(\w+)\}/g, (_, k) => vars[k] ?? `\${${k}}`)
      : Array.isArray(v)
        ? v.map(subst)
        : v && typeof v === "object"
          ? Object.fromEntries(Object.entries(v).map(([k, x]) => [k, subst(x)]))
          : v;

  function handleFrom(result) {
    const sc = result?.structuredContent?.mcp_session?.session_id;
    if (sc) return sc;
    for (const block of result?.content ?? []) {
      const m = typeof block.text === "string" && block.text.match(/session_id: (ses_[0-9A-Za-z]{27})/);
      if (m) return m[1];
    }
    return undefined;
  }

  const transcript = [];
  const clients = [];
  for (const [si, session] of scenario.sessions.entries()) {
    const negotiated = {};
    const client = await connect(session.client ?? { name: "harness-client", version: "1.0.0" }, negotiated);
    clients.push(client);
    // The protocol version the server agreed to on this lane.
    transcript.push({ session: si, op: "initialize", result: { protocolVersion: negotiated.protocolVersion ?? null } });
    for (const step of session.steps) {
      const entry = { session: si, op: step.op };
      try {
        if (step.op === "listTools") {
          entry.result = await client.listTools(step.cursor ? { cursor: step.cursor } : undefined);
        } else {
          entry.request = { name: step.name, arguments: subst(step.args ?? {}) };
          entry.result = await client.callTool(entry.request);
          if (step.saveHandleAs) vars[step.saveHandleAs] = handleFrom(entry.result);
        }
      } catch (e) {
        // Protocol errors (JSON-RPC errors) are part of the observable contract.
        entry.error = { name: e?.constructor?.name, code: e?.code, message: String(e?.message ?? e) };
      }
      transcript.push(entry);
    }
  }

  const settleTimeoutMs = scenario.settleTimeoutMs ?? 10_000;
  const quietMs = scenario.quietMs ?? 500;
  /** Polls the mock until `done(requests)` holds and no request arrived for quietMs, or the deadline. */
  async function settle(done, deadline) {
    let last = -1;
    let stableSince = Date.now();
    for (;;) {
      const seen = await getRequests();
      if (seen.length !== last) {
        last = seen.length;
        stableSince = Date.now();
      }
      if (done(seen) && Date.now() - stableSince >= quietMs) return;
      if (Date.now() > deadline) return;
      await sleep(50);
    }
  }

  // 1. Let the events of the steps arrive while the clients are connected.
  await settle((seen) => countPublishes(seen) >= expected, Date.now() + settleTimeoutMs);
  // 2. Close the clients and wait (bounded) for the servers to exit, so a
  // publish sent on client close or during the shutdown drain is counted.
  // A stdio server exits when its stdin closes (the SDK's stdio client
  // transport waits 2 s, then sends SIGTERM); an HTTP server gets SIGTERM.
  const exitDeadline = Date.now() + settleTimeoutMs;
  await within(Promise.all(clients.map((c) => c.close().catch(() => {}))), exitDeadline);
  if (httpChild && isRunning(httpChild)) {
    httpChild.kill("SIGTERM");
    if (!(await within(exited(httpChild), exitDeadline))) {
      process.stderr.write(`driver: the server did not exit within ${settleTimeoutMs} ms of SIGTERM; killing it\n`);
      httpChild.kill("SIGKILL");
    }
  }
  // 3. Keep the mock open for a final quiet window, then take the snapshot.
  await settle(() => true, Date.now() + settleTimeoutMs);
  const requests = await getRequests();
  killAll();

  const outBase = join(OUT_DIR, base);
  const write = (kind, value) => writeFileSync(`${outBase}.${kind}.json`, `${JSON.stringify(value, null, 2)}\n`);
  write("raw", requests);
  write("transcript", transcript);
  const { normalized, report } = await normalizeRun({ requests, transcript, scenario });
  write("normalized", normalized);
  const publishes = countPublishes(requests);
  // Any request other than a publish is a finding (an unexpected API call).
  const nonPublishRequests = report.nonPublishRequests.length;
  // L4: every publish body must be one the backend accepts: "ok", NO_BODIES or the
  // invalid bodies. Zero bodies passes only when the scenario expects zero publishes.
  const { checked: contractChecked, contract } = contractResult(requests, { allowEmpty: expected === 0 });
  const summary = {
    scenario: scenario.name,
    label,
    transport,
    lane,
    publishes,
    expected,
    nonPublishRequests,
    contract,
    contractChecked,
    report,
  };
  write("report", summary);
  const contractLine = typeof contract === "string" ? contract : `${contract.length} invalid`;
  process.stdout.write(
    `${JSON.stringify({ scenario: scenario.name, label, transport, lane, publishes, expected, nonPublishRequests, contract: contractLine })}\n`,
  );
  if (publishes !== expected) process.stderr.write(`driver: ${publishes} publishes, expected exactly ${expected}\n`);
  if (nonPublishRequests > 0) {
    process.stderr.write(`driver: ${nonPublishRequests} request(s) other than a publish: ${JSON.stringify(report.nonPublishRequests)}\n`);
  }
  if (contract === NO_BODIES) {
    process.stderr.write(`driver: contract: no publish bodies to check (the scenario expects ${expected})\n`);
  } else if (contract !== "ok") {
    process.stderr.write(`driver: ${contract.length} publish body/bodies fail the contract (harness/contract/publish-event.schema.json):\n`);
    for (const line of formatInvalid(contract)) process.stderr.write(`driver: ${line}\n`);
  }
  return publishes === expected && nonPublishRequests === 0 && contract === "ok" ? 0 : 1;
}

main().then(
  (code) => process.exit(code),
  (err) => {
    killAll();
    process.stderr.write(`driver: ${err?.stack ?? err}\n`);
    process.exit(1);
  },
);

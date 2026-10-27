// Language-neutral scenario driver. ONE client implementation (the official TS
// MCP client) drives ANY reference server named by a command line, over stdio
// or streamable HTTP, following a JSON scenario file. The server under test is
// a black box: the TS and Rust reference servers run under the same driver.
//
// usage: node harness/src/driver.mjs <scenario.json> <label> <stdio|http|http-stateless>
//          [--lane 2025-11-25|2026-07-28] [--expect-publishes <n>] [--expect-drained] -- <server cmd...>
//   label      output file tag, e.g. "ts" or "rust"
//   transport  the server is started as `<server cmd...> stdio`, `... http 0` or `... http-stateless 0`
//   --lane     the client's protocol lane: 2025-11-25 (default; the v1 client, initialize
//              handshake) or 2026-07-28 (the v2 client, versionNegotiation "auto"). A session
//              that negotiates another version fails the run (exit 1).
//   --expect-publishes  the publish count to expect, instead of the scenario's expectPublishes
//   --expect-drained    every server process must print the line {"drained":true} on stderr before it
//              exits (the tracked Rust server's drain, decision C46)
// outputs: harness/out/<scenario>.<label>.<transport>.{raw,transcript,normalized,report}.json
// Before each connect and each step, the driver waits until the wall clock is past the millisecond
// in which the previous request was answered: a server stamps an event between a request's arrival and
// its answer, so the events of two requests in a row never share a millisecond, and the time order of
// the normalizer is the request order on every server and every run. A parallel step (its callTool
// steps at once, recorded in step order; an inner step may name an earlier session's client) waits
// once, as a whole. A listTools step lists one page (cursor: the step's `cursor`; `saveCursorAs` keeps
// the result's nextCursor); a scenario's pageSize becomes the server's HARNESS_PAGE_SIZE.
// After the steps, the driver closes the clients, waits for the servers to exit
// and keeps the mock open for a final quiet window before it counts. The servers'
// stderr is piped through the driver's stderr; report.json records per server
// process whether it printed {"drained":true} (`drained`) and whether an HTTP
// server needed SIGKILL after SIGTERM (`killed`).
// Every publish body is then validated against harness/contract/publish-event.schema.json
// (src/contract/validate.mjs); report.json records the result as `contract` and the
// number of bodies checked as `contractChecked`, and the TS commit of the oracle
// (oracle/oracle.lock, or ORACLE_SHA) as `ts_sha`. No body to check is a contract failure
// unless the scenario expects no publish (an untracked run).
// Exit 0: exactly expectPublishes publishes, no other request reached the mock,
//         the contract holds (every publish body is valid), no server needed
//         SIGKILL and, with --expect-drained, every server printed {"drained":true}.
// Exit 1: the count differs, another request arrived, the contract fails, an HTTP
//         server needed SIGKILL after SIGTERM, with --expect-drained a server exited
//         without printing {"drained":true}, or the run failed (a negotiated version
//         other than --lane; in a tracked run, a saveHandleAs that finds no handle or
//         a ${NAME} with no saved value; in any run, a saveCursorAs without a nextCursor
//         or a ${NAME} in a cursor that no saveCursorAs saved).
//         On a run failure every connected client is closed first.
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

import { lock, traceOverride } from "oracle/paths";

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
  "usage: node harness/src/driver.mjs <scenario.json> <label> <stdio|http|http-stateless> [--lane 2025-11-25|2026-07-28] [--expect-publishes <n>] [--expect-drained] -- <server cmd...>";
/** The line a tracked Rust server prints on stderr once its drain succeeded. */
const DRAINED_LINE = '{"drained":true}';

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
let expectOverride;
const expectAt = head.indexOf("--expect-publishes");
if (expectAt >= 0) {
  if (expectAt + 1 >= head.length) usage("--expect-publishes needs a value");
  const text = head[expectAt + 1];
  if (!/^\d+$/.test(text)) usage(`--expect-publishes needs a non-negative integer, got ${JSON.stringify(text)}`);
  expectOverride = Number(text);
  head.splice(expectAt, 2);
}
const drainedAt = head.indexOf("--expect-drained");
const expectDrained = drainedAt >= 0;
if (expectDrained) head.splice(drainedAt, 1);
if (!LANES.includes(lane)) usage(`unsupported lane ${JSON.stringify(lane)} (supported: ${LANES.join(", ")})`);
if (head.length !== 3) usage("expected <scenario.json> <label> <transport>");
const [scenarioPath, label, transport] = head;
if (!/^[A-Za-z0-9_-]+$/.test(label)) usage(`bad label ${JSON.stringify(label)} (letters, digits, _ and - only)`);
if (!TRANSPORTS.includes(transport)) usage(`unknown transport ${JSON.stringify(transport)}`);

const TOP_KEYS = ["name", "projectId", "options", "httpHeaders", "pageSize", "expectPublishes", "settleTimeoutMs", "quietMs", "eventOrder", "sessions"];
/** The values of the scenario key eventOrder (normalize.mjs): absent, the events sort by time. */
const EVENT_ORDERS = ["multiset"];
const STEP_KEYS = {
  listTools: ["op", "cursor", "saveCursorAs"],
  callTool: ["op", "name", "args", "saveHandleAs"],
  parallel: ["op", "steps"],
};
/** The keys of a callTool step inside a parallel step: `session` names the session whose client runs it. */
const INNER_KEYS = [...STEP_KEYS.callTool, "session"];
/** A name that saveHandleAs and saveCursorAs save under, and `${NAME}` reads. */
const VAR_NAME = /^\w+$/;

let scenario;
try {
  scenario = JSON.parse(readFileSync(scenarioPath, "utf8"));
} catch (err) {
  usage(`${scenarioPath}: ${err.message}`);
}
const expected = checkScenario(scenario);
const tracked = process.env.HARNESS_NO_TRACK !== "1";
const settleTimeoutMs = scenario.settleTimeoutMs ?? 10_000;
const quietMs = scenario.quietMs ?? 500;

/** A failure with a message of its own: the driver prints it without a stack and exits 1. */
class DriverFailure extends Error {}

/** Exits 2 when `obj` has a key outside `allowed`. */
function onlyKeys(obj, allowed, where) {
  for (const key of Object.keys(obj)) if (!allowed.includes(key)) usage(`${where}: unknown key ${JSON.stringify(key)}`);
}

/** Exits 2 unless `name`, when given, is a VAR_NAME; `key` names the field. */
function checkName(name, key, where) {
  if (name !== undefined && !(typeof name === "string" && VAR_NAME.test(name))) usage(`${where}: ${key} must be letters, digits and _`);
}

/** Exits 2 unless `step` is a callTool step with a name; `keys` are the keys it may have. */
function checkCall(step, keys, where) {
  if (typeof step.name !== "string") usage(`${where}: a callTool step needs a name`);
  onlyKeys(step, keys, where);
  checkName(step.saveHandleAs, "saveHandleAs", where);
}

/** Exits 2 on a bad step of session `si`: listTools, callTool, or parallel (callTool steps of this or an earlier session). */
function checkStep(step, where, si) {
  if (!step || typeof step !== "object" || Array.isArray(step) || !Object.hasOwn(STEP_KEYS, step.op)) {
    usage(`${where}: op must be listTools, callTool with a name, or parallel`);
  }
  if (step.op === "callTool") return checkCall(step, STEP_KEYS.callTool, where);
  onlyKeys(step, STEP_KEYS[step.op], where);
  if (step.op === "listTools") {
    if (step.cursor !== undefined && typeof step.cursor !== "string") usage(`${where}: cursor must be a string`);
    return checkName(step.saveCursorAs, "saveCursorAs", where);
  }
  if (!Array.isArray(step.steps) || step.steps.length === 0) usage(`${where}: steps must be a non-empty array of callTool steps`);
  for (const [k, inner] of step.steps.entries()) {
    const at = `${where}.steps[${k}]`;
    if (!inner || typeof inner !== "object" || inner.op !== "callTool") usage(`${at}: a parallel step holds callTool steps only`);
    checkCall(inner, INNER_KEYS, at);
    const s = inner.session;
    if (s !== undefined && !(Number.isInteger(s) && s >= 0 && s <= si)) usage(`${at}: session must be the index of this or an earlier session`);
  }
}

/** Validates the scenario; returns the publish count expected on this transport and lane. */
function checkScenario(s) {
  if (!s || typeof s !== "object" || Array.isArray(s)) usage("scenario must be a JSON object");
  onlyKeys(s, TOP_KEYS, "scenario");
  if (typeof s.name !== "string" || !/^[A-Za-z0-9_-]+$/.test(s.name)) usage("scenario.name must be letters, digits, _ and -");
  if (typeof s.projectId !== "string" || s.projectId === "") usage("scenario.projectId must be a non-empty string");
  if (s.pageSize !== undefined && !(Number.isInteger(s.pageSize) && s.pageSize > 0)) usage("scenario.pageSize must be a positive integer");
  if (s.eventOrder !== undefined && !EVENT_ORDERS.includes(s.eventOrder)) usage(`scenario.eventOrder must be one of ${EVENT_ORDERS.join(", ")}`);
  if (!Array.isArray(s.sessions) || s.sessions.length === 0) usage("scenario.sessions must be a non-empty array");
  for (const [si, session] of s.sessions.entries()) {
    if (!session || typeof session !== "object" || Array.isArray(session)) usage(`sessions[${si}] must be an object`);
    onlyKeys(session, ["client", "steps"], `sessions[${si}]`);
    if (!Array.isArray(session.steps)) usage(`sessions[${si}].steps must be an array`);
    for (const [i, step] of session.steps.entries()) checkStep(step, `sessions[${si}].steps[${i}]`, si);
  }
  // A number, or an object keyed by `<transport>` or `<transport>@<lane>` (first match wins).
  const e = s.expectPublishes;
  if (e !== undefined && typeof e === "object" && e !== null && !Array.isArray(e)) {
    const known = TRANSPORTS.flatMap((tr) => [tr, ...LANES.map((l) => `${tr}@${l}`)]);
    for (const key of Object.keys(e)) {
      if (!known.includes(key)) usage(`expectPublishes: unknown key ${JSON.stringify(key)} (use ${TRANSPORTS.join(", ")} or <transport>@<lane> for ${LANES.join(", ")})`);
    }
  }
  if (expectOverride !== undefined) return expectOverride;
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

/** Waits until the wall clock is past the millisecond `ms` (Date.now() > ms). */
async function pastMillisecond(ms) {
  while (Date.now() <= ms) await sleep(1);
}
const isRunning = (child) => child.exitCode === null && child.signalCode === null;
const exited = (child) => (isRunning(child) ? new Promise((resolve) => child.once("exit", resolve)) : Promise.resolve());
/** Resolves true when `promise` settles before `deadline` (epoch ms), false otherwise. */
const within = (promise, deadline) =>
  Promise.race([promise.then(() => true), sleep(Math.max(0, deadline - Date.now())).then(() => false)]);

/**
 * Pipes a server's stderr stream through the driver's stderr, line by line, and watches it for
 * DRAINED_LINE. `where` names the server in messages. `ended` resolves once the stream has ended and
 * every line is read.
 */
function watchStderr(stream, where) {
  const watch = { where, drained: false, ended: Promise.resolve() };
  if (!stream) return watch;
  const lines = createInterface({ input: stream, crlfDelay: Number.POSITIVE_INFINITY });
  watch.ended = new Promise((resolve) => lines.once("close", resolve));
  lines.on("line", (line) => {
    process.stderr.write(`${line}\n`);
    if (line.trim() === DRAINED_LINE) watch.drained = true;
  });
  return watch;
}

/** The stderr of every server process the driver started (stdio: one per session; HTTP: one). */
const stderrWatches = [];
/** Whether an HTTP server needed SIGKILL after SIGTERM. */
let killed = false;

const children = [];
/** Every connected client, so a failed run can close them (a stdio client ends the server it spawned). */
const clients = [];
function killAll() {
  for (const child of children) if (isRunning(child)) child.kill("SIGTERM");
}

const mock = spawn(process.execPath, [join(HERE, "mock-api.mjs"), "0"], { stdio: ["ignore", "pipe", "inherit"] });
children.push(mock);

async function main() {
  traceOverride();
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
  // The Rust SDK reads AGENTCAT_ENABLED (decision C45): a developer shell's value must not turn it off.
  delete serverEnv.AGENTCAT_ENABLED;
  // The scenario's pageSize pages tools/list; without it, no page size, whatever the caller's environment says.
  if (scenario.pageSize === undefined) delete serverEnv.HARNESS_PAGE_SIZE;
  else serverEnv.HARNESS_PAGE_SIZE = String(scenario.pageSize);

  let httpUrl;
  let httpChild;
  if (transport !== "stdio") {
    httpChild = spawn(serverCmd[0], [...serverCmd.slice(1), transport, "0"], {
      env: serverEnv,
      stdio: ["ignore", "pipe", "pipe"],
    });
    children.push(httpChild);
    stderrWatches.push(watchStderr(httpChild.stderr, "the server"));
    ({ url: httpUrl } = await readyLine(httpChild, "server"));
  }

  async function connect(clientInfo, negotiated, si) {
    if (lane === "2026-07-28") return connectV2(clientInfo, negotiated, si);
    const client = new Client(clientInfo, { capabilities: {} });
    const t =
      transport === "stdio"
        ? new StdioClientTransport({
            command: serverCmd[0],
            args: [...serverCmd.slice(1), "stdio"],
            env: serverEnv,
            stderr: "pipe",
          })
        : new StreamableHTTPClientTransport(new URL(httpUrl), { requestInit: { headers: scenario.httpHeaders ?? {} } });
    if (transport === "stdio") stderrWatches.push(watchStderr(t.stderr, `the server of session ${si}`));
    // The client hands the negotiated protocol version to transports that take it.
    const setVersion = t.setProtocolVersion?.bind(t);
    t.setProtocolVersion = (version) => {
      negotiated.protocolVersion = version;
      setVersion?.(version);
    };
    await client.connect(t);
    return client;
  }

  async function connectV2(clientInfo, negotiated, si) {
    const client = new V2Client(clientInfo, { capabilities: {}, versionNegotiation: { mode: "auto" } });
    const t =
      transport === "stdio"
        ? new V2StdioClientTransport({
            command: serverCmd[0],
            args: [...serverCmd.slice(1), "stdio"],
            env: serverEnv,
            stderr: "pipe",
          })
        : new V2StreamableHTTPClientTransport(new URL(httpUrl), { requestInit: { headers: scenario.httpHeaders ?? {} } });
    // The probe sibling of "auto" discards its own stderr; this is the session's server.
    if (transport === "stdio") stderrWatches.push(watchStderr(t.stderr, `the server of session ${si}`));
    await client.connect(t);
    // "auto" falls back to initialize when the server does not prove the
    // 2026-07-28 era; the transcript keeps what was negotiated.
    negotiated.protocolVersion = client.getNegotiatedProtocolVersion();
    return client;
  }

  const vars = {};
  // `${NAME}` becomes the saved handle. With no saved value a tracked run fails;
  // an untracked run (no handles are issued) keeps the text.
  const subst = (v, where) =>
    typeof v === "string"
      ? v.replace(/\$\{(\w+)\}/g, (_, k) => {
          if (vars[k] !== undefined) return vars[k];
          if (tracked) throw new DriverFailure(`${where}: no session handle for ${k}`);
          return `\${${k}}`;
        })
      : Array.isArray(v)
        ? v.map((x) => subst(x, where))
        : v && typeof v === "object"
          ? Object.fromEntries(Object.entries(v).map(([k, x]) => [k, subst(x, where)]))
          : v;

  const cursors = {};
  // `${NAME}` in a listTools cursor becomes the cursor saved under NAME by saveCursorAs. With none, the
  // run fails, tracked or not: the server would answer an unknown cursor with an error.
  const cursorOf = (text, where) =>
    text.replace(/\$\{(\w+)\}/g, (_, k) => {
      if (cursors[k] !== undefined) return cursors[k];
      throw new DriverFailure(`${where}: the cursor names ${k}, but no saveCursorAs saved it`);
    });

  function handleFrom(result) {
    const sc = result?.structuredContent?.mcp_session?.session_id;
    if (sc) return sc;
    for (const block of result?.content ?? []) {
      const m = typeof block.text === "string" && block.text.match(/session_id: (ses_[0-9A-Za-z]{27})/);
      if (m) return m[1];
    }
    return undefined;
  }

  /** Saves the handle of a callTool step's result under its saveHandleAs, if it has one. */
  function saveHandle(step, entry, where) {
    if (!step.saveHandleAs) return;
    const handle = handleFrom(entry.result);
    if (handle === undefined && tracked) throw new DriverFailure(`${where}: no session handle for ${step.saveHandleAs}`);
    vars[step.saveHandleAs] = handle;
  }

  /** A failed request: protocol errors (JSON-RPC errors) are part of the observable contract. */
  const errorOf = (e) => ({ name: e?.constructor?.name, code: e?.code, message: String(e?.message ?? e) });

  /**
   * Runs the callTool step `step` on the client of session `si`; resolves its transcript entry. The
   * request goes out before the first await, so the calls of a parallel step start in step order.
   */
  async function callStep(si, step, where) {
    const entry = { session: si, op: "callTool", request: { name: step.name, arguments: subst(step.args ?? {}, where) } };
    try {
      entry.result = await sessionClients[si].callTool(entry.request);
    } catch (e) {
      if (e instanceof DriverFailure) throw e;
      entry.error = errorOf(e);
    }
    return entry;
  }

  const transcript = [];
  /** The client of each session connected so far, by session index. */
  const sessionClients = [];
  // Date.now() when the previous request was answered (see pastMillisecond).
  let answeredAt = Number.NEGATIVE_INFINITY;
  for (const [si, session] of scenario.sessions.entries()) {
    const negotiated = {};
    await pastMillisecond(answeredAt);
    const client = await connect(session.client ?? { name: "harness-client", version: "1.0.0" }, negotiated, si);
    answeredAt = Date.now();
    clients.push(client);
    sessionClients[si] = client;
    // "auto" can fall back to initialize: the lane must be the one that was asked for.
    if (negotiated.protocolVersion !== lane) {
      throw new DriverFailure(`negotiated ${negotiated.protocolVersion ?? null}, expected ${lane}`);
    }
    // What the server agreed to, on this lane.
    transcript.push({
      session: si,
      op: "initialize",
      result: {
        protocolVersion: negotiated.protocolVersion,
        serverInfo: client.getServerVersion() ?? null,
        capabilities: client.getServerCapabilities() ?? null,
        instructions: client.getInstructions() ?? null,
      },
    });
    for (const [i, step] of session.steps.entries()) {
      const where = `step ${i} of session ${si}`;
      // A parallel step waits once, as a whole: its calls start together.
      await pastMillisecond(answeredAt);
      if (step.op === "parallel") {
        const inner = step.steps.map((s, k) => [s, `step ${i}.${k} of session ${si}`]);
        const entries = await Promise.all(inner.map(([s, at]) => callStep(s.session ?? si, s, at)));
        answeredAt = Date.now();
        // Recorded in step order, whatever order the answers came in.
        for (const [k, [s, at]] of inner.entries()) {
          saveHandle(s, entries[k], at);
          transcript.push(entries[k]);
        }
      } else if (step.op === "listTools") {
        const entry = { session: si, op: "listTools" };
        if (step.cursor !== undefined) entry.request = { cursor: cursorOf(step.cursor, where) };
        // A paged scenario lists one page per step on both lanes: without a cursor, the v2 client's
        // listTools() follows every nextCursor itself and drops it, so the driver sends the request.
        const firstPageOnly = scenario.pageSize !== undefined && lane === "2026-07-28" && entry.request === undefined;
        try {
          entry.result = firstPageOnly ? await client.request({ method: "tools/list" }) : await client.listTools(entry.request);
        } catch (e) {
          if (e instanceof DriverFailure) throw e;
          entry.error = errorOf(e);
        }
        answeredAt = Date.now();
        if (step.saveCursorAs) {
          const next = entry.result?.nextCursor;
          if (typeof next !== "string") throw new DriverFailure(`${where}: no nextCursor for ${step.saveCursorAs}`);
          cursors[step.saveCursorAs] = next;
        }
        transcript.push(entry);
      } else {
        const entry = await callStep(si, step, where);
        answeredAt = Date.now();
        saveHandle(step, entry, where);
        transcript.push(entry);
      }
    }
  }

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
      killed = true;
    }
  }
  // Every server has exited (or been killed): read the rest of its stderr.
  await within(Promise.all(stderrWatches.map((w) => w.ended)), Date.now() + settleTimeoutMs);
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
    ts_sha: lock().sha,
    publishes,
    expected,
    nonPublishRequests,
    contract,
    contractChecked,
    drained: stderrWatches.map((w) => w.drained),
    killed,
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
  if (killed) process.stderr.write("driver: the server needed SIGKILL after SIGTERM\n");
  const undrained = expectDrained ? stderrWatches.filter((w) => !w.drained) : [];
  for (const w of undrained) process.stderr.write(`driver: ${w.where} exited without printing ${DRAINED_LINE}\n`);
  return publishes === expected && nonPublishRequests === 0 && contract === "ok" && !killed && undrained.length === 0 ? 0 : 1;
}

main().then(
  (code) => process.exit(code),
  async (err) => {
    process.stderr.write(`driver: ${err instanceof DriverFailure ? err.message : (err?.stack ?? err)}\n`);
    // Close every client first: closing a stdio client ends the server it spawned, which killAll cannot reach.
    await within(Promise.all(clients.map((c) => c.close().catch(() => {}))), Date.now() + settleTimeoutMs);
    killAll();
    // The servers' last stderr lines, bounded: a stream that never ends must not hold the exit.
    await within(Promise.all(stderrWatches.map((w) => w.ended)), Date.now() + 2000);
    process.exit(1);
  },
);

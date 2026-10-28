// The examples check: runs a customer example of agentcat-sdk against the local mock API and asserts
// what a person who copies the example expects (plan 5, Task 10, decision C54).
//
// usage: node harness/src/examples-check.mjs <stdio | streamable_http | hooks | stateless_http>
//
// For one example the check starts the mock, builds the example (`cargo build -p agentcat-sdk
// --example <name> --all-features`), and runs it with AGENTCAT_API_URL set to the mock,
// AGENTCAT_PROJECT_ID=proj_examples and a temporary HOME (the SDK writes ~/agentcat.log there). It drives
// initialize, tools/list and three tool calls with the TS client, closes the client right after the last
// call (stdio: closes stdin; HTTP: closes the client, then sends SIGTERM) and asserts:
//   - the first call issues a session ID in a block before the tool's result, and keeps that result;
//   - each call returns the expected text;
//   - one tools/call publish per call, and for `hooks` the hook values and the custom event;
//   - every stdout line is a JSON-RPC message;
//   - the example exits 0 within 5 s of the stdin close or the SIGTERM;
//   - the publish of the last call arrived before the exit.
// Nothing here reaches a production host: the API URL is always the mock, and an empty URL stops the run.
import { spawn } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { connect, createServer } from "node:net";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";

import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";
import { serializeMessage } from "@modelcontextprotocol/sdk/shared/stdio.js";

const HERE = dirname(fileURLToPath(import.meta.url));
export const ROOT = join(HERE, "..", "..");

/** The project ID of every example run. */
export const PROJECT_ID = "proj_examples";
/** How long an example may take to exit after the stdin close or the SIGTERM. */
export const EXIT_BUDGET_MS = 5000;

/** The transport and the calls of each example. */
const EXAMPLES = {
  stdio: { http: false, calls: "plain" },
  streamable_http: { http: true, calls: "plain" },
  stateless_http: { http: true, calls: "plain" },
  hooks: { http: false, calls: "hooks" },
};

/** The three calls of each kind: tool, arguments, expected text, expected `isError`. */
const CALLS = {
  plain: [
    ["add", { a: 2, b: 3 }, "5", false],
    ["echo", { text: "hello" }, "hello", false],
    ["add", { a: 4, b: 5 }, "9", false],
  ],
  hooks: [
    ["add", { a: 2, b: 3 }, "5", false],
    ["echo", { text: "my secret-token is here" }, "my secret-token is here", false],
    ["fail", {}, "the record does not exist", true],
  ],
};

/**
 * The environment of an example run. Throws when `mockUrl` is empty, so that no example starts without
 * an API URL. The temporary `home` keeps the SDK from writing the real ~/agentcat.log.
 */
export function exampleEnv({ mockUrl, home, port }) {
  if (!mockUrl) throw new Error("refusing to start the example: AGENTCAT_API_URL is empty");
  const env = {
    ...process.env,
    HOME: home,
    USERPROFILE: home,
    AGENTCAT_API_URL: mockUrl,
    AGENTCAT_PROJECT_ID: PROJECT_ID,
  };
  // A developer shell's value must not turn the SDK off or point it elsewhere.
  delete env.AGENTCAT_ENABLED;
  delete env.MCPCAT_API_URL;
  if (port !== undefined) env.PORT = String(port);
  else delete env.PORT;
  return env;
}

/** The path of the built example binary. */
export function exampleBinary(name) {
  const target = process.env.CARGO_TARGET_DIR || join(ROOT, "target");
  return join(target, "debug", "examples", `${name}${process.platform === "win32" ? ".exe" : ""}`);
}

/** Runs a command to completion; resolves with its exit code. Its output goes to stderr. */
export function runToEnd(cmd, args) {
  return new Promise((resolve, reject) => {
    const child = spawn(cmd, args, { cwd: ROOT, stdio: ["ignore", process.stderr, process.stderr] });
    child.once("error", reject);
    child.once("close", resolve);
  });
}

/** A free TCP port on 127.0.0.1. */
function freePort() {
  return new Promise((resolve, reject) => {
    const server = createServer();
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const { port } = server.address();
      server.close(() => resolve(port));
    });
  });
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/** Starts the mock API; resolves with its URL and a stop function. */
export async function startMock() {
  const child = spawn(process.execPath, [join(HERE, "mock-api.mjs"), "0"], { stdio: ["ignore", "pipe", "inherit"] });
  const line = await new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("close", () => reject(new Error("the mock API ended before it was ready")));
    createInterface({ input: child.stdout }).once("line", resolve);
  });
  return { url: `http://127.0.0.1:${JSON.parse(line).port}`, stop: () => child.kill("SIGKILL") };
}

/** Waits until `port` accepts a connection, for at most `ms`. */
async function waitForPort(port, ms) {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    const ok = await new Promise((resolve) => {
      const conn = connect(port, "127.0.0.1");
      conn.once("connect", () => {
        conn.destroy();
        resolve(true);
      });
      conn.once("error", () => resolve(false));
    });
    if (ok) return true;
    await sleep(50);
  }
  return false;
}

/**
 * A client transport over the stdio of a child process. The TS client's own stdio transport hides the exit
 * status and the time of the exit, which the check needs. It reads stdout line by line: a line that is not
 * a JSON-RPC message goes to `onBadLine` and not to the client.
 */
export class ChildTransport {
  constructor(child, onBadLine) {
    this.child = child;
    this.onBadLine = onBadLine;
  }

  async start() {
    createInterface({ input: this.child.stdout }).on("line", (line) => {
      if (line.trim() === "") return;
      if (!isJsonRpc(line)) return this.onBadLine(line);
      this.onmessage?.(JSON.parse(line));
    });
    this.child.stdin.on("error", () => {});
  }

  async send(message) {
    this.child.stdin.write(serializeMessage(message));
  }

  async close() {
    this.child.stdin.end();
  }
}

/** Whether `line` is a JSON-RPC 2.0 message. */
function isJsonRpc(line) {
  try {
    const value = JSON.parse(line);
    return value !== null && typeof value === "object" && !Array.isArray(value) && value.jsonrpc === "2.0";
  } catch {
    return false;
  }
}

/**
 * Runs the check for one example. Resolves with `{ failures }`, a list of texts; an empty list means the
 * example passed. `command` replaces the built example (a command and its arguments), for the tests of the
 * check; `exitBudgetMs` replaces the 5 s exit budget.
 */
export async function checkExample(name, { command, exitBudgetMs = EXIT_BUDGET_MS } = {}) {
  const spec = EXAMPLES[name];
  if (!spec) throw new Error(`unknown example ${JSON.stringify(name)}; use one of ${Object.keys(EXAMPLES).join(", ")}`);
  const calls = CALLS[spec.calls];
  const failures = [];
  const fail = (text) => failures.push(text);

  if (!command) {
    const built = await runToEnd("cargo", ["build", "-q", "-p", "agentcat-sdk", "--example", name, "--all-features"]);
    if (built !== 0) throw new Error(`cargo build of the example ${name} failed (exit ${built})`);
    command = [exampleBinary(name)];
  }

  const mock = await startMock();
  const home = mkdtempSync(join(tmpdir(), "examples-check-"));
  let child;
  try {
    const port = spec.http ? await freePort() : undefined;
    const env = exampleEnv({ mockUrl: mock.url, home, port });
    child = spawn(command[0], command.slice(1), { env, stdio: ["pipe", "pipe", "pipe"] });
    const spawnFailure = new Promise((_, reject) => child.once("error", reject));
    const exited = new Promise((resolve) => child.once("close", (code, signal) => resolve({ code, signal, at: Date.now() })));
    let stderrText = "";
    child.stderr.setEncoding("utf8").on("data", (text) => (stderrText += text));

    // Every stdout line must be JSON-RPC. An HTTP example prints no line at all.
    const badLine = (line) => fail(`a stdout line is not JSON-RPC: ${JSON.stringify(line)}`);
    child.stdout.setEncoding("utf8");
    if (spec.http) createInterface({ input: child.stdout }).on("line", (line) => line.trim() === "" || badLine(line));

    // Drive the example with the TS client.
    const client = new Client({ name: "examples-check", version: "1.0.0" }, { capabilities: {} });
    let transport;
    if (spec.http) {
      if (!(await Promise.race([waitForPort(port, 20_000), spawnFailure]))) throw new Error(`the example did not listen on port ${port}`);
      transport = new StreamableHTTPClientTransport(new URL(`http://127.0.0.1:${port}/mcp`));
    } else {
      transport = new ChildTransport(child, badLine);
    }
    await Promise.race([client.connect(transport), spawnFailure]);
    const { tools } = await client.listTools();
    const expectedTools = spec.calls === "hooks" ? ["add", "echo", "fail"] : ["add", "echo"];
    const names = tools.map((tool) => tool.name);
    for (const tool of expectedTools) if (!names.includes(tool)) fail(`tools/list does not list ${tool}: ${JSON.stringify(names)}`);

    // The first call of a server has no session ID. The SDK puts a text block that begins
    // `[session_id issued` and holds the ID it issues before the tool's own result, which it keeps; the
    // later calls pass that ID back. So the tool's text is the first block of a later call, and the
    // block after the issued one on the first call.
    let sessionId;
    for (const [tool, args, text, isError] of calls) {
      const sent = { ...args, context: "Checking the example" };
      if (sessionId) sent.session_id = sessionId;
      const result = await client.callTool({ name: tool, arguments: sent });
      const content = result.content ?? [];
      const first = content[0]?.text;
      const issued = sessionId === undefined ? /session_id: (ses_\w+)/.exec(first ?? "")?.[1] : undefined;
      let rest = content;
      if (issued) {
        sessionId = issued;
        rest = content.slice(1);
      } else if (sessionId === undefined) {
        fail(`the first call (${tool}) did not issue a session ID: ${JSON.stringify(first)}`);
        sessionId = "";
      }
      const texts = rest.map((block) => block.text);
      const got = texts.length === 1 ? texts[0] : texts;
      if (got !== text || Boolean(result.isError) !== isError) {
        fail(`${tool} returned ${JSON.stringify(got)} (isError ${Boolean(result.isError)}), expected ${JSON.stringify(text)} (isError ${isError})`);
      }
    }

    // Close right after the last call; no flush, no pause.
    const closedAt = Date.now();
    await client.close();
    if (spec.http) child.kill("SIGTERM");
    const timer = new Promise((resolve) => setTimeout(() => resolve("late"), exitBudgetMs).unref());
    const result = await Promise.race([exited, timer, spawnFailure]);
    if (result === "late") {
      fail(`the example did not exit within ${exitBudgetMs} ms of the ${spec.http ? "SIGTERM" : "stdin close"}`);
      child.kill("SIGKILL");
      await exited;
    } else if (result.code !== 0) {
      fail(`the example exited with status ${result.code ?? result.signal}`);
    }
    const exitedAt = result === "late" ? Infinity : result.at;
    if (process.env.EXAMPLES_CHECK_VERBOSE) process.stderr.write(`${name}: exited ${exitedAt - closedAt} ms after the close\n`);

    // What the mock received.
    const requests = await (await fetch(`${mock.url}/__requests`)).json();
    const publishes = requests.filter((request) => request.method === "POST" && request.path === "/v1/public/events/");
    const stray = requests.filter((request) => !(request.method === "POST" && request.path === "/v1/public/events/"));
    if (stray.length > 0) fail(`the mock received ${stray.length} request(s) that are not publishes`);
    const typeOf = (request) => request.body?.eventType ?? request.body?.event_type;
    const toolCalls = publishes.filter((request) => typeOf(request) === "mcp:tools/call");
    const gotNames = toolCalls.map((request) => request.body.resourceName ?? request.body.resource_name);
    const wantNames = calls.map(([tool]) => tool);
    if (JSON.stringify([...gotNames].sort()) !== JSON.stringify([...wantNames].sort())) {
      fail(`publishes for the tool calls: got ${JSON.stringify(gotNames)}, expected ${JSON.stringify(wantNames)}`);
    }
    const lastCall = toolCalls.filter((request) => (request.body.resourceName ?? request.body.resource_name) === wantNames.at(-1)).at(-1);
    if (lastCall && lastCall.receivedAt > exitedAt) fail("the publish of the last call arrived after the example exited");
    for (const request of publishes) {
      if ((request.body?.projectId ?? request.body?.project_id) !== PROJECT_ID) fail(`a publish names the project ${JSON.stringify(request.body?.projectId)}, expected ${PROJECT_ID}`);
    }
    if (spec.calls === "hooks") checkHooks(publishes, fail);
    if (failures.length > 0 && stderrText) process.stderr.write(`--- stderr of the example ---\n${stderrText}\n`);
  } finally {
    if (child && child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
    mock.stop();
    rmSync(home, { recursive: true, force: true });
  }
  return { failures };
}

/** The hook values of `hooks.rs`, in the publishes of the run. */
function checkHooks(publishes, fail) {
  const body = (request) => request.body;
  const field = (event, camel, snake) => event[camel] ?? event[snake];
  const toolEvents = publishes.map(body).filter((event) => field(event, "eventType", "event_type") === "mcp:tools/call");
  for (const event of toolEvents) {
    const tool = field(event, "resourceName", "resource_name");
    if (field(event, "identifyActorGivenId", "identify_actor_given_id") !== "demo-user") fail(`${tool}: identify user_id is not "demo-user"`);
    if (field(event, "identifyActorName", "identify_actor_name") !== "Demo User") fail(`${tool}: identify user_name is not "Demo User"`);
    if (event.tags?.example !== "hooks") fail(`${tool}: the tag example is not "hooks"`);
    if (event.properties?.source !== "hooks-example") fail(`${tool}: the property source is not "hooks-example"`);
    if (event.properties?.redacted_by !== "example") fail(`${tool}: the redact_event hook did not run`);
  }
  for (const request of publishes) {
    if (request.rawBody.includes("secret-token")) fail("a publish still holds secret-token");
  }
  if (!publishes.some((request) => request.rawBody.includes("[REDACTED]"))) fail("no publish holds [REDACTED]");
  const failEvent = toolEvents.find((event) => field(event, "resourceName", "resource_name") === "fail");
  const errorType = failEvent?.error?.type ?? failEvent?.error?.name;
  if (!failEvent || !(field(failEvent, "isError", "is_error") === true) || !errorType) fail("the fail call has no error recorded by capture_error");
  const customs = publishes.map(body).filter((event) => field(event, "eventType", "event_type") === "agentcat:custom");
  if (customs.length !== 1) fail(`expected 1 custom event, got ${customs.length}`);
  else if (field(customs[0], "resourceName", "resource_name") !== "echo-called") fail(`the custom event has resource ${JSON.stringify(field(customs[0], "resourceName", "resource_name"))}, expected "echo-called"`);
}

// CLI
if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const name = process.argv[2];
  if (!name || !(name in EXAMPLES)) {
    process.stderr.write(`usage: node harness/src/examples-check.mjs <${Object.keys(EXAMPLES).join(" | ")}>\n`);
    process.exit(2);
  }
  try {
    const { failures } = await checkExample(name);
    if (failures.length > 0) {
      for (const text of failures) process.stderr.write(`FAIL ${name}: ${text}\n`);
      process.exit(1);
    }
    process.stderr.write(`ok ${name}\n`);
  } catch (error) {
    process.stderr.write(`examples-check: ${error.message}\n`);
    process.exit(1);
  }
}

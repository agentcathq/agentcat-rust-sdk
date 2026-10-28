// The dashboard demo (plan 5, Task 17, decision C58): one fixed scenario (demo/scenario.json) through the
// `hooks` example (Rust) and through demo/ts-server.mjs (the same tools and options on the TS SDK at the pinned
// commit), over stdio with the TS client.
//
// usage: node harness/demo/run.mjs --mock
//        node harness/demo/run.mjs --prod --project proj_2yAR5QHJ3CE3icTpJlKJqu1ZiXG   (maintainers only)
//
//   --mock  Both servers publish to the local mock API and run as the harness runs them: the TS server under the
//           egress guard. The run fails (exit 1) unless each server publishes, with the same event count and the
//           same event types. Prints one JSON summary line on stdout. This is part of the gate.
//   --prod  Both servers publish to the default AgentCat API, for the project "Testing" only. No egress guard
//           and no API URL. The run is refused (exit 2) for any other project, and when CI is set. It writes the
//           session IDs of both servers and the run time to harness/out/demo/sessions.json.
// Every refusal happens in parseArgs, before the first line of output, any server process or any network call.
import { spawn } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { Client } from "@modelcontextprotocol/sdk/client/index.js";

import { compareEvents, compareTools } from "./compare.mjs";
import { ChildTransport, PROJECT_ID as MOCK_PROJECT_ID, ROOT, exampleBinary, exampleEnv, runToEnd, startMock } from "../src/examples-check.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const GUARD = join(HERE, "..", "src", "egress-guard.mjs");
const OUT_FILE = join(HERE, "..", "out", "demo", "sessions.json");

/** The only project that --prod accepts: "Testing" (ruling R12). */
export const PROD_PROJECT = "proj_2yAR5QHJ3CE3icTpJlKJqu1ZiXG";
/** How long a server may take to exit after the stdin close. */
const EXIT_BUDGET_MS = 10_000;

export class UsageError extends Error {}

/**
 * Reads the command line and the environment. Returns `{ mode, project }` or throws a UsageError. It has no
 * side effect, so a refusal cannot start a process or reach the network.
 */
export function parseArgs(argv, env) {
  const usage = "usage: run.mjs --mock | --prod --project <id>";
  const flags = argv.filter((arg) => arg === "--mock" || arg === "--prod");
  if (flags.length !== 1) throw new UsageError(usage);
  const mode = flags[0] === "--mock" ? "mock" : "prod";
  const rest = argv.filter((arg) => arg !== flags[0]);
  if (mode === "mock") {
    if (rest.length > 0) throw new UsageError(usage);
    return { mode, project: MOCK_PROJECT_ID };
  }
  if (rest.length !== 2 || rest[0] !== "--project") throw new UsageError(`--prod needs --project <id>; ${usage}`);
  const project = rest[1];
  if (project !== PROD_PROJECT) throw new UsageError(`refusing --prod: only the project "Testing" is allowed (got ${JSON.stringify(project)})`);
  if (env.CI !== undefined && env.CI !== "") throw new UsageError("refusing --prod: CI is set; only a maintainer runs --prod");
  return { mode, project };
}

/** The environment of a demo server: no AgentCat setting from the developer's shell, a temporary HOME. */
function serverEnv({ mockUrl, project, home }) {
  if (mockUrl !== undefined) return { ...exampleEnv({ mockUrl, home }), AGENTCAT_PROJECT_ID: project, DISABLE_DIAGNOSTICS: "true" };
  const env = { ...process.env, HOME: home, USERPROFILE: home, AGENTCAT_PROJECT_ID: project, DISABLE_DIAGNOSTICS: "true" };
  for (const name of ["AGENTCAT_API_URL", "MCPCAT_API_URL", "AGENTCAT_ENABLED"]) delete env[name];
  return env;
}

/** The text blocks of a tool result. */
const blocks = (result) => (result.content ?? []).filter((b) => b.type === "text").map((b) => b.text);

/**
 * Runs the scenario against one server process. Resolves with `{ failures, sessions }`: failures is a list of
 * texts, sessions the `ses_` IDs the SDK issued, in order.
 */
async function drive({ label, command, env, scenario }) {
  const failures = [];
  const sessions = [];
  let tools = [];
  const fail = (text) => failures.push(`${label}: ${text}`);
  const child = spawn(command[0], command.slice(1), { env, stdio: ["pipe", "pipe", "pipe"] });
  const spawnFailure = new Promise((_, reject) => child.once("error", reject));
  const exited = new Promise((resolve) => child.once("close", (code, signal) => resolve({ code, signal })));
  let stderrText = "";
  child.stderr.setEncoding("utf8").on("data", (text) => (stderrText += text));
  try {
    const client = new Client({ name: "agentcat-demo", version: "1.0.0" }, { capabilities: {} });
    await Promise.race([client.connect(new ChildTransport(child, (line) => fail(`a stdout line is not JSON-RPC: ${JSON.stringify(line)}`))), spawnFailure]);
    for (const session of scenario.sessions) {
      let sessionId; // each session starts without an ID; the first answer issues one
      for (const step of session.steps) {
        if (step.list) {
          tools = (await client.listTools()).tools;
          const names = tools.map((tool) => tool.name).sort();
          if (!["add", "echo", "fail"].every((name) => names.includes(name))) fail(`tools/list names ${JSON.stringify(names)}`);
          continue;
        }
        const args = { ...step.arguments, context: step.context };
        if (sessionId) args.session_id = sessionId;
        const result = await client.callTool({ name: step.tool, arguments: args });
        const texts = blocks(result);
        const issued = sessionId === undefined ? /session_id: (ses_\w+)/.exec(texts.join("\n"))?.[1] : undefined;
        if (issued) {
          sessionId = issued;
          sessions.push(issued);
        } else if (sessionId === undefined) {
          fail(`${step.tool} did not issue a session ID`);
        }
        if (!texts.some((t) => t.includes(step.expect.text)) || Boolean(result.isError) !== step.expect.isError) {
          fail(`${step.tool} returned ${JSON.stringify(texts)} (isError ${Boolean(result.isError)}), expected ${JSON.stringify(step.expect.text)} (isError ${step.expect.isError})`);
        }
      }
    }
    // Close right after the last call; the server drains its queue and exits.
    await client.close();
    const timer = new Promise((resolve) => setTimeout(() => resolve("late"), EXIT_BUDGET_MS).unref());
    const result = await Promise.race([exited, timer, spawnFailure]);
    if (result === "late") fail(`did not exit within ${EXIT_BUDGET_MS} ms of the stdin close`);
    else if (result.code !== 0) fail(`exited with status ${result.code ?? result.signal}`);
  } catch (error) {
    fail(error.message);
  } finally {
    if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
    if (failures.length > 0 && stderrText) process.stderr.write(`--- stderr of ${label} ---\n${stderrText}\n`);
  }
  return { failures, sessions, tools };
}

/** The publishes the mock holds, and how many other requests it saw. */
async function readPublishes(mockUrl) {
  const requests = await (await fetch(`${mockUrl}/__requests`)).json();
  const publishes = requests.filter((r) => r.method === "POST" && r.path === "/v1/public/events/");
  return { requests: publishes, stray: requests.length - publishes.length };
}

async function main() {
  let args;
  try {
    args = parseArgs(process.argv.slice(2), process.env);
  } catch (error) {
    if (!(error instanceof UsageError)) throw error;
    process.stderr.write(`demo: ${error.message}\n`);
    return 2;
  }
  const { mode, project } = args;
  process.stderr.write(`demo: starting the ${mode} run\n`);
  const scenario = JSON.parse(readFileSync(join(HERE, "scenario.json"), "utf8"));

  const built = await runToEnd("cargo", ["build", "-q", "-p", "agentcat-sdk", "--example", "hooks", "--all-features"]);
  if (built !== 0) throw new Error(`cargo build of the example hooks failed (exit ${built})`);

  const mock = mode === "mock" ? await startMock() : undefined;
  const home = mkdtempSync(join(tmpdir(), "demo-"));
  const env = serverEnv({ mockUrl: mock?.url, project, home });
  const guard = mode === "mock" ? ["--import", pathToFileURL(GUARD).href] : [];
  const servers = [
    { label: "rust", command: [exampleBinary("hooks")] },
    { label: "ts", command: [process.execPath, ...guard, join(HERE, "ts-server.mjs")] },
  ];
  const results = {};
  const failures = [];
  try {
    for (const server of servers) {
      if (mock) await fetch(`${mock.url}/__reset`, { method: "POST" });
      const { failures: found, sessions, tools } = await drive({ ...server, env, scenario });
      failures.push(...found);
      results[server.label] = { sessions, tools };
      if (mock) {
        Object.assign(results[server.label], await readPublishes(mock.url));
        if (results[server.label].requests.length === 0) failures.push(`${server.label}: the mock received no publish`);
        if (results[server.label].stray > 0) failures.push(`${server.label}: the mock received ${results[server.label].stray} request(s) that are not publishes`);
      }
    }
  } finally {
    mock?.stop();
    rmSync(home, { recursive: true, force: true });
  }

  if (mode === "mock") {
    const { rust, ts } = results;
    const secretSteps = scenario.sessions.flatMap((x) => x.steps).filter((step) => JSON.stringify(step.arguments ?? {}).includes("secret-token")).length;
    failures.push(...compareTools(rust.tools, ts.tools));
    const compared = compareEvents({ rust, ts, secretSteps });
    failures.push(...compared.failures);
    const summarize = (r) => {
      const types = {};
      for (const request of r.requests) {
        const type = request.body?.eventType ?? request.body?.event_type ?? "unknown";
        types[type] = (types[type] ?? 0) + 1;
      }
      return { count: r.requests.length, types: Object.fromEntries(Object.entries(types).sort()) };
    };
    process.stdout.write(`${JSON.stringify({ rust: summarize(rust), ts: summarize(ts), compared: compared.compared, tools: rust.tools.length })}\n`);
  } else if (failures.length === 0) {
    mkdirSync(dirname(OUT_FILE), { recursive: true });
    const record = { runAt: new Date().toISOString(), project, rust: results.rust.sessions, ts: results.ts.sessions };
    writeFileSync(OUT_FILE, `${JSON.stringify(record, null, 2)}\n`);
    process.stderr.write(`demo: session IDs of rust ${results.rust.sessions.join(", ")} and ts ${results.ts.sessions.join(", ")}\n`);
  }
  for (const text of failures) process.stderr.write(`FAIL ${text}\n`);
  return failures.length > 0 ? 1 : 0;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  try {
    process.exitCode = await main();
  } catch (error) {
    process.stderr.write(`demo: ${error.message}\n`);
    process.exitCode = 1;
  }
}

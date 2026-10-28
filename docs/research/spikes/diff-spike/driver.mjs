// Language-neutral scenario driver. ONE client implementation (the official TS
// MCP client) drives ANY reference server named by a command line, over stdio
// or streamable HTTP, following a JSON scenario file. The server under test is
// a black box: a TS server and a Rust server are run by the same driver.
//
// usage: node driver.mjs <scenario.json> <label> <transport> -- <server cmd...>
//   label      e.g. "ts" or "rust" (output file prefix)
//   transport  "stdio" | "http"
// outputs: out/<scenario>.<label>.<transport>.{raw,transcript,normalized}.json
import { spawn } from "node:child_process";
import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { createInterface } from "node:readline";
import path from "node:path";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";
import { normalizeRun } from "./normalize.mjs";

const here = path.dirname(new URL(import.meta.url).pathname);
const [scenarioPath, label, transport, sep, ...serverCmd] = process.argv.slice(2);
if (sep !== "--" || serverCmd.length === 0) {
  console.error("usage: node driver.mjs <scenario.json> <label> <stdio|http> -- <server cmd...>");
  process.exit(2);
}
const scenario = JSON.parse(readFileSync(scenarioPath, "utf8"));
const outDir = path.join(here, "out");
mkdirSync(outDir, { recursive: true });

function readyLine(child) {
  return new Promise((resolve, reject) => {
    const rl = createInterface({ input: child.stdout });
    rl.once("line", (line) => {
      try {
        resolve(JSON.parse(line));
      } catch (e) {
        reject(new Error(`bad ready line: ${line}`));
      }
    });
    child.once("exit", (code) => reject(new Error(`process exited early (${code})`)));
  });
}

const mock = spawn(process.execPath, [path.join(here, "mock-api.mjs"), "0"], { stdio: ["ignore", "pipe", "inherit"] });
const { port: mockPort } = await readyLine(mock);
const mockUrl = `http://127.0.0.1:${mockPort}`;
const getRequests = async () => (await fetch(`${mockUrl}/__requests`)).json();

const serverEnv = {
  ...process.env,
  HOME: path.join(here, "home"), // keep ~/agentcat.log out of the real home dir
  AGENTCAT_API_URL: mockUrl,
  DISABLE_DIAGNOSTICS: "true",
  SPIKE_PROJECT_ID: scenario.projectId,
  SPIKE_OPTIONS: JSON.stringify(scenario.options ?? {}),
};

let httpChild;
let httpUrl;
if (transport === "http") {
  httpChild = spawn(serverCmd[0], [...serverCmd.slice(1), "http", "0"], { env: serverEnv, stdio: ["ignore", "pipe", "inherit"] });
  ({ url: httpUrl } = await readyLine(httpChild));
}

async function connect(clientInfo) {
  const client = new Client(clientInfo, { capabilities: {} });
  const t =
    transport === "stdio"
      ? new StdioClientTransport({ command: serverCmd[0], args: [...serverCmd.slice(1), "stdio"], env: serverEnv, stderr: "inherit" })
      : new StreamableHTTPClientTransport(new URL(httpUrl), {
          requestInit: { headers: scenario.httpHeaders ?? {} },
        });
  await client.connect(t);
  return client;
}

const vars = {};
const subst = (v) =>
  typeof v === "string" ? v.replace(/\$\{(\w+)\}/g, (_, k) => vars[k] ?? `\${${k}}`)
  : Array.isArray(v) ? v.map(subst)
  : v && typeof v === "object" ? Object.fromEntries(Object.entries(v).map(([k, x]) => [k, subst(x)]))
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
  const client = await connect(session.client ?? { name: "diff-client", version: "1.0.0" });
  clients.push(client);
  for (const step of session.steps) {
    const entry = { session: si, op: step.op };
    try {
      if (step.op === "listTools") {
        const res = await client.listTools(step.cursor ? { cursor: step.cursor } : undefined);
        entry.result = res;
      } else if (step.op === "callTool") {
        entry.request = { name: step.name, arguments: subst(step.args ?? {}) };
        const res = await client.callTool(entry.request);
        entry.result = res;
        if (step.saveHandleAs) vars[step.saveHandleAs] = handleFrom(res);
      } else {
        throw new Error(`unknown op ${step.op}`);
      }
    } catch (e) {
      // Protocol errors (e.g. JSON-RPC errors) are part of the observable contract.
      entry.error = { name: e?.constructor?.name, code: e?.code, message: String(e?.message ?? e) };
    }
    transcript.push(entry);
  }
}

// Wait for the event stream to settle BEFORE closing clients: expected count
// reached and no new publish for `quietMs`.
const expected = scenario.expectPublishes ?? 0;
const deadline = Date.now() + (scenario.settleTimeoutMs ?? 10_000);
let last = -1;
let stableSince = Date.now();
for (;;) {
  const n = (await getRequests()).length;
  if (n !== last) {
    last = n;
    stableSince = Date.now();
  }
  if (n >= expected && Date.now() - stableSince >= (scenario.quietMs ?? 500)) break;
  if (Date.now() > deadline) break;
  await new Promise((r) => setTimeout(r, 50));
}
const requests = await getRequests();

for (const c of clients) await c.close().catch(() => {});
httpChild?.kill("SIGTERM");
mock.kill("SIGTERM");

const base = path.join(outDir, `${scenario.name}.${label}.${transport}`);
writeFileSync(`${base}.raw.json`, JSON.stringify(requests, null, 2));
writeFileSync(`${base}.transcript.json`, JSON.stringify(transcript, null, 2));
const { normalized, report } = normalizeRun({ requests, transcript });
writeFileSync(`${base}.normalized.json`, JSON.stringify(normalized, null, 2));
const summary = { scenario: scenario.name, label, transport, publishes: requests.filter((r) => r.path === "/v1/public/events/").length, expected, report };
writeFileSync(`${base}.report.json`, JSON.stringify(summary, null, 2));
console.log(JSON.stringify(summary));
process.exit(summary.publishes === expected ? 0 : 1);

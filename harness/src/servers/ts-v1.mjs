// TS v1 reference server: an MCP server on @modelcontextprotocol/sdk 1.x,
// instrumented with the real TypeScript SDK built from the pinned commit.
// The Rust reference server (plan 3) follows the same protocol and exposes the
// same tool contract (harness/contract/tool-contract.json).
//
// usage: node --import ./harness/src/egress-guard.mjs harness/src/servers/ts-v1.mjs stdio
//        ... ts-v1.mjs http <port>             stateful streamable HTTP, one tracked server per MCP session
//        ... ts-v1.mjs http-stateless <port>   no session ID, one tracked server per request
// HTTP modes print one ready line on stdout: {"ready": true, "url": "http://127.0.0.1:<port>/mcp"}.
// env:   AGENTCAT_API_URL    mock API base URL (required; http://127.0.0.1, localhost or [::1] only)
//        HARNESS_PROJECT_ID  project ID (required unless HARNESS_NO_TRACK=1)
//        HARNESS_OPTIONS     JSON options profile (harness/README.md)
//        HARNESS_NO_TRACK=1  serve untracked
//        HOME                a harness temp dir (the TS SDK writes ~/agentcat.log)
// Exit 2: bad arguments, a non-local AGENTCAT_API_URL or an invalid profile.
import "../egress-guard.mjs"; // first import: installed before any other module runs

import http from "node:http";
import { randomUUID } from "node:crypto";
import { pathToFileURL } from "node:url";

import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { StreamableHTTPServerTransport } from "@modelcontextprotocol/sdk/server/streamableHttp.js";
import { ensureTs } from "oracle/fetch-ts";
import { tsDist } from "oracle/paths";
import { z } from "zod";

function fail(message) {
  process.stderr.write(`ts-v1: ${message}\n`);
  process.exit(2);
}

const [mode = "stdio", portArg] = process.argv.slice(2);
if (!["stdio", "http", "http-stateless"].includes(mode) || (mode === "stdio" && portArg !== undefined)) {
  fail("usage: ts-v1.mjs stdio | http <port> | http-stateless <port>");
}
const port = Number(portArg ?? 0);
if (mode !== "stdio" && !(Number.isInteger(port) && port >= 0 && port <= 65535)) fail(`bad port ${portArg}`);

const apiUrl = process.env.AGENTCAT_API_URL ?? "";
if (!isLocalHttpUrl(apiUrl)) {
  process.stderr.write(`refusing to start: AGENTCAT_API_URL must point at the local mock (got ${JSON.stringify(apiUrl)})\n`);
  process.exit(2);
}

const tracked = process.env.HARNESS_NO_TRACK !== "1";
const projectId = process.env.HARNESS_PROJECT_ID ?? "";
if (tracked && projectId === "") fail("HARNESS_PROJECT_ID is required");

function isLocalHttpUrl(text) {
  let url;
  try {
    url = new URL(text);
  } catch {
    return false;
  }
  return url.protocol === "http:" && ["127.0.0.1", "localhost", "[::1]"].includes(url.hostname);
}

// ---------------------------------------------------------------------------
// Options profile -> AgentCat options. Every key needs a Rust twin.
// ---------------------------------------------------------------------------
const FLAGS = ["enableTracing", "enableReportMissing", "enableToolCallContext", "enableAgentTracking", "customContextDescription"];
// Hooks whose src/types.ts signature allows a synchronous return.
const SYNC_HOOKS = ["resolveSessionId", "eventTags", "eventProperties"];
// Hooks whose signature returns a Promise.
const ASYNC_HOOKS = ["identify", "redactSensitiveInformation"];

function buildOptions(profile) {
  if (!profile || typeof profile !== "object" || Array.isArray(profile)) throw new Error("must be a JSON object");
  const o = { disableDiagnostics: true };
  for (const [key, value] of Object.entries(profile)) {
    if (FLAGS.includes(key)) o[key] = value; // copied as given
    else if (SYNC_HOOKS.includes(key) || ASYNC_HOOKS.includes(key)) o[key] = hook(key, value);
    else throw new Error(`unknown key ${JSON.stringify(key)}`);
  }
  return o;
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/**
 * A canned behavior -> a function with the hook's signature:
 *   {"returns": <value or null>} | {"throws": "<message>"}, either plus "sleepMs": <n>;
 *   redactSensitiveInformation also takes {"replace": "<secret>"} (each occurrence -> "[REDACTED]").
 * A synchronous-capable hook answers synchronously unless sleepMs is given; identify
 * and redactSensitiveInformation always return a Promise. Each call returns a fresh copy.
 */
function hook(name, spec) {
  if (!spec || typeof spec !== "object" || Array.isArray(spec)) throw new Error(`${name}: a behavior must be an object`);
  const kinds = name === "redactSensitiveInformation" ? ["returns", "throws", "replace"] : ["returns", "throws"];
  for (const key of Object.keys(spec)) {
    if (key !== "sleepMs" && !kinds.includes(key)) throw new Error(`${name}: unknown key ${JSON.stringify(key)}`);
  }
  const given = kinds.filter((k) => k in spec);
  if (given.length !== 1) throw new Error(`${name}: needs exactly one of ${kinds.join(", ")}`);
  const kind = given[0];
  if (kind === "throws" && typeof spec.throws !== "string") throw new Error(`${name}: "throws" must be a string`);
  if (kind === "replace" && (typeof spec.replace !== "string" || spec.replace === "")) {
    throw new Error(`${name}: "replace" must be a non-empty string`);
  }
  if ("sleepMs" in spec && !(Number.isInteger(spec.sleepMs) && spec.sleepMs >= 0)) {
    throw new Error(`${name}: "sleepMs" must be a non-negative integer`);
  }
  const answer = (text) => {
    if (kind === "throws") throw new Error(spec.throws);
    if (kind === "replace") return text.split(spec.replace).join("[REDACTED]");
    return structuredClone(spec.returns);
  };
  if (name === "redactSensitiveInformation") {
    return async (text) => {
      if (spec.sleepMs !== undefined) await sleep(spec.sleepMs);
      return answer(text);
    };
  }
  if (ASYNC_HOOKS.includes(name) || spec.sleepMs !== undefined) {
    return async (_request, _extra) => {
      if (spec.sleepMs !== undefined) await sleep(spec.sleepMs);
      return answer();
    };
  }
  return (_request, _extra) => answer();
}

let options;
try {
  options = buildOptions(JSON.parse(process.env.HARNESS_OPTIONS ?? "{}"));
} catch (err) {
  fail(`HARNESS_OPTIONS: ${err.message}`);
}

// ---------------------------------------------------------------------------
// The tool contract shared with the Rust reference server.
// ---------------------------------------------------------------------------
await ensureTs();
const agentcat = await import(pathToFileURL(tsDist()).href);

function buildServer() {
  const server = new McpServer({ name: "harness-ref-server", version: "1.0.0" });
  server.registerTool(
    "echo",
    { description: "Echo a message back", inputSchema: { msg: z.string().describe("Message to echo") } },
    async ({ msg }) => ({ content: [{ type: "text", text: msg }] }),
  );
  server.registerTool(
    "structured",
    {
      description: "Returns structuredContent per its outputSchema",
      inputSchema: { msg: z.string() },
      outputSchema: { echoed: z.string().describe("The echoed message") },
    },
    async ({ msg }) => ({ content: [{ type: "text", text: msg }], structuredContent: { echoed: msg } }),
  );
  server.registerTool("boom", { description: "Always throws", inputSchema: {} }, async () => {
    throw new Error("boom");
  });
  server.registerTool(
    "tool_error",
    { description: "Returns an isError result", inputSchema: {} },
    async () => ({ isError: true, content: [{ type: "text", text: "handled failure" }] }),
  );
  server.registerTool(
    "big",
    { description: "Returns a large text payload", inputSchema: { size: z.number().int() } },
    async ({ size }) => ({ content: [{ type: "text", text: "x".repeat(size) }] }),
  );
  server.registerTool(
    "emit_custom",
    {
      description: "Publishes an AgentCat custom event",
      inputSchema: { session: z.string(), resource: z.string(), message: z.string().optional() },
    },
    async ({ session, resource, message }) => {
      // Untracked: track() never configured the SDK's API URL, so a publish
      // would aim at the production default. Answer the same without one.
      if (!tracked) return { content: [{ type: "text", text: "published" }] };
      await agentcat.publishCustomEvent(session, projectId, {
        resourceName: resource,
        message,
        parameters: { from: "emit_custom" },
        tags: { kind: "custom" },
      });
      return { content: [{ type: "text", text: "published" }] };
    },
  );
  if (tracked) agentcat.track(server, projectId, options);
  return server;
}

// ---------------------------------------------------------------------------
// Transports.
// ---------------------------------------------------------------------------
function sendError(res, err) {
  process.stderr.write(`ts-v1: http error: ${err?.stack ?? err}\n`);
  if (!res.headersSent) res.writeHead(500);
  res.end();
}

if (mode === "stdio") {
  await buildServer().connect(new StdioServerTransport());
} else {
  // Stateful: one tracked McpServer per MCP session (SDK 1.31 forbids reusing a
  // connected server). Stateless: a fresh tracked server and transport per POST,
  // closed with the response, as in the TS e2e v1-stateless lane; GET and DELETE
  // answer 405.
  const sessions = new Map();
  const handle =
    mode === "http"
      ? async (req, res) => {
          const sid = req.headers["mcp-session-id"];
          let transport = typeof sid === "string" ? sessions.get(sid) : undefined;
          if (!transport) {
            const created = new StreamableHTTPServerTransport({
              sessionIdGenerator: () => randomUUID(),
              enableJsonResponse: true,
              onsessioninitialized: (id) => sessions.set(id, created),
            });
            created.onclose = () => {
              if (created.sessionId) sessions.delete(created.sessionId);
            };
            await buildServer().connect(created);
            transport = created;
          }
          await transport.handleRequest(req, res);
        }
      : async (req, res) => {
          if (req.method !== "POST") {
            res.writeHead(405, { "content-type": "application/json", allow: "POST" });
            res.end(JSON.stringify({ jsonrpc: "2.0", error: { code: -32000, message: "Method not allowed." }, id: null }));
            return;
          }
          const server = buildServer();
          const transport = new StreamableHTTPServerTransport({ sessionIdGenerator: undefined, enableJsonResponse: true });
          res.on("close", () => {
            void transport.close();
            void server.close();
          });
          await server.connect(transport);
          await transport.handleRequest(req, res);
        };
  const httpServer = http.createServer((req, res) => {
    handle(req, res).catch((err) => sendError(res, err));
  });
  httpServer.listen(port, "127.0.0.1", () => {
    process.stdout.write(`${JSON.stringify({ ready: true, url: `http://127.0.0.1:${httpServer.address().port}/mcp` })}\n`);
  });
}

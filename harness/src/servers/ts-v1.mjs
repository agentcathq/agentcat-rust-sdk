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

import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { StreamableHTTPServerTransport } from "@modelcontextprotocol/sdk/server/streamableHttp.js";
import { z } from "zod";

import { SERVER_INFO, listen, loadAgentCat, methodNotAllowed, sendError, startup, toolHandlers } from "./common.mjs";

const NAME = "ts-v1";
const { mode, port, tracked, projectId, options } = startup(NAME);

// ---------------------------------------------------------------------------
// The tools of the other reference servers (R20: ts-v2 is the answer key; the
// contract file in contract/tool-contract.json is ts-v2's tools/list). These zod
// schemas describe the same tools, minus the `execution` field v1 adds; a test
// (contract.test.mjs) keeps them in step with the contract.
// ---------------------------------------------------------------------------
const agentcat = await loadAgentCat();

function buildServer() {
  // Fresh handler functions per server: the TS SDK marks each function it
  // wraps, so a function shared by two servers is wrapped only on the first.
  const handlers = toolHandlers({ tracked, projectId, agentcat });
  const server = new McpServer(SERVER_INFO);
  server.registerTool(
    "echo",
    { description: "Echo a message back", inputSchema: { msg: z.string().describe("Message to echo") } },
    handlers.echo,
  );
  server.registerTool(
    "structured",
    {
      description: "Returns structuredContent per its outputSchema",
      inputSchema: { msg: z.string() },
      outputSchema: { echoed: z.string().describe("The echoed message") },
    },
    handlers.structured,
  );
  server.registerTool("boom", { description: "Always throws", inputSchema: {} }, handlers.boom);
  server.registerTool("tool_error", { description: "Returns an isError result", inputSchema: {} }, handlers.tool_error);
  server.registerTool(
    "big",
    { description: "Returns a large text payload", inputSchema: { size: z.number().int() } },
    handlers.big,
  );
  server.registerTool(
    "emit_custom",
    {
      description: "Publishes an AgentCat custom event",
      inputSchema: { session: z.string(), resource: z.string(), message: z.string().optional() },
    },
    handlers.emit_custom,
  );
  server.registerTool(
    "own_session",
    {
      description: "Declares its own session_id and returns the value it received",
      inputSchema: { session_id: z.string(), x: z.string().optional() },
    },
    handlers.own_session,
  );
  server.registerTool(
    "own_context",
    { description: "Declares its own context and returns it", inputSchema: { context: z.string() } },
    handlers.own_context,
  );
  server.registerTool(
    "structured_own",
    {
      description: "Returns structuredContent; its outputSchema declares mcp_session",
      inputSchema: { msg: z.string() },
      outputSchema: {
        echoed: z.string(),
        mcp_session: z.object({ session_id: z.string() }).passthrough().optional(),
      },
    },
    handlers.structured_own,
  );
  if (tracked) agentcat.track(server, projectId, options);
  return server;
}

// ---------------------------------------------------------------------------
// Transports.
// ---------------------------------------------------------------------------
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
            methodNotAllowed(res);
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
    handle(req, res).catch((err) => sendError(NAME, res, err));
  });
  listen(httpServer, port);
}

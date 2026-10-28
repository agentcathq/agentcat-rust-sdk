// TS v2 reference server: an MCP server on @modelcontextprotocol/server 2.x,
// instrumented with the real TypeScript SDK built from the pinned commit. It
// serves protocol 2026-07-28 (no initialize handshake; design decision C18)
// and the 2025 era, with the tools, identity and behaviors of ts-v1.mjs. The
// tool names, descriptions and schemas come from TOOLS in common.mjs; dump-contract.mjs
// writes this server's tools/list to harness/contract/tool-contract.json (R20), which the
// Rust reference server embeds.
//
// usage: node --import ./harness/src/egress-guard.mjs harness/src/servers/ts-v2.mjs stdio
//                                              both eras (serveStdio), one tracked server per connection
//        ... ts-v2.mjs http <port>             2026-07-28: one tracked server per request (createMcpHandler);
//                                              2025 era: stateful, one tracked server per MCP session
//        ... ts-v2.mjs http-sse <port>         as http, with SSE responses (enableJsonResponse false; createMcpHandler responseMode "sse")
//        ... ts-v2.mjs http-stateless <port>   2026-07-28 as above; 2025 era: one tracked server per request
// HTTP modes print one ready line on stdout: {"ready": true, "url": "http://127.0.0.1:<port>/mcp"}.
// env:   as ts-v1.mjs (AGENTCAT_API_URL, HARNESS_PROJECT_ID, HARNESS_OPTIONS, HARNESS_NO_TRACK, HOME)
// Exit 2: bad arguments, a non-local AGENTCAT_API_URL or an invalid profile.
import "../egress-guard.mjs"; // first import: installed before any other module runs

import http from "node:http";
import { randomUUID } from "node:crypto";

import { toNodeHandler } from "@modelcontextprotocol/node";
import {
  McpServer,
  ProtocolError,
  ProtocolErrorCode,
  WebStandardStreamableHTTPServerTransport,
  createMcpHandler,
  fromJsonSchema,
  isLegacyRequest,
} from "@modelcontextprotocol/server";
import { serveStdio } from "@modelcontextprotocol/server/stdio";
import { AjvJsonSchemaValidator } from "@modelcontextprotocol/server/validators/ajv";

import { SERVER_INFO, TOOLS, listen, loadAgentCat, methodNotAllowed, pageToolsList, sendError, startup, toolHandlers } from "./common.mjs";

const NAME = "ts-v2";
const { mode, port, tracked, projectId, options, pageSize } = startup(NAME);

// ---------------------------------------------------------------------------
// The tools: names, descriptions and JSON schemas from TOOLS, behaviors from
// toolHandlers (both in common.mjs).
// ---------------------------------------------------------------------------
const agentcat = await loadAgentCat();

// Arguments are validated as ts-v1's zod objects validate them: a key the
// schema does not declare is dropped, not rejected, although the listed schema
// says additionalProperties: false. (Untracked, nothing strips session_id or
// context, and plain ajv would reject them.)
const ajv = new AjvJsonSchemaValidator();
const zodLikeArguments = {
  getValidator(schema) {
    const check = ajv.getValidator(schema);
    const declared = new Set(Object.keys(schema.properties ?? {}));
    return (args) =>
      check(
        args !== null && typeof args === "object" && !Array.isArray(args)
          ? Object.fromEntries(Object.entries(args).filter(([key]) => declared.has(key)))
          : args,
      );
  },
};

function buildServer() {
  // Fresh handler functions per server: the TS SDK marks each function it
  // wraps, so a function shared by two servers is wrapped only on the first.
  const handlers = toolHandlers({ tracked, projectId, agentcat, server: () => server });
  const server = new McpServer(SERVER_INFO);
  for (const tool of TOOLS) {
    server.registerTool(
      tool.name,
      {
        description: tool.description,
        inputSchema: fromJsonSchema(tool.inputSchema, zodLikeArguments),
        ...(tool.outputSchema && { outputSchema: fromJsonSchema(tool.outputSchema) }),
      },
      handlers[tool.name],
    );
  }
  if (pageSize !== undefined) pageToolsList(server.server, pageSize, (message) => new ProtocolError(ProtocolErrorCode.InvalidParams, message));
  if (tracked) agentcat.track(server, projectId, options);
  return server;
}

// ---------------------------------------------------------------------------
// Transports.
// ---------------------------------------------------------------------------
if (mode === "stdio") {
  // serveStdio picks the era from the opening message and pins one tracked
  // server for the connection. A bare StdioServerTransport serves only the
  // 2025 era (it answers server/discover with "Method not found").
  serveStdio(buildServer, { onerror: (err) => process.stderr.write(`${NAME}: ${err?.message ?? err}\n`) });
} else {
  // 2026-07-28 requests (they carry the per-request _meta envelope) go to
  // createMcpHandler, which builds and tracks a fresh server per request, as
  // in the TS e2e v2-handler lanes. 2025-era requests (isLegacyRequest) get
  // the topology of ts-v1: stateful, one tracked server per MCP session
  // (initialize creates it), or a fresh tracked server and transport per POST.
  // Every exchange answers in JSON response mode.
  const modern = createMcpHandler(buildServer, {
    legacy: "reject",
    responseMode: mode === "http-sse" ? "sse" : "json",
    onerror: (err) => process.stderr.write(`${NAME}: ${err?.message ?? err}\n`),
  });
  const sessions = new Map();
  const legacy =
    mode === "http" || mode === "http-sse"
      ? async (request) => {
          const sid = request.headers.get("mcp-session-id");
          let transport = sid === null ? undefined : sessions.get(sid);
          if (!transport) {
            const created = new WebStandardStreamableHTTPServerTransport({
              sessionIdGenerator: () => randomUUID(),
              enableJsonResponse: mode === "http",
              onsessioninitialized: (id) => sessions.set(id, created),
            });
            created.onclose = () => {
              if (created.sessionId) sessions.delete(created.sessionId);
            };
            await buildServer().connect(created);
            transport = created;
          }
          return transport.handleRequest(request);
        }
      : async (request) => {
          const server = buildServer();
          const transport = new WebStandardStreamableHTTPServerTransport({
            sessionIdGenerator: undefined,
            enableJsonResponse: true,
          });
          await server.connect(transport);
          try {
            // JSON response mode: the response is complete when it resolves.
            return await transport.handleRequest(request);
          } finally {
            void transport.close();
            void server.close();
          }
        };
  const handle = toNodeHandler(
    { fetch: async (request) => ((await isLegacyRequest(request)) ? legacy(request) : modern.fetch(request)) },
    { onerror: (err) => process.stderr.write(`${NAME}: http error: ${err?.stack ?? err}\n`) },
  );
  const httpServer = http.createServer((req, res) => {
    if (mode === "http-stateless" && req.method !== "POST") {
      methodNotAllowed(res);
      return;
    }
    handle(req, res).catch((err) => sendError(NAME, res, err));
  });
  listen(httpServer, port);
}

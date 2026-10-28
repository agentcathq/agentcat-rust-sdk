// Reference MCP server instrumented with the REAL TypeScript SDK (built from a
// pinned agentcat-typescript-sdk commit). The Rust reference server must expose
// the same tool contract and interpret the same JSON "options profile".
//
// usage: node --import ./egress-guard.mjs ts-server.mjs stdio
//        node --import ./egress-guard.mjs ts-server.mjs http <port>
// env:   AGENTCAT_API_URL      mock API base URL (required; 127.0.0.1 only)
//        SPIKE_PROJECT_ID      project id
//        SPIKE_OPTIONS         JSON options profile (see buildOptions)
import http from "node:http";
import { randomUUID } from "node:crypto";
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { StreamableHTTPServerTransport } from "@modelcontextprotocol/sdk/server/streamableHttp.js";
import { z } from "zod";
import * as agentcat from "agentcat";

const mode = process.argv[2] ?? "stdio";
const projectId = process.env.SPIKE_PROJECT_ID ?? "proj_0000000000000000000DIFFSPIKE";
const profile = JSON.parse(process.env.SPIKE_OPTIONS ?? "{}");

if (!/^http:\/\/(127\.0\.0\.1|localhost)(:\d+)?/.test(process.env.AGENTCAT_API_URL ?? "")) {
  process.stderr.write("refusing to start: AGENTCAT_API_URL must point at the local mock\n");
  process.exit(2);
}

// JSON options profile -> SDK options. Every key here must have a Rust twin.
function buildOptions(p) {
  const o = { disableDiagnostics: true };
  for (const k of ["enableTracing", "enableReportMissing", "enableToolCallContext", "enableAgentTracking", "customContextDescription"]) {
    if (k in p) o[k] = p[k];
  }
  if (p.sessionHook !== undefined) o.resolveSessionId = () => p.sessionHook; // string or null
  if (p.identify) o.identify = async () => ({ ...p.identify });
  if (p.eventTags) o.eventTags = () => ({ ...p.eventTags });
  if (p.eventProperties) o.eventProperties = () => ({ ...p.eventProperties });
  if (p.redactSecret) {
    o.redactSensitiveInformation = async (text) => text.split(p.redactSecret).join("[REDACTED]");
  }
  return o;
}

// Fixed tool contract shared with the Rust reference server.
function buildServer() {
  const server = new McpServer({ name: "diff-ref-server", version: "1.0.0" });
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
  server.registerTool(
    "boom",
    { description: "Always throws", inputSchema: {} },
    async () => {
      throw new Error("boom");
    },
  );
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
      await agentcat.publishCustomEvent(session, projectId, {
        resourceName: resource,
        message,
        parameters: { from: "emit_custom" },
        tags: { kind: "custom" },
      });
      return { content: [{ type: "text", text: "published" }] };
    },
  );
  if (process.env.SPIKE_NO_TRACK !== "1") agentcat.track(server, projectId, buildOptions(profile));
  return server;
}

if (mode === "stdio") {
  const server = buildServer();
  await server.connect(new StdioServerTransport());
} else {
  const port = Number(process.argv[3] ?? 0);
  // One tracked McpServer per MCP session (SDK 1.31 forbids reusing a
  // connected server); track() is called per server, like real deployments.
  const transports = new Map();
  const httpServer = http.createServer((req, res) => {
    void (async () => {
      const sid = req.headers["mcp-session-id"];
      let transport = sid ? transports.get(sid) : undefined;
      if (!transport) {
        transport = new StreamableHTTPServerTransport({
          sessionIdGenerator: () => randomUUID(),
          enableJsonResponse: true,
          onsessioninitialized: (id) => transports.set(id, transport),
        });
        await buildServer().connect(transport);
      }
      await transport.handleRequest(req, res);
    })().catch((e) => {
      process.stderr.write(`http error: ${e}\n`);
      if (!res.headersSent) res.writeHead(500);
      res.end();
    });
  });
  httpServer.listen(port, "127.0.0.1", () => {
    process.stdout.write(JSON.stringify({ ready: true, url: `http://127.0.0.1:${httpServer.address().port}/mcp` }) + "\n");
  });
}

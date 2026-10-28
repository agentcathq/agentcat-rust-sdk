// The TS twin of agentcat-sdk/examples/hooks.rs for the dashboard demo (plan 5, Task 17, decision C58).
// The same three tools (add, echo, fail), the same hooks and the same custom event, on the TS SDK built
// from the pinned commit, over stdio. The example is the source of truth for every value here.
//
// usage: node [--import harness/src/egress-guard.mjs] harness/demo/ts-server.mjs
// env:   AGENTCAT_PROJECT_ID  the project of every event (required)
//        AGENTCAT_API_URL     the API URL; when unset, the SDK default (the demo's --prod mode)
// This file loads no egress guard: run.mjs preloads it in --mock mode and not in --prod mode.
import { McpServer, fromJsonSchema } from "@modelcontextprotocol/server";
import { serveStdio } from "@modelcontextprotocol/server/stdio";

import { loadAgentCat } from "../src/servers/common.mjs";

const projectId = process.env.AGENTCAT_PROJECT_ID ?? "";
if (projectId === "") {
  process.stderr.write("ts-server: AGENTCAT_PROJECT_ID is required\n");
  process.exit(2);
}
const apiUrl = process.env.AGENTCAT_API_URL;

const agentcat = await loadAgentCat();

const text = (value) => ({ content: [{ type: "text", text: value }] });

const DRAFT = "https://json-schema.org/draft/2020-12/schema";

// The schemas that rmcp derives from the argument structs of hooks.rs.
const SCHEMAS = {
  add: {
    $schema: DRAFT,
    type: "object",
    properties: {
      a: { description: "The first number.", type: "number", format: "double" },
      b: { description: "The second number.", type: "number", format: "double" },
    },
    required: ["a", "b"],
  },
  echo: { $schema: DRAFT, type: "object", properties: { text: { description: "The text to send back.", type: "string" } }, required: ["text"] },
  fail: { type: "object", properties: {} },
};

/** The session_id that each call supplied, by the context of the call. */
const suppliedSessions = new WeakMap();

/** A JSON object with one string entry (`one_entry` of hooks.rs). */
const oneEntry = (key, value) => ({ [key]: value });

function buildServer() {
  const server = new McpServer({ name: "agentcat-example-hooks", version: "1.0.0" });
  const reg = (name, description, handler) =>
    server.registerTool(name, { description, inputSchema: fromJsonSchema(SCHEMAS[name]) }, handler);

  reg("add", "Adds two numbers", async ({ a, b }) => text(String(a + b)));

  // Sends the text back, and publishes one custom event for each call.
  reg("echo", "Sends the text back", async (args, ctx) => {
    // hooks.rs attaches the event to the session of the call. The SDK strips session_id from the
    // arguments of the tool, so the event_tags hook (it sees the raw request) leaves the ID for it. The
    // first call of a session has no ID yet, because the SDK issues it after the tool returns: the TS
    // custom event of such a call has no session (a limit of the TS API, not of the demo).
    const sessionId = suppliedSessions.get(ctx);
    try {
      await agentcat.publishCustomEvent(server, projectId, {
        resourceName: "echo-called",
        message: "the echo tool ran",
        ...(typeof sessionId === "string" && sessionId.startsWith("ses_") ? { sessionId } : {}),
      });
    } catch {
      // An event that cannot be queued is not an error for the tool.
    }
    return text(args.text);
  });

  // Always fails. The SDK records the error that the handler throws, as capture_error does in hooks.rs.
  reg("fail", "Always fails", async () => {
    const error = new Error("the record does not exist");
    // capture_error records the Rust type of the error, "core::io::error::Error" for the io::Error of hooks.rs.
    // The TS SDK records error.name as the type, so the example sets it. The platform, the stack and the frames
    // stay the SDK's own (README, "Demo").
    error.name = "core::io::error::Error";
    throw error;
  });

  agentcat.track(server, projectId, {
    ...(apiUrl ? { apiBaseUrl: apiUrl } : {}),
    disableDiagnostics: true,
    identify: async () => ({ userId: "demo-user", userName: "Demo User" }),
    eventTags: (request, extra) => {
      const supplied = request?.params?.arguments?.session_id;
      if (extra && typeof supplied === "string") suppliedSessions.set(extra, supplied);
      return oneEntry("example", "hooks");
    },
    eventProperties: () => oneEntry("source", "hooks-example"),
    redactSensitiveInformation: async (value) => value.replaceAll("secret-token", "[REDACTED]"),
    redactEvent: (event) => ({ ...event, properties: { ...(event.properties ?? {}), redacted_by: "example" } }),
  });
  return server;
}

serveStdio(buildServer, { onerror: (err) => process.stderr.write(`ts-server: ${err?.message ?? err}\n`) });

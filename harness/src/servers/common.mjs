// Shared by the TS reference servers, so they stay in step: ts-v1.mjs
// (@modelcontextprotocol/sdk 1.x) and ts-v2.mjs (@modelcontextprotocol/server
// 2.x). It holds the command line, the environment checks, the options profile
// and the tool behaviors; each server keeps only its MCP SDK wiring.
// Exit 2: bad arguments, a non-local AGENTCAT_API_URL or an invalid profile.
import { pathToFileURL } from "node:url";

import { ensureTs } from "oracle/fetch-ts";
import { tsDist } from "oracle/paths";

/** `serverInfo` of every reference server. */
export const SERVER_INFO = { name: "harness-ref-server", version: "1.0.0" };

/** Writes `<name>: <message>` to stderr and exits 2. */
export function fail(name, message) {
  process.stderr.write(`${name}: ${message}\n`);
  process.exit(2);
}

function isLocalHttpUrl(text) {
  let url;
  try {
    url = new URL(text);
  } catch {
    return false;
  }
  return url.protocol === "http:" && ["127.0.0.1", "localhost", "[::1]"].includes(url.hostname);
}

/**
 * Reads the command line and environment of server `name` (e.g. "ts-v1"), or exits 2.
 * Returns {mode, port, tracked, projectId, options}.
 */
export function startup(name) {
  const [mode = "stdio", portArg] = process.argv.slice(2);
  if (!["stdio", "http", "http-stateless"].includes(mode) || (mode === "stdio" && portArg !== undefined)) {
    fail(name, `usage: ${name}.mjs stdio | http <port> | http-stateless <port>`);
  }
  const port = Number(portArg ?? 0);
  if (mode !== "stdio" && !(Number.isInteger(port) && port >= 0 && port <= 65535)) fail(name, `bad port ${portArg}`);

  const apiUrl = process.env.AGENTCAT_API_URL ?? "";
  if (!isLocalHttpUrl(apiUrl)) {
    process.stderr.write(`refusing to start: AGENTCAT_API_URL must point at the local mock (got ${JSON.stringify(apiUrl)})\n`);
    process.exit(2);
  }

  const tracked = process.env.HARNESS_NO_TRACK !== "1";
  const projectId = process.env.HARNESS_PROJECT_ID ?? "";
  if (tracked && projectId === "") fail(name, "HARNESS_PROJECT_ID is required");

  let options;
  try {
    options = buildOptions(JSON.parse(process.env.HARNESS_OPTIONS ?? "{}"), apiUrl);
  } catch (err) {
    fail(name, `HARNESS_OPTIONS: ${err.message}`);
  }
  return { mode, port, tracked, projectId, options };
}

/** The TS SDK built from the pinned commit (fetched and built on first use). */
export async function loadAgentCat() {
  await ensureTs();
  return import(pathToFileURL(tsDist()).href);
}

// ---------------------------------------------------------------------------
// Options profile -> AgentCat options. Every key needs a Rust twin.
// ---------------------------------------------------------------------------
const FLAGS = ["enableTracing", "enableReportMissing", "enableToolCallContext", "enableAgentTracking", "customContextDescription"];
// Hooks whose src/types.ts signature allows a synchronous return.
const SYNC_HOOKS = ["resolveSessionId", "eventTags", "eventProperties"];
// Hooks whose signature returns a Promise.
const ASYNC_HOOKS = ["identify", "redactSensitiveInformation"];

// `apiUrl` (AGENTCAT_API_URL, which startup has checked) goes in as the `apiBaseUrl` option, the
// name at the pinned commit, not only through the environment: a TS version that stopped reading
// AGENTCAT_API_URL would otherwise fall back to the production default. `disableDiagnostics`
// backs up DISABLE_DIAGNOSTICS=true the same way.
function buildOptions(profile, apiUrl) {
  if (!profile || typeof profile !== "object" || Array.isArray(profile)) throw new Error("must be a JSON object");
  const o = { apiBaseUrl: apiUrl, disableDiagnostics: true };
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

// ---------------------------------------------------------------------------
// Tool definitions and behaviors (harness/README.md, "Tools"). TOOLS holds each
// tool's name, description and JSON schemas once; ts-v2 registers from it, and
// dump-contract.mjs writes ts-v2's tools/list to contract/tool-contract.json
// (R20), which the Rust server embeds. ts-v1 declares the same tools in zod; a
// test keeps the two in step. Adding a tool: TOOLS, toolHandlers, ts-v1's zod,
// the behavior in rust-server/src/tools.rs, then regenerate the contract.
// ---------------------------------------------------------------------------

/** Tool definitions: name, description, inputSchema and (optional) outputSchema, as JSON Schema. */
export const TOOLS = [
  {
    "name": "echo",
    "description": "Echo a message back",
    "inputSchema": {
      "type": "object",
      "properties": {
        "msg": {
          "type": "string",
          "description": "Message to echo"
        }
      },
      "required": [
        "msg"
      ],
      "additionalProperties": false,
      "$schema": "http://json-schema.org/draft-07/schema#"
    }
  },
  {
    "name": "structured",
    "description": "Returns structuredContent per its outputSchema",
    "inputSchema": {
      "type": "object",
      "properties": {
        "msg": {
          "type": "string"
        }
      },
      "required": [
        "msg"
      ],
      "additionalProperties": false,
      "$schema": "http://json-schema.org/draft-07/schema#"
    },
    "outputSchema": {
      "type": "object",
      "properties": {
        "echoed": {
          "type": "string",
          "description": "The echoed message"
        }
      },
      "required": [
        "echoed"
      ],
      "additionalProperties": false,
      "$schema": "http://json-schema.org/draft-07/schema#"
    }
  },
  {
    "name": "boom",
    "description": "Always throws",
    "inputSchema": {
      "type": "object",
      "properties": {},
      "$schema": "http://json-schema.org/draft-07/schema#"
    }
  },
  {
    "name": "tool_error",
    "description": "Returns an isError result",
    "inputSchema": {
      "type": "object",
      "properties": {},
      "$schema": "http://json-schema.org/draft-07/schema#"
    }
  },
  {
    "name": "big",
    "description": "Returns a large text payload",
    "inputSchema": {
      "type": "object",
      "properties": {
        "size": {
          "type": "integer"
        }
      },
      "required": [
        "size"
      ],
      "additionalProperties": false,
      "$schema": "http://json-schema.org/draft-07/schema#"
    }
  },
  {
    "name": "emit_custom",
    "description": "Publishes an AgentCat custom event",
    "inputSchema": {
      "type": "object",
      "properties": {
        "session": {
          "type": "string"
        },
        "resource": {
          "type": "string"
        },
        "message": {
          "type": "string"
        }
      },
      "required": [
        "session",
        "resource"
      ],
      "additionalProperties": false,
      "$schema": "http://json-schema.org/draft-07/schema#"
    }
  },
  {
    "name": "own_session",
    "description": "Declares its own session_id and returns the value it received",
    "inputSchema": {
      "type": "object",
      "properties": {
        "session_id": {
          "type": "string"
        },
        "x": {
          "type": "string"
        }
      },
      "required": [
        "session_id"
      ],
      "additionalProperties": false,
      "$schema": "http://json-schema.org/draft-07/schema#"
    }
  },
  {
    "name": "own_context",
    "description": "Declares its own context and returns it",
    "inputSchema": {
      "type": "object",
      "properties": {
        "context": {
          "type": "string"
        }
      },
      "required": [
        "context"
      ],
      "additionalProperties": false,
      "$schema": "http://json-schema.org/draft-07/schema#"
    }
  },
  {
    "name": "structured_own",
    "description": "Returns structuredContent; its outputSchema declares mcp_session",
    "inputSchema": {
      "type": "object",
      "properties": {
        "msg": {
          "type": "string"
        }
      },
      "required": [
        "msg"
      ],
      "additionalProperties": false,
      "$schema": "http://json-schema.org/draft-07/schema#"
    },
    "outputSchema": {
      "type": "object",
      "properties": {
        "echoed": {
          "type": "string"
        },
        "mcp_session": {
          "type": "object",
          "properties": {
            "session_id": {
              "type": "string"
            }
          },
          "required": [
            "session_id"
          ],
          "additionalProperties": true
        }
      },
      "required": [
        "echoed"
      ],
      "additionalProperties": false,
      "$schema": "http://json-schema.org/draft-07/schema#"
    }
  }
];

/**
 * Tool name -> handler (arguments -> CallToolResult). `agentcat` is the TS SDK
 * module. Each call returns new functions: call it once per server.
 */
export function toolHandlers({ tracked, projectId, agentcat }) {
  const text = (value) => ({ content: [{ type: "text", text: value }] });
  return {
    echo: async ({ msg }) => text(msg),
    structured: async ({ msg }) => ({ ...text(msg), structuredContent: { echoed: msg } }),
    boom: async () => {
      throw new Error("boom");
    },
    tool_error: async () => ({ isError: true, ...text("handled failure") }),
    big: async ({ size }) => text("x".repeat(size)),
    emit_custom: async ({ session, resource, message }) => {
      // Untracked: track() never configured the SDK's API URL, so a publish
      // would aim at the production default. Answer the same without one.
      if (!tracked) return text("published");
      await agentcat.publishCustomEvent(session, projectId, {
        resourceName: resource,
        message,
        parameters: { from: "emit_custom" },
        tags: { kind: "custom" },
      });
      return text("published");
    },
    own_session: async ({ session_id }) => text(`session_id=${session_id}`),
    own_context: async ({ context }) => text(context),
    structured_own: async ({ msg }) => ({ ...text(msg), structuredContent: { echoed: msg } }),
  };
}

// ---------------------------------------------------------------------------
// HTTP helpers.
// ---------------------------------------------------------------------------

/** The http-stateless answer to GET and DELETE. */
export function methodNotAllowed(res) {
  res.writeHead(405, { "content-type": "application/json", allow: "POST" });
  res.end(JSON.stringify({ jsonrpc: "2.0", error: { code: -32000, message: "Method not allowed." }, id: null }));
}

/** Logs a failed HTTP exchange of server `name` and answers 500 if nothing was sent yet. */
export function sendError(name, res, err) {
  process.stderr.write(`${name}: http error: ${err?.stack ?? err}\n`);
  if (!res.headersSent) res.writeHead(500);
  res.end();
}

/** Listens on 127.0.0.1:<port> and prints the ready line once listening. */
export function listen(httpServer, port) {
  httpServer.listen(port, "127.0.0.1", () => {
    process.stdout.write(`${JSON.stringify({ ready: true, url: `http://127.0.0.1:${httpServer.address().port}/mcp` })}\n`);
  });
}

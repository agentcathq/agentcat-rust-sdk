// Vectors for the key order of TS's in-memory events, from the real TS SDK: `track` on a v1
// McpServer (the @modelcontextprotocol/sdk of the TS clone) over an in-memory transport, a v1 client
// that sends `_meta` and the injected `context` argument, and `publishCustomEvent` in both forms.
// The redactEvent hook records the keys it sees and returns the event unchanged; the queue's
// sendEvent is replaced by a recorder, so nothing is published.
//
// Cases, each with its fields:
//   tool_call      event:   the keys of the tool-call event at the redactEvent hook, after the queue's
//                           stage 0, in order: [{key}, ...], with `"undefined": true` on each key whose
//                           value is undefined (JSON leaves it out). applyEventRedaction hides
//                           `redactionFn` and `eventRedactionFn` from the hook; the queue deletes
//                           `pending` only when it is set (this call has hooks, so it is gone)
//                  request: Object.keys of parameters.request
//                  params:  Object.keys of parameters.request.params, for a call with `_meta`
//                           TS records the JSON-RPC message with its keys in arrival order, so these
//                           two lists are not a TS order. `method, params` and the params order
//                           `name, arguments, _meta` are this probe's own input, which v1's
//                           Protocol.request passes through unchanged (`{...request, jsonrpc, id}`);
//                           only `jsonrpc, id`, after the caller's keys, come from v1. A client that
//                           sends another order gets that order back
//                  extra:   Object.keys of parameters.extra (v1's RequestHandlerExtra over the
//                           in-memory transport)
//   custom_string  event:   the keys of a session-string custom event as the queue sends it: the keys
//                           publishCustomEvent built, the keys truncateEvent assigned (undefined
//                           server and client fields), then `id`, which the queue adds last
//   custom_server  event:   the keys of a server-form custom event at the redactEvent hook (built by
//                           publishEvent, so `id` is first, as for a tool call; its `pending` is an
//                           undefined key the queue leaves in place)
// Every custom event field is set, so the undefined marks show only what TS itself leaves undefined.
import "../../egress-guard.mjs"; // first import: nothing below may reach the network

import { createRequire } from "node:module";
import { join } from "node:path";

import { loadCore } from "../../bundle.mjs";
import { tsDir } from "../../paths.mjs";

/** `Object.keys(object)` as [{key}, ...], with `undefined: true` on the undefined values. */
function keyEntries(object) {
  return Object.keys(object).map((key) => (object[key] === undefined ? { key, undefined: true } : { key }));
}

/** The MCP SDK v1 classes and zod from the TS clone's own dependencies. */
function mcpSdk() {
  const require = createRequire(join(tsDir(), "package.json"));
  return {
    McpServer: require("@modelcontextprotocol/sdk/server/mcp.js").McpServer,
    Client: require("@modelcontextprotocol/sdk/client/index.js").Client,
    InMemoryTransport: require("@modelcontextprotocol/sdk/inMemory.js").InMemoryTransport,
    CallToolResultSchema: require("@modelcontextprotocol/sdk/types.js").CallToolResultSchema,
    z: require("zod").z,
  };
}

/** Every field of CustomEventData, set. */
const CUSTOM_DATA = {
  sessionId: "ses_order000000000000000000000",
  resourceName: "custom-action",
  parameters: { action: "probe" },
  response: { ok: true },
  message: "probing key order",
  duration: 12,
  isError: false,
  error: { message: "none" },
  tags: { env: "probe" },
  properties: { device: "probe" },
};

/**
 * Runs the probes with the queue's sendEvent replaced by `onSend` (restored afterwards); resolves with
 * the keys the redactEvent hook saw ([entries, ...]) and the tool call's parameters.
 */
async function probe(core, onSend) {
  const { McpServer, Client, InMemoryTransport, CallToolResultSchema, z } = mcpSdk();
  const queue = core.eventQueue.eventQueue;
  const ownSend = Object.hasOwn(queue, "sendEvent") ? queue.sendEvent : undefined;
  const hookKeys = [];
  let parameters;
  let sent = 0;
  queue.sendEvent = async (event) => {
    onSend(event);
    sent += 1;
  };

  const server = new McpServer({ name: "order-server", version: "1.0.0" });
  server.registerTool("echo", { description: "Echo", inputSchema: { msg: z.string() } }, async ({ msg }) => ({
    content: [{ type: "text", text: msg }],
  }));
  core.sdk.track(server, "proj_order", {
    disableDiagnostics: true,
    identify: async () => ({ userId: "user-1", userName: "User One", userData: { plan: "pro" } }),
    eventTags: async () => ({ env: "probe" }),
    eventProperties: async () => ({ device: "probe" }),
    redactEvent: (event) => {
      hookKeys.push(keyEntries(event));
      parameters ??= event.parameters;
      return event;
    },
  });
  const client = new Client({ name: "order-client", version: "2.0.0" });
  const [clientTransport, serverTransport] = InMemoryTransport.createLinkedPair();
  try {
    await Promise.all([client.connect(clientTransport), server.server.connect(serverTransport)]);
    await client.listTools();
    await client.request(
      {
        method: "tools/call",
        params: { name: "echo", arguments: { msg: "hello", context: "probing key order" }, _meta: { progressToken: "probe-1" } },
      },
      CallToolResultSchema,
    );
    // One event at a time, so the hook and send orders are the probe order.
    await waitFor(() => sent === 1);
    await core.sdk.publishCustomEvent(server, "proj_order", CUSTOM_DATA);
    await waitFor(() => sent === 2);
    await core.sdk.publishCustomEvent("order-session", "proj_order", CUSTOM_DATA);
    await waitFor(() => sent === 3);
  } finally {
    await client.close();
    if (ownSend === undefined) delete queue.sendEvent;
    else queue.sendEvent = ownSend;
  }
  return { hookKeys, parameters };
}

/** Resolves once `done()` is true; throws after 5 s. */
async function waitFor(done) {
  const deadline = Date.now() + 5000;
  while (!done()) {
    if (Date.now() > deadline) throw new Error("oracle: the TS queue did not send the probe events within 5 s");
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
}

export async function generate() {
  const core = await loadCore();
  const sentKeys = [];
  const { hookKeys, parameters } = await probe(core, (event) => sentKeys.push(keyEntries(event)));
  // The tool call and the server-form custom event pass the hook, in that order; the session-string
  // form has no hook (B-088), so its keys are taken where the queue sends it, the third send.
  if (hookKeys.length !== 2 || sentKeys.length !== 3) {
    throw new Error(`oracle: expected 2 hooked and 3 sent events, got ${hookKeys.length} and ${sentKeys.length}`);
  }
  return {
    cases: [
      {
        name: "tool_call",
        event: hookKeys[0],
        request: Object.keys(parameters.request),
        params: Object.keys(parameters.request.params),
        extra: Object.keys(parameters.extra),
      },
      { name: "custom_string", event: sentKeys[2] },
      { name: "custom_server", event: hookKeys[1] },
    ],
  };
}

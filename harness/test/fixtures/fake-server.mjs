// Test fixture: a fake MCP server over stdio (the low-level server of @modelcontextprotocol/sdk 1.x,
// untracked, no publishes), for the driver's own steps.
//   tools/list  five tools t0..t4, two per page; the cursors are "page-2" and "page-3" (opaque, not
//               the reference servers' start indexes), and the last page has no nextCursor
//   gate        {id, want}: holds the call until `want` calls are held at the gate on this process (or
//               3 s pass), then answers after max(0, 200 - 40 * id) ms, so later calls answer first; the
//               text is {"id": <id>, "together": <calls held when the gate released this one>}
//   env         the text is HARNESS_PAGE_SIZE as the server received it, or "unset"
import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { CallToolRequestSchema, ListToolsRequestSchema } from "@modelcontextprotocol/sdk/types.js";

const TOOLS = ["t0", "t1", "t2", "t3", "t4"].map((name) => ({ name, inputSchema: { type: "object", properties: {} } }));
/** cursor -> [start index, nextCursor]. */
const PAGES = new Map([
  [undefined, [0, "page-2"]],
  ["page-2", [2, "page-3"]],
  ["page-3", [4, undefined]],
]);

const server = new Server({ name: "fake-server", version: "1.0.0" }, { capabilities: { tools: {} } });
server.setRequestHandler(ListToolsRequestSchema, (request) => {
  const [start, nextCursor] = PAGES.get(request.params?.cursor) ?? [0, "page-2"];
  return { tools: TOOLS.slice(start, start + 2), ...(nextCursor && { nextCursor }) };
});

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const text = (value) => ({ content: [{ type: "text", text: value }] });
/** The release functions of the calls held at the gate. */
const held = [];

/** Holds a call until `want` calls are held (all are released together) or 3 s pass; resolves the count. */
function gate(want) {
  return new Promise((release) => {
    held.push(release);
    if (held.length >= want) {
      const count = held.length;
      for (const r of held.splice(0)) r(count);
      return;
    }
    setTimeout(() => {
      const at = held.indexOf(release);
      if (at < 0) return;
      const count = held.length;
      held.splice(at, 1);
      release(count);
    }, 3000);
  });
}

server.setRequestHandler(CallToolRequestSchema, async (request) => {
  const { name, arguments: args = {} } = request.params;
  if (name === "env") return text(process.env.HARNESS_PAGE_SIZE ?? "unset");
  if (name !== "gate") return { isError: true, ...text(`no tool ${name}`) };
  const together = await gate(args.want);
  await sleep(Math.max(0, 200 - 40 * args.id));
  return text(JSON.stringify({ id: args.id, together }));
});

await server.connect(new StdioServerTransport());

// A fake customer example for the examples-check tests: a stdio MCP server with the tools `add`, `echo`
// and `fail`, which publishes one tool-call event per call to AGENTCAT_API_URL, as a tracked example does.
// FAKE_MODE picks one misbehavior:
//   ok           behaves
//   junk         prints a line that is not JSON-RPC on stdout before it serves
//   no-publish   publishes nothing for the `add` call
//   exit1        exits 1 when stdin closes
//   slow         exits 0 two seconds after stdin closes
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";

const mode = process.env.FAKE_MODE ?? "ok";
const api = process.env.AGENTCAT_API_URL;
if (mode === "junk") process.stdout.write("starting up\n");

async function publish(resourceName) {
  if (mode === "no-publish" && resourceName === "add") return;
  await fetch(`${api}/v1/public/events/`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ eventType: "mcp:tools/call", resourceName, projectId: process.env.AGENTCAT_PROJECT_ID }),
  });
}

const text = (value, isError = false) => ({ content: [{ type: "text", text: value }], isError });
const server = new McpServer({ name: "example-fake", version: "1.0.0" });
server.tool("add", { a: z.number(), b: z.number() }, async ({ a, b }) => {
  await publish("add");
  return text(String(a + b));
});
server.tool("echo", { text: z.string() }, async (args) => {
  await publish("echo");
  return text(args.text);
});
server.tool("fail", {}, async () => {
  await publish("fail");
  return text("failed on purpose", true);
});

process.stdin.on("end", () => {
  if (mode === "exit1") process.exit(1);
  if (mode === "slow") setTimeout(() => process.exit(0), 2000);
  else process.exit(0);
});
await server.connect(new StdioServerTransport());

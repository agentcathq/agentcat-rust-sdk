// Long-running oracle process for property tests. Reads one JSON request per line on
// stdin: {"id": n, "fn": "<name>", "args": [...]}. Writes one JSON reply per line on
// stdout, in request order: {"id": n, "ok": <result>}, {"id": n, "undefined": true}
// or {"id": n, "err": "<message>"}. Exits when stdin closes.
// Stdout is the reply channel: progress and diagnostics go to stderr only.
import { createInterface } from "node:readline";

import { ensureTs } from "./fetch-ts.mjs";

// Start reading now, so lines sent while the TS SDK builds wait in the queue.
const lines = createInterface({ input: process.stdin })[Symbol.asyncIterator]();

function reply(message) {
  return new Promise((resolve) => {
    // A closed stdout means the client is gone, so stop serving.
    process.stdout.write(`${JSON.stringify(message)}\n`, (error) => (error ? process.exit(0) : resolve()));
  });
}

async function handle(line, functions) {
  let request;
  try {
    request = JSON.parse(line);
  } catch (error) {
    return { id: null, err: `invalid JSON request: ${error.message}` };
  }
  const id = request?.id ?? null;
  try {
    const fn = Object.hasOwn(functions, request?.fn) ? functions[request.fn] : undefined;
    if (typeof fn !== "function") return { id, err: `unknown function ${JSON.stringify(request?.fn)}` };
    if (!Array.isArray(request.args)) return { id, err: '"args" must be an array' };
    const result = await fn(...request.args);
    if (result === undefined) return { id, undefined: true };
    // JSON.stringify returns undefined for functions and symbols; report it as an error.
    if (JSON.stringify(result) === undefined) return { id, err: "result is not JSON-serializable" };
    return { id, ok: result };
  } catch (error) {
    return { id, err: String(error?.stack ?? error) };
  }
}

try {
  await ensureTs();
  const { functions } = await import("./functions.mjs");
  for await (const line of { [Symbol.asyncIterator]: () => lines }) {
    if (line.trim() === "") continue;
    await reply(await handle(line, functions));
  }
} catch (error) {
  process.stderr.write(`oracle server failed: ${error?.stack ?? error}\n`);
  process.exit(1);
}

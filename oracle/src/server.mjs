// Long-running oracle process for property tests. Reads one JSON request per line on
// stdin: {"id": n, "fn": "<name>", "args": [...]}. Writes one JSON reply per line on
// stdout, in request order:
//   {"id": n, "ok": <result>}                  the result in the strict encoding (strict-json.mjs)
//   {"id": n, "undefined": true}               the function returned undefined
//   {"id": n, "err": "<message>", "stack": s}  the function threw: `message` is error.message for an
//                                              Error and String(error) otherwise; `stack` is the
//                                              Error's stack, or null. A bad request, and a result
//                                              the strict encoding rejects ("result is not strict
//                                              JSON: <reason>"), are err replies with a null stack.
// Exits when stdin closes.
// Stdout is the reply channel: progress and diagnostics go to stderr only.
import { createInterface } from "node:readline";

import { ensureTs } from "./fetch-ts.mjs";
import { traceOverride } from "./paths.mjs";
import { encodeStrict } from "./strict-json.mjs";

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
    return { id: null, err: `invalid JSON request: ${error.message}`, stack: null };
  }
  const id = request?.id ?? null;
  let result;
  try {
    const fn = Object.hasOwn(functions, request?.fn) ? functions[request.fn] : undefined;
    if (typeof fn !== "function") return { id, err: `unknown function ${JSON.stringify(request?.fn)}`, stack: null };
    if (!Array.isArray(request.args)) return { id, err: '"args" must be an array', stack: null };
    result = await fn(...request.args);
  } catch (error) {
    return { id, ...thrown(error) };
  }
  if (result === undefined) return { id, undefined: true };
  try {
    return { id, ok: encodeStrict(result) };
  } catch (error) {
    const reason = String(error?.message ?? error).replace(/^strict-json: /, "");
    return { id, err: `result is not strict JSON: ${reason}`, stack: null };
  }
}

/** The err and stack of a thrown value: TS's message verbatim, apart from the stack. */
function thrown(error) {
  if (error instanceof Error) {
    return { err: String(error.message), stack: typeof error.stack === "string" ? error.stack : null };
  }
  let err;
  try {
    err = String(error);
  } catch {
    err = Object.prototype.toString.call(error); // an object with no usable toString
  }
  return { err, stack: null };
}

try {
  traceOverride();
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

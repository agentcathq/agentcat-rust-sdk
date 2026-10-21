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
//                                              `err` and `stack` are in the strict encoding too: a
//                                              message can quote a lone surrogate of the input, which
//                                              is a $utf16 marker ({"$utf16": [...]}), not a string.
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
    return failure(null, `invalid JSON request: ${error.message}`);
  }
  const id = request?.id ?? null;
  let result;
  try {
    const fn = Object.hasOwn(functions, request?.fn) ? functions[request.fn] : undefined;
    if (typeof fn !== "function") return failure(id, `unknown function ${JSON.stringify(request?.fn)}`);
    if (!Array.isArray(request.args)) return failure(id, '"args" must be an array');
    result = await fn(...request.args);
  } catch (error) {
    const { err, stack } = thrown(error);
    return failure(id, err, stack);
  }
  if (result === undefined) return { id, undefined: true };
  try {
    return { id, ok: encodeStrict(result) };
  } catch (error) {
    const reason = String(error?.message ?? error).replace(/^strict-json: /, "");
    return failure(id, `result is not strict JSON: ${reason}`);
  }
}

/** An err reply. A string that is not well-formed UTF-16 becomes a $utf16 marker; null stays null. */
function failure(id, err, stack = null) {
  return { id, err: encodeStrict(err), stack: encodeStrict(stack) };
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

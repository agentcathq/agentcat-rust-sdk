// The oracle server speaks JSON lines: one request line in, one response line out, in order.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { readFileSync } from "node:fs";
import { after, test } from "node:test";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";

import { lock } from "../src/paths.mjs";

const SERVER = fileURLToPath(new URL("../src/server.mjs", import.meta.url));
const child = spawn(process.execPath, [SERVER], { stdio: ["pipe", "pipe", "inherit"] });
const lines = createInterface({ input: child.stdout })[Symbol.asyncIterator]();
after(() => child.stdin.end());

async function ask(request) {
  child.stdin.write(`${typeof request === "string" ? request : JSON.stringify(request)}\n`);
  const { value, done } = await lines.next();
  assert.equal(done, false, "server closed stdout");
  return value;
}

test("JSON.stringify drops the fraction of whole floats", async () => {
  const line = await ask('{"id":1,"fn":"JSON.stringify","args":[{"n":1.0}]}');
  assert.equal(line, '{"id":1,"ok":"{\\"n\\":1}"}');
});

test("an unknown function gets an err line and the server keeps serving", async () => {
  const reply = JSON.parse(await ask({ id: 2, fn: "nope", args: [] }));
  assert.deepEqual(reply, { id: 2, err: 'unknown function "nope"', stack: null });
  assert.equal(await ask({ id: 21, fn: "JSON.stringify", args: [1] }), '{"id":21,"ok":"1"}');
});

test("an undefined result is sent as undefined:true", async () => {
  const line = await ask('{"id":3,"fn":"tokenEstimate.estimateInputTokens","args":[null]}');
  assert.equal(line, '{"id":3,"undefined":true}');
});

test("estimateInputTokens counts a whole float as 1", async () => {
  const line = await ask('{"id":4,"fn":"tokenEstimate.estimateInputTokens","args":[{"n":1.0}]}');
  assert.equal(line, '{"id":4,"ok":2}');
});

test("a malformed line gets an err line and the server keeps serving", async () => {
  const reply = JSON.parse(await ask("not json"));
  assert.equal(typeof reply.err, "string");
  assert.equal(await ask({ id: 5, fn: "JSON.stringify", args: [2] }), '{"id":5,"ok":"2"}');
});

test("a thrown error replies its message and, separately, its stack", async () => {
  const reply = JSON.parse(await ask({ id: 6, fn: "oracle.throw", args: ["boom"] }));
  assert.equal(reply.id, 6);
  assert.equal(reply.err, "boom");
  assert.equal(typeof reply.stack, "string");
  assert.ok(reply.stack.includes("TypeError: boom"), reply.stack);
});

test("oracle.sha replies the SHA the oracle runs", async () => {
  assert.equal(await ask({ id: 7, fn: "oracle.sha", args: [] }), JSON.stringify({ id: 7, ok: lock().sha }));
});

test("a result is sent in the strict encoding: -0 is a $f64 marker", async () => {
  const line = await ask('{"id":5,"fn":"oracle.echo","args":[-0]}');
  assert.equal(line, '{"id":5,"ok":{"$f64":"8000000000000000"}}');
});

test("a result the strict encoding rejects is an err line with a null stack", async () => {
  const line = await ask('{"id":8,"fn":"oracle.echo","args":[{"$f64":"x"}]}');
  assert.equal(line, '{"id":8,"err":"result is not strict JSON: $: reserved key $f64","stack":null}');
});

test("err and stack are in the strict encoding: a lone surrogate in a message is a $utf16 marker", async () => {
  const line = await ask({ id: 9, fn: "oracle.throw", args: ["a\ud83d"] });
  const reply = JSON.parse(line);
  assert.equal(reply.id, 9);
  assert.deepEqual(reply.err, { $utf16: [97, 0xd83d] });
  assert.deepEqual(Object.keys(reply.stack), ["$utf16"]);
  const stack = String.fromCharCode(...reply.stack.$utf16);
  assert.ok(stack.startsWith("TypeError: a\ud83d\n"), stack);
  // The line itself is well-formed: serde_json rejects a lone \udXXX escape.
  assert.ok(line.isWellFormed() && !/\\ud[89a-f]/i.test(line), line);
});

test("a server started with ORACLE_SHA writes one trace line to stderr", { timeout: 120_000 }, async () => {
  const locked = JSON.parse(readFileSync(new URL("../oracle.lock", import.meta.url), "utf8")).sha;
  const traced = spawn(process.execPath, [SERVER], {
    env: { ...process.env, ORACLE_SHA: locked },
    stdio: ["pipe", "ignore", "pipe"],
  });
  let stderr = "";
  traced.stderr.setEncoding("utf8").on("data", (text) => (stderr += text));
  const exited = new Promise((resolve) => traced.once("close", resolve));
  traced.stdin.end();
  assert.equal(await exited, 0, stderr);
  const line = `oracle: ORACLE_SHA=${locked} overrides oracle/oracle.lock (${locked})\n`;
  assert.equal(stderr.split(line).length - 1, 1, stderr);
});

test("the server exits when stdin closes", async () => {
  const exited = new Promise((resolve) => child.once("exit", resolve));
  child.stdin.end();
  assert.equal(await exited, 0);
});

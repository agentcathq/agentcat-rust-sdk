// The oracle server speaks JSON lines: one request line in, one response line out, in order.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { after, test } from "node:test";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";

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
  assert.equal(reply.id, 2);
  assert.equal(typeof reply.err, "string");
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

test("the server exits when stdin closes", async () => {
  const exited = new Promise((resolve) => child.once("exit", resolve));
  child.stdin.end();
  assert.equal(await exited, 0);
});

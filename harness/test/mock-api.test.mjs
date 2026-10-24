// The mock API keeps each request's raw body text beside the parsed body (number-source reads it).
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { join } from "node:path";
import { createInterface } from "node:readline";
import { test } from "node:test";

import { HARNESS } from "./support.mjs";

const MOCK = join(HARNESS, "src", "mock-api.mjs");

/** Starts the mock on an ephemeral port; resolves with its base URL. The test kills it at the end. */
async function startMock(t) {
  const child = spawn(process.execPath, [MOCK, "0"], { stdio: ["ignore", "pipe", "inherit"] });
  t.after(() => child.kill("SIGKILL"));
  const line = await new Promise((resolve, reject) => {
    child.once("error", reject);
    createInterface({ input: child.stdout }).once("line", resolve);
  });
  return `http://127.0.0.1:${JSON.parse(line).port}`;
}

test("a publish keeps its raw body text as rawBody beside the parsed body", async (t) => {
  const url = await startMock(t);
  const raw = '{"event_type":"mcp:tools/call","n":1.0,"z":-0,"big":9007199254740993}';
  const res = await fetch(`${url}/v1/public/events/`, { method: "POST", headers: { "content-type": "application/json" }, body: raw });
  assert.equal(res.status, 202);
  const [request] = await (await fetch(`${url}/__requests`)).json();
  assert.equal(request.rawBody, raw);
  assert.deepEqual(request.body, { event_type: "mcp:tools/call", n: 1, z: 0, big: 9007199254740992 });
});

test("a body that is not JSON is kept as text in both fields", async (t) => {
  const url = await startMock(t);
  await fetch(`${url}/v1/public/events/`, { method: "POST", body: "not json" });
  const [request] = await (await fetch(`${url}/__requests`)).json();
  assert.equal(request.rawBody, "not json");
  assert.equal(request.body, "not json");
});

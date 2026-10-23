// The queue generator runs each case through TS's own EventQueue (processEvent with a recording API
// client and a recording backoff delay) and writes the requests, delays and log lines TS produced.
import assert from "node:assert/strict";
import { test } from "node:test";

import { generate } from "../src/vectors/gen/queue.mjs";
import { checkSet } from "../src/vectors/write.mjs";

const { cases } = await generate();
const byName = new Map(cases.map((c) => [c.name, c]));

/** The cases named `name` or starting with `name:`. */
function family(name) {
  return cases.filter((c) => c.name === name || c.name.startsWith(`${name}:`));
}

const FAMILIES = [
  "call:plain",
  "call:telemetry-only",
  "call:no-session",
  "hooks:identify",
  "hooks:identify-throws",
  "hooks:identify-hangs",
  "hooks:tags-invalid",
  "hooks:tags-collide",
  "hooks:properties-empty",
  "hooks:properties-throws",
  "hooks:session-hook",
  "hooks:session-hook-null",
  "redact-event:drop",
  "redact-event:throws",
  "redact-event:merge-restored",
  "redact-strings:replace",
  "redact-strings:throws",
  "sanitize:image-block",
  "truncate:big-response",
  "truncate:non-string-context",
  "retry:2-failures",
  "retry:4-failures",
  "custom:string-form",
  "custom:string-form-invalid-tags",
  "custom:server-form",
  "custom:server-form-redact-drop",
];

test("queue follows the vector-set rules and has every case family of the brief", () => {
  checkSet("queue", cases);
  for (const name of FAMILIES) assert.ok(family(name).length > 0, `no case ${name}`);
});

test("every case has an event, requests, delays and logs", () => {
  for (const c of cases) {
    assert.equal(typeof c.event, "object", c.name);
    for (const field of ["requests", "delays", "logs"]) assert.ok(Array.isArray(c[field]), `${c.name}: ${field}`);
    assert.ok(c.logs.every((line) => typeof line === "string"), c.name);
    assert.ok(c.delays.every((ms) => Number.isInteger(ms)), c.name);
  }
});

test("retry:4-failures backs off 1, 2 and 4 s, then logs the final failure", () => {
  const c = byName.get("retry:4-failures");
  assert.deepEqual(c.delays, [1000, 2000, 4000]);
  assert.equal(c.requests.length, 4);
  assert.match(c.logs.at(-1), /^Failed to deliver AgentCat event after retries:/);
});

test("retry:2-failures backs off 1 and 2 s and is sent on the third attempt", () => {
  const c = byName.get("retry:2-failures");
  assert.deepEqual(c.delays, [1000, 2000]);
  assert.equal(c.requests.length, 3);
  assert.match(c.logs.at(-1), /^Successfully sent event <evt> \| /);
});

test("a telemetry-only event is not sent, and a sessionless event is sent with a null session", () => {
  assert.deepEqual(byName.get("call:telemetry-only").requests, []);
  const [request] = byName.get("call:no-session").requests;
  assert.equal(request.sessionId, null);
});

test("event IDs and minted session IDs are masked; the hooks that hang time out", () => {
  const text = JSON.stringify(cases);
  assert.doesNotMatch(text, /evt_[0-9A-Za-z]{27}/);
  for (const c of cases) for (const request of c.requests) assert.equal(request.id, "<evt>", c.name);
  assert.equal(byName.get("hooks:session-hook-null").requests[0].sessionId, "<minted>");
  assert.deepEqual(byName.get("hooks:identify-hangs").logs.slice(0, 1), [
    "Warning: identify hook did not settle within 30000ms; publishing event without it",
  ]);
});

test("a second run writes the same cases", async () => {
  const again = await generate();
  assert.deepEqual(again.cases, cases);
});

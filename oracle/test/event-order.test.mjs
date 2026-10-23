// The event_order generator runs the real TS SDK (track on a v1 McpServer over an in-memory
// transport, publishCustomEvent in both forms) and writes the key order of each in-memory event.
import assert from "node:assert/strict";
import { test } from "node:test";

import { generate } from "../src/vectors/gen/event_order.mjs";
import { checkSet } from "../src/vectors/write.mjs";

const { cases } = await generate();
const byName = new Map(cases.map((c) => [c.name, c]));
const keys = (entries) => entries.map((entry) => entry.key);

test("event_order has the tool_call, custom_string and custom_server cases and follows the vector-set rules", () => {
  assert.deepEqual([...byName.keys()], ["tool_call", "custom_string", "custom_server"]);
  checkSet("event_order", cases);
});

test("a tool-call event lists id, sessionId, projectId, eventType and timestamp first", () => {
  assert.deepEqual(keys(byName.get("tool_call").event).slice(0, 5), ["id", "sessionId", "projectId", "eventType", "timestamp"]);
});

test("every event entry is {key} or {key, undefined: true}, and undefined keys are marked", () => {
  for (const name of ["tool_call", "custom_string", "custom_server"]) {
    const entries = byName.get(name).event;
    assert.ok(entries.length > 0, name);
    for (const entry of entries) {
      assert.equal(typeof entry.key, "string", name);
      const extra = Object.keys(entry).filter((k) => k !== "key");
      assert.ok(extra.length === 0 || (extra.length === 1 && entry.undefined === true), `${name}: ${JSON.stringify(entry)}`);
    }
    assert.equal(new Set(keys(entries)).size, entries.length, `${name}: a key is listed twice`);
  }
  // TS's publishEvent always sets ipAddress, to undefined.
  assert.deepEqual(byName.get("tool_call").event.find((e) => e.key === "ipAddress"), { key: "ipAddress", undefined: true });
});

test("the tool call's request, params and extra key lists are string arrays, and params has _meta", () => {
  const call = byName.get("tool_call");
  for (const field of ["request", "params", "extra"]) {
    assert.ok(Array.isArray(call[field]) && call[field].length > 0, field);
    assert.ok(call[field].every((k) => typeof k === "string"), field);
  }
  assert.ok(call.request.includes("params"));
  assert.ok(call.params.includes("_meta"));
  assert.ok(call.params.includes("arguments"));
});

test("the queue adds a session-string custom event's id last; the server form has it first", () => {
  assert.equal(keys(byName.get("custom_string").event).at(-1), "id");
  assert.equal(keys(byName.get("custom_string").event).indexOf("id"), byName.get("custom_string").event.length - 1);
  assert.equal(keys(byName.get("custom_server").event)[0], "id");
});

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

/**
 * Every case family of the brief and what at least one of its cases must hold: `fields` it carries
 * (as inputs), and `holds`, a check on its outputs. A family is a name or a `name:` prefix.
 */
const FAMILIES = [
  ["call:plain", ["event"], (c) => c.event.eventType === "mcp:tools/call" && c.event.userIntent === "say hello" && c.requests.length === 1],
  ["call:telemetry-only", [], (c) => c.requests.length === 0],
  ["call:no-session", ["ts_test"], (c) => c.requests[0].sessionId === null],
  ["hooks:identify", ["hooks"], (c) => c.hooks.identify.returns?.userId === "user-42" && c.requests[0].identifyActorGivenId === "user-42"],
  ["hooks:identify-throws", ["hooks"], (c) => c.hooks.identify.throws !== undefined && c.logs.length > 1],
  ["hooks:identify-hangs", ["hooks"], (c) => c.hooks.identify.hangs === true && c.requests.length === 1],
  ["hooks:tags-invalid", ["hooks"], (c) => c.hooks.eventTags !== undefined && c.logs.some((l) => /tag/i.test(l))],
  ["hooks:tags-collide", ["hooks"], (c) => c.hooks.eventTags !== undefined && c.requests.length === 1],
  ["hooks:properties-empty", ["hooks"], (c) => c.hooks.eventProperties !== undefined],
  ["hooks:properties-throws", ["hooks"], (c) => c.hooks.eventProperties.throws !== undefined],
  ["hooks:session-hook", ["hook_mode", "hooks"], (c) => c.hook_mode === true && c.hooks.resolveSessionId !== undefined],
  ["hooks:session-hook-null", ["hook_mode", "hooks"], (c) => c.requests[0].sessionId === "<minted>"],
  ["redact-event:drop", ["redact_event"], (c) => c.redact_event.returns === null && c.requests.length === 0],
  ["redact-event:throws", ["redact_event"], (c) => c.redact_event.throws !== undefined],
  ["redact-event:merge-restored", ["redact_event"], (c) => c.redact_event.merge !== undefined && c.requests.length === 1],
  ["redact-strings:replace", ["redact_strings"], (c) => c.redact_strings.replace !== undefined && c.requests[0].error.message.includes("[REDACTED]") && !c.requests[0].error.message.includes(c.redact_strings.replace)],
  ["redact-strings:throws", ["redact_strings"], (c) => c.redact_strings.throw_at !== undefined],
  ["sanitize:image-block", ["event"], (c) => c.requests.length === 1],
  ["truncate:big-response", ["event"], (c) => c.requests.length === 1],
  ["truncate:non-string-context", ["event"], (c) => c.requests.length === 0 && c.logs.length === 1],
  ["retry:2-failures", ["fail_attempts"], (c) => c.fail_attempts === 2],
  ["retry:4-failures", ["fail_attempts"], (c) => c.fail_attempts === 4],
  ["custom:string-form", ["custom"], (c) => c.custom.form === "string" && c.event.eventType === "agentcat:custom"],
  ["custom:string-form-invalid-tags", ["custom"], (c) => c.custom.form === "string" && c.logs.some((l) => /tag/i.test(l))],
  ["custom:server-form", ["custom"], (c) => c.custom.form === "server" && c.requests.length === 1],
  ["custom:server-form-redact-drop", ["custom", "redact_event"], (c) => c.custom.form === "server" && c.requests.length === 0],
];

test("queue follows the vector-set rules", () => {
  checkSet("queue", cases);
});

test("every case family of the brief has a case with the fields and the outputs it names", () => {
  for (const [name, fields, holds] of FAMILIES) {
    const members = family(name);
    assert.ok(members.length > 0, `no case ${name}`);
    const withFields = members.filter((c) => fields.every((field) => field in c));
    assert.ok(withFields.length > 0, `${name}: no case has the fields ${fields.join(", ")}`);
    assert.ok(withFields.some((c) => holds(c)), `${name}: no case has the expected content`);
  }
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

test("a tool-call error is the one TS's captureException builds from the result, as callWrap does", () => {
  const withError = cases.filter((c) => c.event.isError === true && c.custom === undefined);
  assert.deepEqual(withError.map((c) => c.name).sort(), ["call:tool-error", "redact-strings:replace"]);
  for (const c of withError) {
    const text = c.event.response.content.filter((block) => block.type === "text").map((block) => block.text).join(" ").trim();
    assert.deepEqual(c.event.error, { message: text, platform: "javascript" }, c.name);
  }
});

test("call:protocol-version carries the negotiated protocol version as the SDK tag, as callWrap builds the tags", () => {
  const c = byName.get("call:protocol-version");
  assert.equal(c.event.tags.agentcat_protocol_version, "2025-11-25");
  assert.equal(c.requests[0].tags.agentcat_protocol_version, "2025-11-25");
  assert.equal(byName.get("call:plain").requests[0].tags.agentcat_protocol_version, undefined);
});

// Vectors for the redaction of the TS SDK (src/modules/redaction.ts), computed by the TS functions
// through the oracle functions `oracle.redactEvent` and `oracle.applyEventRedaction`: the string
// walk with its protected top-level keys and its call order, the restore of the system-managed
// fields after the event-level hook, and the redactEvent and applyEventRedaction tests of
// redaction.test.ts that JSON can carry.
//
// A top-level `timestamp` is a Date in TS and an ISO string in the Rust event: the oracle functions
// turn a top-level string `timestamp` into a Date before TS runs and back after. A nested
// `timestamp` stays a string.
//
// Two kinds of case:
//   walk     input: the event, or input_text: its JSON text, when the key order of the text
//            matters or it holds a `__proto__` own key; throw_at: the redactor call (1-based) that
//            throws, when one does; output: the redacted event, or error: String(error) when
//            redactEvent throws; calls: every string the redactor got, in order (the redactor
//            gives "[R:" + s + "]").
//   restore  original: the event before the hook; replacement: what the hook returns (null, or
//            absent for undefined: the event is dropped); output: the event after
//            applyEventRedaction, or null when it is dropped. The names start with "apply:".
// `deviation: "RD-24"` marks a case whose TS output lacks a `__proto__` key that Rust keeps: TS
// assigns each key of the walk to a new object (and Object.assign the hook's result to the
// event), and assigning `__proto__` sets a prototype instead of making a key. The calls are TS's.
import { functions } from "../../functions.mjs";
import { int, pick, seeded, text, tsKey } from "../support.mjs";

const redactEvent = functions["oracle.redactEvent"];
const applyEventRedaction = functions["oracle.applyEventRedaction"];

/** The PROTECTED_FIELDS of redaction.ts. */
const PROTECTED = [
  "sessionId",
  "id",
  "projectId",
  "server",
  "serverName",
  "serverVersion",
  "clientName",
  "clientVersion",
  "identifyActorGivenId",
  "identifyActorName",
  "identifyData",
  "resourceName",
  "eventType",
  "actorId",
  "tags",
  "properties",
];

/** The RESTORED_FIELDS of redaction.ts. */
const RESTORED = ["id", "sessionId", "projectId", "eventType", "timestamp"];

const TIMESTAMP = "2025-01-15T12:00:00.000Z";

const tsTest = (group, title) => [tsKey("redaction", `${group} > ${title}`)];

/** Throws with `message` unless `ok`. */
function check(ok, message) {
  if (!ok) throw new Error(`redact vectors: ${message}`);
}

/** `true` when `value` has a `__proto__` own key anywhere. */
function hasProtoKey(value) {
  if (Array.isArray(value)) return value.some(hasProtoKey);
  if (value !== null && typeof value === "object") {
    return Object.hasOwn(value, "__proto__") || Object.values(value).some(hasProtoKey);
  }
  return false;
}

/** `event` without the keys of `keys`. */
const without = (event, ...keys) => Object.fromEntries(Object.entries(event).filter(([k]) => !keys.includes(k)));

export async function generate() {
  const cases = [];
  const names = new Set();
  const named = (name) => {
    if (names.has(name)) throw new Error(`duplicate case ${name}`);
    names.add(name);
  };

  // Walk cases.
  const finishWalk = async (name, fields, input, { throwAt, tsTest: keys, deviation } = {}) => {
    named(name);
    const reply = await redactEvent({ event: input, ...(throwAt !== undefined && { throwAt }) });
    let result;
    if ("error" in reply) {
      result = { error: reply.error };
    } else {
      // An assigned `__proto__` leaves a non-plain object; a JSON round trip makes it plain.
      result = { output: deviation === "RD-24" ? JSON.parse(JSON.stringify(reply.value)) : reply.value };
    }
    if (deviation === "RD-24") check(!hasProtoKey(result.output), `${name}: TS drops the __proto__ key`);
    cases.push({
      name,
      ...fields,
      ...(throwAt !== undefined && { throw_at: throwAt }),
      ...result,
      calls: reply.calls,
      ...(keys && { ts_test: keys }),
      ...(deviation && { deviation }),
    });
    return reply;
  };
  const walk = (name, input, options) => finishWalk(name, { input }, input, options);
  const walkText = (name, inputText, options) =>
    finishWalk(name, { input_text: inputText }, JSON.parse(inputText), options);

  // Restore cases.
  const restore = async (name, original, replacement, { tsTest: keys, deviation } = {}) => {
    named(name);
    let output = await applyEventRedaction({ original, ...(replacement !== undefined && { replacement }) });
    if (deviation === "RD-24") output = JSON.parse(JSON.stringify(output));
    cases.push({
      name,
      original,
      ...(replacement !== undefined && { replacement }),
      output,
      ...(keys && { ts_test: keys }),
      ...(deviation && { deviation }),
    });
    return output;
  };

  // The redactEvent tests of redaction.test.ts that JSON can carry. A `new Date(...)` timestamp
  // is its ISO string; `new Date()` is TIMESTAMP.
  const ses = "ses_123000000000000000000000000";
  await walk(
    "test:basic",
    { sessionId: ses, userIntent: "sensitive user intent", timestamp: "2024-01-01T00:00:00.000Z" },
    { tsTest: tsTest("redactEvent", "should redact basic string fields") },
  );
  await walk(
    "test:protected-fields",
    {
      sessionId: ses,
      id: "evt_456",
      projectId: "proj_789",
      server: "my-server",
      serverName: "my-mcp-server",
      serverVersion: "1.2.3",
      clientName: "my-mcp-client",
      clientVersion: "4.5.6",
      identifyActorGivenId: "actor_123",
      identifyActorName: "John Doe",
      resourceName: "my-resource",
      eventType: "mcp:tools/call",
      actorId: "actor_456",
    },
    { tsTest: tsTest("redactEvent", "should not redact protected fields") },
  );
  await walk(
    "test:nested-strings",
    {
      sessionId: ses,
      parameters: { query: "sensitive query", options: { apiKey: "secret-key-123", timeout: 5000 } },
      response: { data: "sensitive response data", metadata: { source: "sensitive source" } },
    },
    { tsTest: tsTest("redactEvent", "should redact nested string fields") },
  );
  await walk(
    "test:arrays-of-strings",
    { sessionId: ses, parameters: { tags: ["sensitive1", "sensitive2", "sensitive3"], numbers: [1, 2, 3] } },
    { tsTest: tsTest("redactEvent", "should handle arrays of strings") },
  );
  await walk(
    "test:mixed-nested",
    {
      sessionId: ses,
      parameters: {
        users: [
          { name: "John", age: 30, email: "john@example.com" },
          { name: "Jane", age: 25, email: "jane@example.com" },
        ],
      },
    },
    { tsTest: tsTest("redactEvent", "should handle mixed nested structures") },
  );
  await walk(
    "test:identify-data",
    {
      sessionId: ses,
      identifyData: {
        email: "user@example.com",
        preferences: { theme: "dark", notifications: true },
        metadata: { source: "webapp", version: "1.0.0" },
      },
    },
    { tsTest: tsTest("redactEvent", "should handle complex identifyData object without redacting it") },
  );
  await walk(
    "test:error-object",
    {
      sessionId: ses,
      isError: true,
      error: {
        message: "Sensitive error message",
        code: "ERR_001",
        stack: "Error: Sensitive error message\n    at functionName (file.js:10:5)",
      },
    },
    { tsTest: tsTest("redactEvent", "should handle error objects") },
  );
  await walk(
    "test:deeply-nested-protected-name",
    {
      sessionId: ses,
      parameters: { nested: { deeply: { sessionId: "This should be redacted", data: "sensitive data" } } },
    },
    { tsTest: tsTest("redactEvent", "should handle deeply nested protected fields correctly") },
  );
  const failed = await walk(
    "test:redactor-throws",
    { sessionId: ses, userIntent: "sensitive data" },
    { throwAt: 1, tsTest: tsTest("redactEvent", "should handle custom redaction function errors gracefully") },
  );
  check("error" in failed, "test:redactor-throws: TS throws");
  await walk(
    "test:nested-in-protected",
    {
      sessionId: ses,
      identifyData: { nested: { deeply: { sensitive: "this should NOT be redacted", data: ["array", "of", "strings"] } } },
      otherData: { nested: { deeply: { sensitive: "this SHOULD be redacted" } } },
    },
    { tsTest: tsTest("redactEvent", "should not redact nested fields within any protected field") },
  );
  await walk(
    "test:all-fields",
    {
      id: "evt_123",
      projectId: "proj_123",
      sessionId: ses,
      actorId: "actor_123",
      eventId: "custom_evt_123",
      eventType: "mcp:tools/call",
      isError: false,
      error: { message: "error message" },
      resourceName: "resource_123",
      duration: 1000,
      timestamp: TIMESTAMP,
      userIntent: "user intent text",
      parameters: { param1: "value1" },
      response: { result: "success" },
      identifyActorGivenId: "given_id_123",
      identifyActorName: "Actor Name",
      identifyData: { key: "value" },
      server: "server_name",
    },
    { tsTest: tsTest("redactEvent", "should handle events with all possible fields") },
  );
  // "should preserve null and undefined values" without its undefined (RD-20).
  await walk("null-and-empty-string", { sessionId: ses, parameters: { nullValue: null, emptyString: "" } });

  // Each protected key at the top level, holding a string and holding nested strings; the same
  // names nested are not protected.
  const nest = { s: "x", list: ["y", { t: "z" }], n: 1 };
  await walk("protected:all-strings", Object.fromEntries(PROTECTED.map((key) => [key, `v-${key}`])));
  for (const key of PROTECTED) {
    await walk(`protected:${key}`, { [key]: nest, userIntent: "u" });
    await walk(`nested:${key}`, { parameters: { [key]: `v-${key}`, deep: { [key]: nest } }, response: [{ [key]: "r" }] });
  }
  // identifyActorData is the field the events carry; only identifyData is protected (K-04).
  await walk("identify-actor-data:strings", {
    sessionId: ses,
    identifyActorGivenId: "actor_1",
    identifyActorData: { email: "a@example.com", plan: { tier: "pro" }, list: ["x", 1] },
  });
  await walk("identify-actor-data:with-identify-data", { identifyData: { a: "kept" }, identifyActorData: { a: "redacted" } });
  await walk("identify-actor-data:string", { identifyActorData: "text" });

  // The top-level timestamp is a Date in TS and is kept; a nested one is a string and is redacted.
  // A top-level timestamp that is not a string is not a Date, so its strings are redacted.
  await walk("timestamp:top-level", { sessionId: ses, timestamp: TIMESTAMP, userIntent: "u" });
  await walk("timestamp:nested", {
    timestamp: TIMESTAMP,
    parameters: { timestamp: TIMESTAMP, list: [{ timestamp: "t" }] },
    response: { timestamp: "r" },
    error: { timestamp: "e" },
  });
  await walk("timestamp:not-a-date-string", { timestamp: "not a date", userIntent: "u" });
  await walk("timestamp:empty-string", { timestamp: "" });
  await walk("timestamp:object", { timestamp: { at: TIMESTAMP, list: ["x"] } });
  await walk("timestamp:array", { timestamp: ["x", "y"] });
  await walk("timestamp:number", { timestamp: 1736942400000, userIntent: "u" });
  await walk("timestamp:null", { timestamp: null });

  // TS checks the protected names where the path is "": at the event, and in an object reached
  // from it through "" keys only.
  await walk("empty-path:protected", { "": { sessionId: "s", tags: { k: "v" }, timestamp: "t", a: "y" }, b: "z" });
  await walk("empty-path:twice", { "": { "": { id: "i", properties: { p: "q" }, a: "y" }, a: "x" } });
  await walk("empty-path:string", { "": "s" });
  await walk("empty-path:array", { "": [{ id: "i" }, "s"] });
  await walk("empty-path:after-named-key", { x: { "": { id: "i" } }, sessionId: { "": "kept" } });
  await walk("empty-path:inside-protected", { tags: { "": { a: "kept" } } });

  // Index-like keys come first, in ascending order, in TS's walk: they change the call order.
  await walkText("order:index-keys", '{"sessionId":"s","b":"x","2":"y","a":"z","1":"w","10":"t","0":"u"}');
  await walkText("order:index-keys-nested", '{"parameters":{"z":"a","5":"b","y":{"k":"c","3":"d"},"4294967294":"e","1":"f"}}');
  await walkText("order:non-index-keys", '{"parameters":{"b":"a","01":"b","-1":"c","4294967295":"d","1.5":"e","a":"f"}}');
  await walkText("order:protected-after-index", '{"id":"i","7":"x","tags":{"2":"t","1":"u"},"1":"y"}');

  // Values: other types, empty containers, arrays, and characters.
  await walk("types:no-strings", { duration: 5, isError: false, parameters: { n: -0.5, t: true, z: null, e: {}, a: [] } });
  await walk("types:empty-event", {});
  await walk("types:empty-strings", { userIntent: "", parameters: { a: "", b: [""] } });
  await walk("types:characters", { parameters: { a: "é世😀", b: "\u0000\n\"\\", c: "😀x" } });
  await walk("array:nested", { parameters: [["a", ["b", { c: "d" }]], [], "e"] });
  await walk("array:response", { response: [{ type: "text", text: "t" }, "s", 1] });
  await walk("array:top-level-protected", { tags: ["a", { b: "c" }] });
  await walk("fields:unknown", { custom: "c", nested: { deep: "d" }, redactionFn: "r", eventRedactionFn: "e" });

  // The first error stops the walk; the calls are those before it and the one that threw.
  const three = { sessionId: ses, userIntent: "a", parameters: { b: "b", c: ["c", "d"] }, response: { e: "e" } };
  await walk("throw:at-1", three, { throwAt: 1 });
  await walk("throw:at-3", three, { throwAt: 3 });
  await walk("throw:at-last", three, { throwAt: 5 });
  await walk("throw:after-last", three, { throwAt: 6 });

  // RD-24: a `__proto__` own key. TS passes its strings to the redactor, then drops the key.
  const proto = (name, inputText) => walkText(name, inputText, { deviation: "RD-24" });
  await proto("proto:kept", '{"sessionId":"s","parameters":{"a":"1","__proto__":{"x":"y","z":["w"]},"b":"2"}}');
  await proto("proto:string-value", '{"parameters":{"__proto__":"text","b":"2"}}');
  await proto("proto:null-value", '{"parameters":{"b":"2","__proto__":null}}');
  await proto("proto:array-value", '{"parameters":{"__proto__":["x",{"c":"d"}],"b":"2"}}');
  await proto("proto:top-level", '{"userIntent":"u","__proto__":{"x":"y"},"b":"2"}');
  await proto("proto:in-protected", '{"tags":{"__proto__":{"x":"y"},"k":"v"},"userIntent":"u"}');
  await proto("proto:nested-twice", '{"parameters":[{"__proto__":{"deep":{"__proto__":"x"}},"k":"v"}]}');

  // Restore cases: the applyEventRedaction tests of redaction.test.ts that JSON can carry.
  const base = {
    id: "evt_original",
    sessionId: ses,
    projectId: "proj_789",
    eventType: "mcp:tools/call",
    timestamp: "2024-01-01T00:00:00.000Z",
    resourceName: "add_todo",
    parameters: { text: "sensitive text" },
    response: { content: "sensitive response" },
    userIntent: "sensitive intent",
  };
  await restore(
    "apply:test-sync-hook",
    base,
    { ...base, parameters: { text: "[SCRUBBED]" } },
    { tsTest: tsTest("applyEventRedaction", "should apply a synchronous hook in place") },
  );
  await restore(
    "apply:test-async-hook",
    base,
    { ...base, userIntent: "[SCRUBBED]" },
    { tsTest: tsTest("applyEventRedaction", "should apply an asynchronous hook in place") },
  );
  const droppedNull = await restore("apply:test-drop-null", base, null, {
    tsTest: tsTest("applyEventRedaction", "should report a drop and leave the event untouched when the hook returns null"),
  });
  const droppedUndefined = await restore("apply:test-drop-undefined", base, undefined, {
    tsTest: tsTest("applyEventRedaction", "should report a drop when the hook returns undefined"),
  });
  check(droppedNull === null && droppedUndefined === null, "a null or undefined hook result drops the event");
  await restore("apply:test-deletions", base, without(base, "response", "userIntent"), {
    tsTest: tsTest("applyEventRedaction", "should honor field deletions instead of resurrecting them"),
  });
  await restore(
    "apply:test-restore",
    base,
    {
      ...without(base, "timestamp"),
      id: "evt_forged",
      sessionId: "ses_forged000000000000000000000",
      projectId: "proj_forged",
      eventType: "forged:event",
    },
    { tsTest: tsTest("applyEventRedaction", "should restore system-managed fields if the hook changes or removes them") },
  );

  // Each restored key changed (it keeps its place), removed (it comes back at the end) and moved.
  for (const key of RESTORED) {
    await restore(`apply:change-${key}`, base, { ...base, [key]: "forged" });
    await restore(`apply:remove-${key}`, base, without(base, key));
    await restore(`apply:move-${key}-first`, base, { [key]: "forged", ...without(base, key) });
    await restore(`apply:only-${key}`, base, { [key]: "forged" });
  }
  await restore("apply:reorder-all", base, Object.fromEntries(Object.entries(base).reverse()));
  await restore("apply:empty-replacement", base, {});
  await restore("apply:remove-all-restored", base, without(base, ...RESTORED));
  await restore("apply:new-keys", base, { ...base, extra: { a: 1 }, userIntent: "x", more: [1] });
  await restore("apply:new-keys-only", base, { a: 1, b: "two" });
  // The original's own value is restored; a key the original lacks is removed.
  await restore("apply:original-without-project-id", without(base, "projectId"), { ...base, projectId: "proj_forged" });
  await restore("apply:original-without-restored", { userIntent: "u" }, { ...base });
  await restore("apply:original-null-values", { ...base, projectId: null, timestamp: null }, without(base, "projectId"));
  await restore("apply:original-timestamp-not-a-date", { ...base, timestamp: { at: 1 } }, without(base, "timestamp"));
  await restore("apply:original-without-timestamp", without(base, "timestamp"), { ...base, timestamp: TIMESTAMP });
  // The hook fields: TS sets `redactionFn` to the original's (none here, or a JSON value) before
  // the restored fields, and deletes `eventRedactionFn`.
  await restore("apply:internal-keys", base, { redactionFn: "r", ...base, eventRedactionFn: { a: 1 } });
  await restore("apply:internal-keys-null", base, { ...base, redactionFn: null, eventRedactionFn: null });
  await restore("apply:original-redaction-fn", { ...base, redactionFn: "o" }, without(base, "id", "timestamp"));
  await restore("apply:original-redaction-fn-in-place", { ...base, redactionFn: { a: 1 } }, { redactionFn: "h", ...without(base, "id") });
  await restore("apply:original-event-redaction-fn", { ...base, eventRedactionFn: "o" }, { ...base });
  await restore("apply:index-keys", base, { ...base, 1: "x", 0: "y" });
  // RD-24: Object.assign drops a top-level `__proto__` key of the hook's result; a nested one is
  // kept by the spread and the assign, as in Rust.
  await restore("apply:proto-top-level", base, JSON.parse('{"a":1,"__proto__":{"x":"y"},"id":"f"}'), { deviation: "RD-24" });
  await restore("apply:proto-top-level-string", base, JSON.parse('{"__proto__":"s","b":2}'), { deviation: "RD-24" });
  const nestedProto = await restore("apply:proto-nested-kept", base, JSON.parse('{"a":{"__proto__":{"x":"y"}},"b":2}'));
  check(hasProtoKey(nestedProto), "apply:proto-nested-kept: TS keeps a nested __proto__ key");

  // Random events and hook results.
  const rand = seeded(10);
  const word = (lo, hi) => text(rand, [..."abcé世😀 \"\\\n"], lo, hi);
  const key = () =>
    pick(rand, [() => pick(rand, PROTECTED), () => "timestamp", () => "", () => String(int(rand, 0, 12)), () => word(1, 4)])();
  const value = (d) => {
    const r = rand();
    if (d <= 0 || r < 0.4) return pick(rand, [word(0, 9), word(0, 3), int(rand, -5, 99), null, true, 2.5]);
    if (r < 0.65) return Array.from({ length: int(rand, 0, 4) }, () => value(d - 1));
    const o = {};
    for (let i = 0, n = int(rand, 0, 4); i < n; i++) o[key()] = value(d - 1);
    return o;
  };
  // One key in five is protected.
  const eventKey = () =>
    pick(rand, [
      () => pick(rand, PROTECTED),
      () => pick(rand, ["timestamp", "", "eventId"]),
      () => pick(rand, ["parameters", "response", "userIntent", "error", "identifyActorData"]),
      () => pick(rand, ["parameters", "response"]),
      () => pick(rand, ["parameters", "response"]),
    ])();
  const randomEvent = () => {
    const event = {};
    for (let i = 0, n = int(rand, 1, 8); i < n; i++) {
      const k = eventKey();
      event[k] = k === "timestamp" && rand() < 0.7 ? TIMESTAMP : value(3);
    }
    return event;
  };
  for (let i = 0; i < 20; i++) {
    const event = randomEvent();
    const calls = (await redactEvent({ event })).calls.length;
    const throwAt = rand() < 0.25 && calls > 0 ? int(rand, 1, calls) : undefined;
    await walk(`seeded-${i}`, event, { throwAt });
  }
  for (let i = 0; i < 10; i++) {
    await restore(`apply:seeded-${i}`, randomEvent(), rand() < 0.1 ? null : randomEvent());
  }

  return { cases };
}

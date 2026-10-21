// Vectors for `truncateEvent` of the TS SDK (src/modules/truncation.ts), computed by the TS function
// through the oracle function `oracle.truncateEvent`: the field limits, the frames cut, the response
// and the normalized fields, and the truncateEvent tests of truncation.test.ts that JSON can carry
// (the pathological-payload one is in truncate_size.mjs). The size guard's own cases are in
// truncate_size.mjs: one file cannot hold both within the 2 MiB cap (a cut emoji alone writes
// 32 771 code units as a `$utf16` marker).
//
// Case fields: input: the event, long strings as `$repeat` macros (TS runs on the expanded event),
// or input_text: its JSON text with such macros, when the key order of the text matters (a JS
// object puts array-index keys first); output: the truncated event, or error: String(error) when
// TS throws (the `value` and the `error` of the reply of `oracle.truncateEvent`).
// `deviation` marks a case whose TS output Rust does not give:
//   RD-25  an array `response` stays an array (TS spreads it into an object with index keys);
//   RD-26  an object whose `length` is at most the limit passes TS's limit; Rust gives the error;
//   RD-27  an object with an own `toString` key in a joined array throws in TS; Rust writes
//          "[object Object]";
//   RD-28  TS gives an `error` without `message` an own `message: undefined`, which counts toward
//          normalize's 100-key breadth; Rust counts only the keys JSON holds.
import { loadCore } from "oracle/bundle";

import { functions } from "../../functions.mjs";
import { expand } from "../../strict-json.mjs";
import { int, pick, seeded, text, tsKey } from "../support.mjs";

const truncate = functions["oracle.truncateEvent"];

/** A `$repeat` macro: `unit` repeated `count` times. */
export const rep = (unit, count) => ({ $repeat: [unit, count] });

/** The event of makeEvent in truncation.test.ts, with the timestamp as its ISO string. */
export const makeEvent = (overrides = {}) => ({
  id: "evt_1",
  sessionId: "ses_1",
  eventType: "mcp:tools/call",
  timestamp: "2025-01-15T12:00:00.000Z",
  ...overrides,
});

/** The UTF-8 size of JSON.stringify(value): what TS's size guard measures. */
export const bytes = (value) => Buffer.byteLength(JSON.stringify(value));

/** The key of a truncateEvent test of truncation.test.ts. */
export const tsTest = (group, title) => [tsKey("truncation", `truncateEvent - ${group} > ${title}`)];

/**
 * A case list, its `add(name, input, {tsTest, deviation})`, which runs TS on the expanded input,
 * stores the case and returns the reply of `oracle.truncateEvent`, and its `addText(name,
 * inputText, ...)`, the same for an input given as JSON text.
 */
export function collector() {
  const cases = [];
  const names = new Set();
  const finish = (name, fields, input, { tsTest: keys, deviation } = {}) => {
    if (names.has(name)) throw new Error(`duplicate case ${name}`);
    names.add(name);
    const reply = truncate(expand(input));
    const result = "error" in reply ? { error: reply.error } : { output: reply.value };
    cases.push({ name, ...fields, ...result, ...(keys && { ts_test: keys }), ...(deviation && { deviation }) });
    return reply;
  };
  const add = (name, input, options) => finish(name, { input }, input, options);
  const addText = (name, inputText, options) => finish(name, { input_text: inputText }, JSON.parse(inputText), options);
  return { cases, add, addText };
}

/** Throws with `message` unless `ok`. */
export function check(ok, message) {
  if (!ok) throw new Error(`truncate vectors: ${message}`);
}

/** `value` with each string of 100 or more copies of one ASCII character as a `$repeat` macro. */
function compress(value) {
  if (typeof value === "string") {
    return /^([\x00-\x7f])\1{99,}$/s.test(value) ? rep(value[0], value.length) : value;
  }
  if (Array.isArray(value)) return value.map(compress);
  if (value !== null && typeof value === "object") {
    return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, compress(v)]));
  }
  return value;
}

const frame = (i, withLine) => ({
  filename: `file${i}.ts`,
  function: `func${i}`,
  ...(withLine && { lineno: i + 1 }),
  in_app: true,
});
const rustFrames = (n) =>
  Array.from({ length: n }, (_, i) => ({ filename: `src/f${i}.rs`, function: `f${i}`, lineno: i + 1, in_app: i % 2 === 0 }));
const withFrames = (frames) => makeEvent({ error: { message: "m", type: "Error", platform: "rust", frames } });

// The items of the joined `userIntent` arrays: String() of each, `null` as "", nested arrays
// joined with ",", objects as "[object Object]", numbers as Number.prototype.toString writes them.
const INTENT_ITEMS = ["s", 7, null, 1.5, "é世😀", -0, 1e21, 0.1 + 0.2, true, 1e-7, "", -12];
const NESTED_ITEMS = [[1, [2, null], "x"], { a: 1 }, [[{ b: 1 }]], [], [null, [[]], ""]];
const intentArray = (n) =>
  Array.from({ length: n }, (_, i) => (i % 500 === 3 ? NESTED_ITEMS[(i - 3) / 500] : INTENT_ITEMS[i % INTENT_ITEMS.length]));

export async function generate() {
  const core = await loadCore();
  const { cases, add } = collector();

  // The truncateEvent tests of truncation.test.ts that JSON can carry. An undefined field is an
  // absent key.
  add("test:user-intent-3000", makeEvent({ userIntent: rep("x", 3000) }), {
    tsTest: tsTest("field-level string limits", "should truncate userIntent exceeding 2048 chars"),
  });
  add("test:resource-name-300", makeEvent({ resourceName: rep("t", 300) }), {
    tsTest: tsTest("field-level string limits", "should truncate resourceName exceeding 256 chars"),
  });
  add(
    "test:metadata-300",
    makeEvent({ serverName: rep("s", 300), serverVersion: rep("v", 300), clientName: rep("c", 300), clientVersion: rep("cv", 200) }),
    { tsTest: tsTest("field-level string limits", "should truncate serverName, serverVersion, clientName, clientVersion exceeding 256 chars") },
  );
  add("test:short-fields", makeEvent({ userIntent: "Get weather", resourceName: "fetch_weather", serverName: "my-server" }), {
    tsTest: tsTest("field-level string limits", "should leave short field values unchanged"),
  });
  add("test:absent-fields", makeEvent(), {
    tsTest: tsTest("field-level string limits", "should handle undefined/null fields gracefully"),
  });
  add("test:error-message-3000", makeEvent({ error: { message: rep("e", 3000), type: "Error" } }), {
    tsTest: tsTest("error field limits", "should truncate error.message exceeding 2048 chars"),
  });
  add("test:frames-80", makeEvent({ error: { message: "test", frames: Array.from({ length: 80 }, (_, i) => frame(i, true)) } }), {
    tsTest: tsTest("error field limits", "should limit error.frames to 50 (first 25 + last 25)"),
  });
  add("test:frames-50", makeEvent({ error: { message: "test", frames: Array.from({ length: 50 }, (_, i) => frame(i, false)) } }), {
    tsTest: tsTest("error field limits", "should leave frames at exactly 50 unchanged"),
  });
  add("test:error-without-frames", makeEvent({ error: { message: "test" } }), {
    tsTest: tsTest("error field limits", "should handle error without frames"),
  });
  add(
    "test:content-text-40000",
    makeEvent({ response: { content: [{ type: "text", text: rep("x", 40000) }, { type: "text", text: "short" }] } }),
    { tsTest: tsTest("response content text truncation", "should truncate text content blocks exceeding 32KB") },
  );
  add("test:small-event", makeEvent({ parameters: { query: "hello" }, response: { content: [{ type: "text", text: "world" }] } }), {
    tsTest: tsTest("size targeting", "should leave small events unchanged"),
  });
  // "should preserve timestamp as Date" checks a JS Date and has no case.
  const bigNested = {};
  let current = bigNested;
  for (let i = 0; i < 8; i++) {
    current.data = rep("x", 15000);
    current.next = {};
    current = current.next;
  }
  current.data = "leaf";
  add("test:depth-progressive", makeEvent({ parameters: bigNested }), {
    tsTest: tsTest("size targeting", "should reduce depth progressively for events exceeding 100KB"),
  });
  add("test:last-resort", makeEvent({ parameters: { data: rep("x", 120_000) } }), {
    tsTest: tsTest("size targeting", "should truncate largest string fields as last resort"),
  });
  add("test:minimal", makeEvent({}), { tsTest: tsTest("edge cases", "should handle empty event with minimal fields") });
  add(
    "test:already-small",
    makeEvent({
      userIntent: "Get weather",
      resourceName: "fetch_weather",
      parameters: { location: "SF" },
      response: { content: [{ type: "text", text: "65F" }] },
    }),
    { tsTest: tsTest("edge cases", "should pass through already-small events unchanged") },
  );
  add("test:null-payloads", makeEvent({ parameters: null, response: null, error: null }), {
    tsTest: tsTest("edge cases", "should handle event with null parameters and response"),
  });
  add(
    "test:sdk-fields",
    makeEvent({
      id: "evt_abc123",
      sessionId: "ses_xyz789000000000000000000000",
      projectId: "proj_test",
      eventType: "mcp:tools/call",
      duration: 342,
      sdkLanguage: "typescript",
      agentcatVersion: "0.1.12",
      ipAddress: "192.168.1.1",
      isError: false,
    }),
    { tsTest: tsTest("edge cases", "should preserve SDK-controlled fields exactly") },
  );
  // The input is what TS's sanitizeEvent makes of the test's event; truncateEvent runs on it.
  const unsanitized = makeEvent({
    userIntent: "x".repeat(3000),
    parameters: { imageData: `${"A".repeat(12000)}=`, query: "hello", nested: { deep: { value: "y".repeat(40000) } } },
    response: {
      content: [
        { type: "text", text: "z".repeat(40000) },
        { type: "image", data: "base64img", mimeType: "image/png" },
      ],
    },
  });
  add("test:after-sanitize", compress(core.sanitization.sanitizeEvent(unsanitized)), {
    tsTest: tsTest("integration with sanitization pipeline", "should work correctly after sanitizeEvent in the pipeline"),
  });

  // Appendix C of docs/research/01-ts-sdk-behavior.md.
  const appendix = add(
    "appendix-c",
    {
      id: "e1",
      sessionId: "s",
      eventType: "mcp:tools/call",
      timestamp: "2025-01-15T12:00:00.000Z",
      userIntent: rep("x", 3000),
      resourceName: rep("t", 300),
      parameters: { q: rep("y", 40000) },
      response: { content: [{ type: "text", text: rep("z", 40000) }] },
    },
  ).value;
  const lengths = [appendix.userIntent, appendix.resourceName, appendix.parameters.q, appendix.response.content[0].text].map((s) => s.length);
  check(lengths.join() === "2051,259,32771,32771" && bytes(appendix) === 68054, `appendix-c: ${lengths} ${bytes(appendix)}`);

  // Each limited field at its limit and one unit over; null fields; values that are not strings.
  const fields = [
    ["userIntent", "user-intent", 2048],
    ["resourceName", "resource-name", 256],
    ["serverName", "server-name", 256],
    ["serverVersion", "server-version", 256],
    ["clientName", "client-name", 256],
    ["clientVersion", "client-version", 256],
  ];
  for (const [key, slug, limit] of fields) {
    for (const n of [limit, limit + 1]) add(`field:${slug}-${n}`, makeEvent({ [key]: rep("a", n) }));
  }
  for (const n of [2048, 2049]) add(`field:error-message-${n}`, makeEvent({ error: { message: rep("m", n), type: "Error" } }));
  add("field:user-intent-emoji-at-2048", makeEvent({ userIntent: `${"x".repeat(2047)}😀tail` }));
  add("field:error-message-emoji-at-2048", makeEvent({ error: { message: `${"x".repeat(2047)}😀tail` } }));
  add("field:null", makeEvent(Object.fromEntries(fields.map(([key]) => [key, null]))));
  add("field:resource-name-number", makeEvent({ resourceName: 42 }));
  add("field:server-name-object", makeEvent({ serverName: {} }));
  add("field:client-version-array-300", makeEvent({ clientVersion: Array.from({ length: 300 }, (_, i) => i) }));
  add("field:error-message-number", makeEvent({ error: { message: 404, type: "Error" } }));
  add("field:error-message-absent", makeEvent({ error: { type: "Error", platform: "rust" } }));
  add("field:first-error-wins", makeEvent({ userIntent: 1, error: { message: "m", frames: true } }));

  // A non-string userIntent (the customer's `context` argument).
  add("intent:number", makeEvent({ userIntent: 5 }));
  add("intent:true", makeEvent({ userIntent: true }));
  add("intent:false", makeEvent({ userIntent: false }));
  add("intent:null", makeEvent({ userIntent: null }));
  add("intent:object", makeEvent({ userIntent: { a: 1 } }));
  add("intent:length-object", makeEvent({ userIntent: { length: 10 } }), { deviation: "RD-26" });
  add("intent:length-object-over", makeEvent({ userIntent: { length: 2049 } }));
  add("intent:array-2048", makeEvent({ userIntent: Array.from({ length: 2048 }, (_, i) => (i % 3 === 0 ? `s${i}` : i)) }));
  add("intent:array-2049", makeEvent({ userIntent: intentArray(2049) }));
  // Numbers, and one object: the one with an own `toString` key, which String() cannot convert.
  const withToString = Array.from({ length: 2049 }, (_, i) => (i === 16 ? { toString: "x" } : i));
  const thrown = add("intent:array-with-tostring-key", makeEvent({ userIntent: withToString }), { deviation: "RD-27" });
  check(thrown.error === "TypeError: Cannot convert object to primitive value", `RD-27 case: ${JSON.stringify(thrown)}`);
  // The RD-27 twin: the same array with {"a": 1} at index 16, which TS joins as "[object Object]".
  const withObject = withToString.map((item, i) => (i === 16 ? { a: 1 } : item));
  const joined = add("intent:array-with-object", makeEvent({ userIntent: withObject })).value.userIntent;
  check(joined.split(",")[16] === "[object Object]", `RD-27 twin: ${joined.slice(0, 80)}`);

  // The response: text blocks around the 32 768 limit, an emoji cut there, other shapes.
  add("content:text-32768", makeEvent({ response: { content: [{ type: "text", text: rep("c", 32768) }] } }));
  add("content:text-32769", makeEvent({ response: { content: [{ text: rep("c", 32769), type: "text", annotations: { a: 1 } }] } }));
  add("content:emoji-at-32768", makeEvent({ response: { content: [{ type: "text", text: `${"x".repeat(32767)}😀tail` }] } }));
  add("content:not-an-array", makeEvent({ response: { content: { type: "text", text: rep("c", 32769) } } }));
  add("content:other-block-types", makeEvent({
    response: {
      content: [
        { type: "image", data: rep("i", 32769), mimeType: "image/png" },
        { type: "resource", resource: { uri: "file:///a", text: "r" } },
        null,
        "text",
      ],
      structuredContent: { text: "s" },
      isError: false,
    },
  }));
  add("response:array", makeEvent({ response: [{ type: "text", text: "a" }, 2, rep("x", 32769)] }), { deviation: "RD-25" });
  add("response:string", makeEvent({ response: rep("x", 32769) }));
  add("response:number", makeEvent({ response: 3 }));

  // error.frames: an array around 50 entries, and the other values TS's frames cut meets.
  for (const n of [0, 49, 50, 51]) add(`frames:${n}`, withFrames(rustFrames(n)));
  add("frames:null", withFrames(null));
  add("frames:false", withFrames(false));
  add("frames:zero", withFrames(0));
  add("frames:empty-string", withFrames(""));
  add("frames:true", withFrames(true));
  add("frames:number", withFrames(7));
  add("frames:object", withFrames({ a: 1 }));
  add("frames:object-length-10", withFrames({ length: 10 }), { deviation: "RD-26" });
  add("frames:object-length-51", withFrames({ length: 51 }));
  add("frames:string-50", withFrames("f".repeat(50)));
  add("frames:string-51", withFrames(`${"a".repeat(25)}é${"b".repeat(25)}`));
  add("frames:string-emoji-at-head-cut", withFrames(`${"x".repeat(24)}😀${"y".repeat(30)}`));
  add("frames:string-emoji-at-tail-cut", withFrames(`${"x".repeat(30)}😀${"y".repeat(24)}`));

  // An error that is not an object: an array is spread into an object with index keys.
  add("error:array", makeEvent({ error: ["a", { message: "b" }] }));
  add("error:string", makeEvent({ error: rep("e", 32769) }));
  add("error:number", makeEvent({ error: 3 }));

  // An error without `message`: TS assigns it `message: undefined`, an own property that normalize
  // counts toward the 100-key breadth before it skips the undefined value (RD-28). With 100
  // entries TS adds the "..." marker; with 99 keys and a "..." key, TS writes the marker into that
  // key. 99 and 101 entries are the twins, where Rust gives TS's output.
  const marker = "[MaxProperties ~]";
  const keyed = (from, to) => Array.from({ length: to - from }, (_, i) => [`k${from + i}`, from + i]);
  const indexed = (n) => Array.from({ length: n }, (_, i) => i);
  const errorOf = (reply) => reply.value.error;
  for (const n of [99, 101]) {
    const error = errorOf(add(`error:${n}-keys-no-message`, makeEvent({ error: Object.fromEntries(keyed(0, n)) })));
    check((error["..."] === marker) === (n === 101) && !("message" in error), `error:${n}-keys-no-message`);
  }
  const hundred = errorOf(add("error:100-keys-no-message", makeEvent({ error: Object.fromEntries(keyed(0, 100)) }), { deviation: "RD-28" }));
  check(hundred["..."] === marker && Object.keys(hundred).length === 101, "error:100-keys-no-message");
  for (const n of [99, 101]) {
    const error = errorOf(add(`error:array-${n}`, makeEvent({ error: indexed(n) })));
    check((error["..."] === marker) === (n === 101), `error:array-${n}`);
  }
  const spread = errorOf(add("error:array-100", makeEvent({ error: indexed(100) }), { deviation: "RD-28" }));
  check(spread["..."] === marker && Object.keys(spread).length === 101, "error:array-100");
  const dotted = Object.fromEntries([...keyed(0, 50), ["...", "dots"], ...keyed(50, 99)]);
  const dots = errorOf(add("error:99-keys-plus-dots", makeEvent({ error: dotted }), { deviation: "RD-28" }));
  check(dots["..."] === marker && Object.keys(dots)[50] === "..." && Object.keys(dots).length === 100, "error:99-keys-plus-dots");

  // normalize on parameters, response, identifyActorData and error; tags and properties untouched.
  let deep = "leaf";
  for (let i = 0; i < 12; i++) deep = { n: deep };
  const wide = Object.fromEntries(Array.from({ length: 102 }, (_, i) => [`k${i}`, i]));
  add("normalize:fields", makeEvent({
    parameters: { request: deep, extra: wide },
    response: { content: Array.from({ length: 102 }, (_, i) => ({ type: "text", text: `t${i}` })) },
    identifyActorData: { profile: deep, long: rep("p", 32769) },
    error: { message: "m", data: deep },
  }));
  add("normalize:tags-and-properties-untouched", makeEvent({
    tags: { long: rep("t", 32769) },
    properties: { deep, wide },
  }));
  add("normalize:index-keys", makeEvent({ parameters: { b: 1, 10: 2, 2: 3, a: 4 }, properties: { z: 1, 1: 2 } }));

  // Random events, all well under the size limit.
  const rand = seeded(8);
  const word = (lo, hi) => text(rand, [..."abcé世😀 \"\\\n"], lo, hi);
  const near = (limit) => `${"x".repeat(limit - int(rand, 1, 6))}${word(0, 8)}`;
  const value = (d) => {
    const r = rand();
    if (d <= 0 || r < 0.4) return pick(rand, [word(0, 9), int(rand, -5, 99), null, true, false, 2.5]);
    if (r < 0.65) return Array.from({ length: int(rand, 0, 4) }, () => value(d - 1));
    const o = {};
    for (let i = 0, n = int(rand, 0, 4); i < n; i++) o[pick(rand, ["a", "b", "text", "type", "1", "0", "..."])] = value(d - 1);
    return o;
  };
  const block = () =>
    pick(rand, [
      () => ({ type: "text", text: word(0, 20) }),
      () => ({ type: "image", data: word(0, 20), mimeType: "image/png" }),
      () => ({ type: "resource", resource: { uri: "file:///x", text: word(0, 20) } }),
      () => ({ type: "resource_link", uri: "file:///y", name: word(1, 5) }),
      () => ({ type: "custom", value: value(2) }),
    ])();
  const maybe = (p, make) => (rand() < p ? make() : undefined);
  for (let i = 0; i < 20; i++) {
    const event = makeEvent({
      projectId: maybe(0.5, () => "proj_1"),
      duration: maybe(0.5, () => int(rand, 0, 5000)),
      resourceName: maybe(0.7, () => (rand() < 0.3 ? near(256) : word(1, 12))),
      parameters: maybe(0.8, () => ({ request: value(4), extra: value(3) })),
      response: maybe(0.8, () => ({ content: Array.from({ length: int(rand, 0, 4) }, block), ...(rand() < 0.3 && { structuredContent: value(3) }) })),
      userIntent: maybe(0.6, () => (rand() < 0.3 ? near(2048) : rand() < 0.1 ? int(rand, 0, 9) : word(0, 30))),
      isError: maybe(0.4, () => rand() < 0.5),
      error: maybe(0.4, () => ({
        message: rand() < 0.3 ? near(2048) : word(0, 30),
        type: "Error",
        platform: "rust",
        ...(rand() < 0.6 && { frames: rustFrames(int(rand, 0, 60)) }),
      })),
      identifyActorData: maybe(0.4, () => value(3)),
      serverName: maybe(0.6, () => (rand() < 0.3 ? near(256) : word(1, 12))),
      clientVersion: maybe(0.6, () => (rand() < 0.3 ? near(256) : word(1, 12))),
      tags: maybe(0.4, () => ({ env: word(1, 6) })),
      properties: maybe(0.4, () => value(3)),
    });
    for (const key of Object.keys(event)) if (event[key] === undefined) delete event[key];
    check(bytes(event) < 90_000, `seeded-${i} is ${bytes(event)} bytes`);
    add(`seeded-${i}`, event);
  }

  return { cases };
}

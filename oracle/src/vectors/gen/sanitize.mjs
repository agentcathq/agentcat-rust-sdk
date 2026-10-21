// Vectors for `sanitizeEvent` of the TS SDK (src/modules/sanitization.ts), computed by the TS
// function through the oracle function `sanitization.sanitizeEvent`: the content blocks of the
// response, the 10 240-unit base64 gate of the parameter walk, and the sanitizeEvent tests of
// sanitization.test.ts that JSON can carry.
//
// Case fields: input: the event, long strings as `$repeat` macros where a string is one repeated
// unit (TS runs on the expanded event), or input_text: its JSON text, when the input holds a
// `__proto__` own key; output: the sanitized event, or error: String(error) when TS throws.
// `deviation` marks a case whose TS output Rust does not give; the Rust test checks Rust against
// TS data of the case's twin, a case of this file:
//   RD-24  the parameter walk assigns each key, and assigning `__proto__` sets a prototype, so TS
//          drops the key; Rust keeps it. The twin is the same event with `_proto_` as the key:
//          Rust's output with the key renamed is TS's output for the twin.
//   RD-25  an array `response` stays an array; TS spreads it into an object with index keys (no
//          twin: the check compares the array with the object's values).
//   RD-27  a block `type` with an own `toString` key makes TS throw when it writes the message;
//          Rust writes "[object Object]". The twin has {"a": 1} as the `type`.
import { functions } from "../../functions.mjs";
import { expand } from "../../strict-json.mjs";
import { int, pick, seeded, text, tsKey } from "../support.mjs";
import { makeEvent, rep } from "./truncate.mjs";

const sanitize = functions["sanitization.sanitizeEvent"];

const TS_ERROR_TOSTRING = "TypeError: Cannot convert object to primitive value";

const tsTest = (group, title) => [tsKey("sanitization", `sanitizeEvent - ${group} > ${title}`)];

/** `n` units: "A"s, then `tail`. A `$repeat` macro when `tail` is empty. */
const longA = (n, tail = "") => (tail === "" ? rep("A", n) : `${"A".repeat(n - tail.length)}${tail}`);

/** makeLargeBase64 of sanitization.test.ts: "A" * (n - 1) + "=". */
const large = (n = 12000) => `${"A".repeat(n - 1)}=`;

/** `value` without any `key` key, at any depth. */
function dropKey(value, key) {
  if (Array.isArray(value)) return value.map((item) => dropKey(item, key));
  if (value !== null && typeof value === "object") {
    return Object.fromEntries(Object.entries(value).filter(([k]) => k !== key).map(([k, v]) => [k, dropKey(v, key)]));
  }
  return value;
}

/** Throws with `message` unless `ok`. */
function check(ok, message) {
  if (!ok) throw new Error(`sanitize vectors: ${message}`);
}

export async function generate() {
  const cases = [];
  const names = new Set();
  const finish = (name, fields, input, { tsTest: keys, deviation } = {}) => {
    if (names.has(name)) throw new Error(`duplicate case ${name}`);
    names.add(name);
    let result;
    try {
      let output = sanitize(expand(input));
      // An assigned `__proto__` leaves a non-plain object; a JSON round trip makes it plain.
      if (deviation === "RD-24") output = JSON.parse(JSON.stringify(output));
      result = { output };
    } catch (e) {
      result = { error: String(e) };
    }
    cases.push({ name, ...fields, ...result, ...(keys && { ts_test: keys }), ...(deviation && { deviation }) });
    return result;
  };
  const add = (name, input, options) => finish(name, { input }, input, options);
  const addText = (name, inputText, options) => finish(name, { input_text: inputText }, JSON.parse(inputText), options);
  const params = (name, parameters) => add(name, makeEvent({ parameters }));
  const response = (name, value, options) => add(name, makeEvent({ response: value }), options);
  const blocks = (name, ...content) => response(name, { content });
  const outputOf = (name) => cases.find((c) => c.name === name).output;

  // The tests of sanitization.test.ts that JSON can carry.
  add(
    "test:image-audio-text",
    makeEvent({
      response: {
        content: [
          { type: "text", text: "Hello world" },
          { type: "image", data: "base64imagedata...", mimeType: "image/png" },
          { type: "audio", data: "base64audiodata...", mimeType: "audio/wav" },
        ],
      },
    }),
    { tsTest: tsTest("response content blocks", "should replace image and audio blocks but keep text blocks") },
  );
  add("test:text-only", makeEvent({ response: { content: [{ type: "text", text: "First" }, { type: "text", text: "Second" }] } }), {
    tsTest: tsTest("response content blocks", "should leave text-only content unchanged"),
  });
  add(
    "test:resource-blob",
    makeEvent({
      response: {
        content: [{ type: "resource", resource: { uri: "file:///data.bin", blob: "base64blobdata...", mimeType: "application/octet-stream" } }],
      },
    }),
    { tsTest: tsTest("response content blocks", "should redact EmbeddedResource with blob") },
  );
  add(
    "test:resource-text",
    makeEvent({
      response: {
        content: [{ type: "resource", resource: { uri: "file:///readme.txt", text: "This is a text resource", mimeType: "text/plain" } }],
      },
    }),
    { tsTest: tsTest("response content blocks", "should pass through EmbeddedResource with text") },
  );
  add("test:unknown-type", makeEvent({ response: { content: [{ type: "video", data: "somestuff", mimeType: "video/mp4" }] } }), {
    tsTest: tsTest("response content blocks", "should redact unknown content types with type name in message"),
  });
  add("test:resource-link", makeEvent({ response: { content: [{ type: "resource_link", uri: "file:///some/resource", name: "My Resource" }] } }), {
    tsTest: tsTest("response content blocks", "should pass through resource_link unchanged"),
  });
  add("test:structured-content-base64", makeEvent({ response: { structuredContent: { data: large(), label: "some label" } } }), {
    tsTest: tsTest("response content blocks", "should redact large base64 inside structuredContent"),
  });
  add("test:no-content-array", makeEvent({ response: { result: "success" } }), {
    tsTest: tsTest("response content blocks", "should handle response without content array"),
  });
  add("test:null-response", makeEvent({ response: null }), {
    tsTest: tsTest("response content blocks", "should handle null and undefined response without error"),
  });
  add("response:absent", makeEvent());
  add("test:small-strings", makeEvent({ parameters: { name: "hello", query: "some query text" } }), {
    tsTest: tsTest("parameter scanning", "should leave small strings unchanged"),
  });
  add("test:base64-12000", makeEvent({ parameters: { imageData: large() } }), {
    tsTest: tsTest("parameter scanning", "should redact large base64 strings (>10KB)"),
  });
  add("test:non-base64-12000", makeEvent({ parameters: { longText: rep("Hello, world! This is NOT base64. ", Math.ceil(12000 / 34)) } }), {
    tsTest: tsTest("parameter scanning", "should leave large non-base64 strings unchanged"),
  });
  add("test:nested-base64", makeEvent({ parameters: { level1: { level2: { level3: { data: large() } } } } }), {
    tsTest: tsTest("parameter scanning", "should find and redact deeply nested large base64"),
  });
  add(
    "test:mixed-types",
    makeEvent({ parameters: { count: 42, active: true, name: "short string", binaryData: large(), tags: ["a", "b", "c"] } }),
    { tsTest: tsTest("parameter scanning", "should only redact large base64 in mixed-type parameters") },
  );
  add("test:base64-in-array", makeEvent({ parameters: { items: ["small string", large(), "another small"] } }), {
    tsTest: tsTest("parameter scanning", "should redact large base64 inside an array"),
  });
  add("test:size-gate-boundary", makeEvent({ parameters: { atGate: large(10240), belowGate: large(10239) } }), {
    tsTest: tsTest("parameter scanning", "should redact base64 string at exactly SIZE_GATE boundary (10240 chars)"),
  });
  add("test:null-parameters", makeEvent({ parameters: null }), {
    tsTest: tsTest("parameter scanning", "should handle null and undefined parameters without error"),
  });
  add("parameters:absent", makeEvent());
  add(
    "test:full-event",
    makeEvent({
      projectId: "proj_1",
      resourceName: "my-tool",
      parameters: { imageData: large(), query: "hello" },
      response: { content: [{ type: "text", text: "Result text" }, { type: "image", data: "base64img", mimeType: "image/png" }] },
    }),
    { tsTest: tsTest("integration", "should sanitize both parameters and response in a full event") },
  );
  // "should not mutate the original event" checks object identity in JS and has no case.

  // The gate: 10 239 and 10 240 units.
  for (const n of [10239, 10240, 10241]) params(`gate:a-${n}`, { v: longA(n) });
  params("gate:a-10239-padded", { v: longA(10239, "=") });
  params("gate:a-10240-padded", { v: longA(10240, "=") });
  params("gate:a-10240-double-padded", { v: longA(10240, "==") });
  params("gate:a-10239-double-padded", { v: longA(10239, "==") });
  params("gate:a-200000", { v: rep("A", 200000) });
  params("gate:alphabet", { v: rep("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/", 160) });

  // Line breaks are in the character class; a final newline after the padding is not (no `m` flag,
  // and `$` does not match before a final newline in JavaScript).
  params("lines:lf", { v: rep(`${"A".repeat(76)}\n`, 140) });
  params("lines:crlf", { v: rep(`${"A".repeat(76)}\r\n`, 140) });
  params("lines:only-newlines", { v: rep("\n", 10240) });
  params("lines:only-crlf", { v: rep("\r\n", 5120) });
  params("lines:trailing-lf", { v: longA(10240, "\n") });
  params("lines:trailing-lf-after-padding", { v: longA(10240, "=\n") });
  params("lines:lf-then-padding", { v: longA(10240, "\n=") });
  params("lines:crlf-before-padding", { v: longA(10240, "\r\n==") });
  params("lines:leading-lf", { v: `\n${"A".repeat(10239)}` });
  params("lines:lf-in-padding", { v: longA(10240, "=\n=") });

  // What the pattern rejects, and the odd strings it accepts.
  params("reject:url-safe-dash", { v: longA(10240, "-") });
  params("reject:url-safe-underscore", { v: `_${"A".repeat(10239)}` });
  params("reject:text-after-padding", { v: longA(10240, "=x") });
  params("reject:padding-in-middle", { v: `${"A".repeat(5000)}=${"A".repeat(5239)}` });
  params("reject:all-equals", { v: rep("=", 10240) });
  params("reject:space", { v: longA(10240, " ") });
  params("reject:tab", { v: longA(10240, "\t") });
  params("reject:non-ascii", { v: longA(10240, "é") });
  params("reject:astral-units", { v: longA(10240, "😀") });
  params("reject:non-base64-long", { v: rep("Hello, world! ", 800) });
  params("accept:one-letter-then-padding", { v: `A${"=".repeat(10239)}` });
  params("accept:one-digit-then-padding", { v: `7${"=".repeat(10239)}` });
  params("accept:plus-slash", { v: rep("+/", 5120) });

  // Where the walk goes: arrays, nested objects, a top-level string, other types, other fields.
  params("walk:top-level-string", rep("A", 11000));
  params("walk:top-level-short-string", "hello");
  params("walk:top-level-number", 42);
  params("walk:top-level-true", true);
  params("walk:top-level-false", false);
  params("walk:top-level-zero", 0);
  params("walk:top-level-empty-string", "");
  params("walk:top-level-array", ["x", rep("A", 11000), ["y", rep("B", 10240)], null, 1.5]);
  params("walk:empty-object", {});
  params("walk:empty-array", []);
  params("walk:nested", { a: [{ b: [{ c: rep("Q", 20000), d: "keep" }] }], e: null });
  params("walk:index-keys", { b: 1, 10: rep("A", 10240), 2: "x", a: 4 });
  params("walk:long-key-not-walked", { [`${"A".repeat(10240)}`]: "v" });
  add("fields:others-untouched", makeEvent({ userIntent: longA(10240), tags: { t: longA(10240) }, properties: { p: "x" } }));
  add("fields:extra-keys-and-order", { eventType: "mcp:tools/call", response: { content: [] }, id: "evt_1", parameters: { a: 1 }, extra: 1 });

  // Every block type. A block `type` that is not a string is written as String() writes it.
  blocks("block:text", { type: "text", text: "t", annotations: { a: 1 } });
  blocks("block:image", { type: "image", data: "d", mimeType: "image/png", annotations: { a: 1 } });
  blocks("block:audio", { type: "audio", data: "d", mimeType: "audio/wav" });
  blocks("block:resource-link", { type: "resource_link", uri: "u", name: "n", extra: [1, 2] });
  blocks("block:video", { type: "video", data: "d" });
  blocks("block:type-empty", { type: "", text: "t" });
  blocks("block:type-case", { type: "Text", text: "t" }, { type: "IMAGE" }, { type: "Resource" });
  blocks("block:type-quote", { type: 'a"b' });
  blocks("block:type-number", { type: 5, text: "t" });
  blocks("block:type-float", { type: 1.5 });
  blocks("block:type-zero", { type: 0 });
  blocks("block:type-true", { type: true });
  blocks("block:type-false", { type: false });
  blocks("block:type-null", { type: null, text: "t" });
  blocks("block:type-absent", { text: "t" });
  blocks("block:type-array", { type: [1, [2, null], "x"] });
  blocks("block:type-empty-array", { type: [] });
  blocks("block:type-array-null-items", { type: [null, null] });
  blocks("block:type-object", { type: { a: 1 } });
  blocks("block:type-empty-object", { type: {} });
  blocks("block:type-array-with-object", { type: ["x", { a: 1 }] });
  // RD-27: String() of an object with an own `toString` key throws in TS. The twin is
  // block:type-object, with {"a": 1} for the type.
  const thrown = response("block:type-tostring-key", { content: [{ type: { toString: 1 } }] }, { deviation: "RD-27" });
  check(thrown.error === TS_ERROR_TOSTRING, `RD-27 case: ${JSON.stringify(thrown)}`);
  const twin = outputOf("block:type-object").response.content[0].text;
  check(twin === '[unsupported content type "[object Object]" redacted - not supported by AgentCat]', `RD-27 twin: ${twin}`);

  // Blocks that are not objects stay; an array is an object in JavaScript, so its `type` is
  // undefined and it is replaced.
  blocks("block:string", "text");
  blocks("block:number", 7);
  blocks("block:null", null);
  blocks("block:true", true);
  blocks("block:array", [{ type: "image" }]);
  blocks("block:array-empty", []);
  blocks("block:mixed", "s", 1, null, [], { type: "text", text: "t" }, { type: "image" });

  // `resource` blocks: only an object `resource` with a `blob` key of any value is replaced.
  const resource = (name, value) => blocks(name, { type: "resource", resource: value });
  resource("resource:blob-string", { uri: "u", blob: "AAAA" });
  resource("resource:blob-null", { uri: "u", blob: null });
  resource("resource:blob-empty", { blob: "" });
  resource("resource:blob-zero", { blob: 0 });
  resource("resource:blob-false", { blob: false });
  resource("resource:blob-object", { blob: { a: 1 } });
  resource("resource:text", { uri: "u", text: "t" });
  resource("resource:text-and-blob", { uri: "u", text: "t", blob: "AAAA" });
  resource("resource:empty-object", {});
  resource("resource:string", "blob");
  resource("resource:empty-string", "");
  resource("resource:number", 5);
  resource("resource:zero", 0);
  resource("resource:true", true);
  resource("resource:null", null);
  resource("resource:array", [{ blob: "x" }]);
  resource("resource:blob-key-case", { Blob: "x" });
  blocks("resource:absent", { type: "resource" });
  blocks("resource:top-level-blob", { type: "resource", blob: "x" });

  // The response around the blocks.
  response("response:empty-object", {});
  response("response:content-empty", { content: [] });
  response("response:content-object", { content: { type: "image" } });
  response("response:content-string", { content: "image" });
  response("response:content-null", { content: null });
  response("response:string", "just text");
  response("response:number", 5);
  response("response:zero", 0);
  response("response:true", true);
  response("response:false", false);
  response("response:empty-string", "");
  response("response:content-and-structured", {
    isError: false,
    content: [{ type: "image" }],
    structuredContent: { img: rep("Z", 10240), small: "s", list: [rep("Z", 10239), rep("Y", 10240)] },
  });
  response("structured:array", { structuredContent: [rep("A", 10240), "x", { k: rep("A", 10240) }] });
  response("structured:string-kept", { structuredContent: longA(10240) });
  response("structured:null", { structuredContent: null });
  response("structured:number", { structuredContent: 5 });
  response("structured:true", { structuredContent: true });
  response("structured:empty-object", { structuredContent: {} });
  response("content:text-base64-kept", { content: [{ type: "text", text: longA(10240) }] });
  response("content:blob-base64", { content: [{ type: "resource", resource: { blob: rep("A", 11000) } }] });
  const eventText = (field, value) =>
    `{"id":"evt_1","sessionId":"ses_1","eventType":"mcp:tools/call","timestamp":"2025-01-15T12:00:00.000Z","${field}":${value}}`;
  addText("response:key-order", eventText("response", '{"isError":false,"structuredContent":{"a":1},"1":"x","content":[{"type":"image"}]}'));

  // RD-25: an array response. TS spreads it into an object with index keys and leaves the items.
  response("response:array", [{ type: "image", data: "x" }, "s", { type: "text", text: "t" }], { deviation: "RD-25" });
  response("response:array-empty", [], { deviation: "RD-25" });
  response("response:array-of-base64", [rep("A", 11000)], { deviation: "RD-25" });

  // RD-24: `__proto__` own keys. The walk drops them in TS (the assignment sets a prototype or, for
  // a value that is not an object, does nothing) and keeps them in Rust. The twin renames the key.
  const proto = (name, inputText) => {
    addText(name, inputText, { deviation: "RD-24" });
    const renamed = inputText.replaceAll('"__proto__"', '"_proto_"');
    check(renamed !== inputText, `${name}: has a __proto__ key`);
    addText(`${name}-twin`, renamed);
    check(
      JSON.stringify(dropKey(outputOf(`${name}-twin`), "_proto_")) === JSON.stringify(outputOf(name)),
      `${name}: TS drops the key and nothing else`,
    );
  };
  proto("proto:parameters", eventText("parameters", '{"a":1,"__proto__":{"x":"y"},"b":2}'));
  proto("proto:parameters-string", eventText("parameters", '{"__proto__":"text","b":2}'));
  proto("proto:parameters-null", eventText("parameters", '{"b":2,"__proto__":null}'));
  proto("proto:parameters-array-value", eventText("parameters", '{"__proto__":["x",{"c":1}],"b":2}'));
  proto("proto:parameters-nested", eventText("parameters", '{"a":[{"__proto__":{"deep":{"__proto__":{"x":1}}},"k":"v"}],"z":1}'));
  proto("proto:structured-content", eventText("response", '{"content":[],"structuredContent":{"__proto__":{"x":"y"},"b":2}}'));
  // Kept by TS too, so no deviation: the spread of the response and of the event, and a block or a
  // resource that is returned as it is.
  addText("proto:response-kept", eventText("response", '{"__proto__":{"x":"y"},"content":[{"type":"image"}]}'));
  addText("proto:block-kept", eventText("response", '{"content":[{"type":"text","text":"t","__proto__":{"x":1}}]}'));
  addText("proto:event-kept", '{"id":"evt_1","__proto__":{"x":"y"},"parameters":{"a":1}}');
  addText("proto:resource-kept", eventText("response", '{"content":[{"type":"resource","resource":{"__proto__":"s","text":"t"}}]}'));

  // Random events; a string is sometimes a long run of base64 characters.
  const rand = seeded(9);
  const word = (lo, hi) => text(rand, [..."abcé世😀 \"\\\n"], lo, hi);
  const unit = () => text(rand, [..."ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/\n\r"], 4, 64);
  const longString = () => {
    const u = unit();
    const body = u.repeat(Math.ceil(int(rand, 10230, 10260) / u.length));
    return pick(rand, [body, body, `${body}=`, `${body}==`, `${body}-`, `${body}=\n`]);
  };
  const value = (d) => {
    const r = rand();
    if (d <= 0 || r < 0.35) {
      if (rand() < 0.06) return longString();
      return pick(rand, [word(0, 9), int(rand, -5, 99), null, true, false, 2.5]);
    }
    if (r < 0.6) return Array.from({ length: int(rand, 0, 4) }, () => value(d - 1));
    const o = {};
    for (let i = 0, n = int(rand, 0, 4); i < n; i++) o[pick(rand, ["a", "b", "text", "type", "1", "0", "data"])] = value(d - 1);
    return o;
  };
  const block = () =>
    pick(rand, [
      () => ({ type: "text", text: word(0, 20) }),
      () => ({ type: "image", data: word(0, 20), mimeType: "image/png" }),
      () => ({ type: "audio", data: word(0, 20) }),
      () => ({ type: "resource", resource: { uri: "file:///x", text: word(0, 20) } }),
      () => ({ type: "resource", resource: { uri: "file:///x", blob: word(0, 20) } }),
      () => ({ type: "resource_link", uri: "file:///y", name: word(1, 5) }),
      () => ({ type: pick(rand, ["custom", "video", 5, null, true, ["a", 1], { a: 1 }]), value: value(2) }),
      () => ({ text: "no type" }),
      () => pick(rand, ["str", 3, null, false, []]),
    ])();
  const maybe = (p, make) => (rand() < p ? make() : undefined);
  for (let i = 0; i < 20; i++) {
    const seededEvent = makeEvent({
      projectId: maybe(0.5, () => "proj_1"),
      parameters: maybe(0.8, () => (rand() < 0.1 ? value(1) : { request: value(4), extra: value(3) })),
      response: maybe(0.8, () =>
        rand() < 0.1
          ? pick(rand, ["text", 5, true])
          : { content: Array.from({ length: int(rand, 0, 4) }, block), ...(rand() < 0.4 && { structuredContent: value(3) }) },
      ),
      userIntent: maybe(0.6, () => word(0, 30)),
      tags: maybe(0.4, () => ({ env: word(1, 6) })),
    });
    for (const key of Object.keys(seededEvent)) if (seededEvent[key] === undefined) delete seededEvent[key];
    add(`seeded-${i}`, seededEvent);
  }

  return { cases };
}

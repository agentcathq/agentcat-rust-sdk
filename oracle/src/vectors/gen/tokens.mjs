// Vectors for the token estimates of the TS SDK (src/modules/tokenEstimate.ts), computed by the
// TS functions themselves.
//
// Case name prefixes, each with its fields:
//   bytes:<n>        input: a byte count;                output: estimateTokens(n)
//   utf8:<label>     input: a well-formed string;        output: utf8ByteLength(input)
//   input:<label>    input: the raw arguments;           output: estimateInputTokens(input), null for undefined
//   output:<label>   input: the tool result;             output: estimateOutputTokens(input), null for undefined
//   divisor          output: TOKEN_ESTIMATE_BYTES_PER_TOKEN
//
// The TS tests that throw (a toJSON that throws, a circular value, a throwing getter) and the lone
// surrogate string cannot be held by a Rust serde_json::Value (RD-20), so no case carries them.
import { loadCore } from "oracle/bundle";
import { int, pick, seeded, text, tsKey } from "../support.mjs";

const IN = "estimateInputTokens (raw arguments) > ";
const OUT = "estimateOutputTokens (content blocks) > ";
const ts = (title) => [tsKey("tokenEstimate", title)];

const textBlock = (t) => ({ type: "text", text: t });
const ALPHABET = 'ab Z09"\\/<>&\u0000\u0009\u000A é世😀𝄞';

export async function generate() {
  const { tokenEstimate: te } = await loadCore();
  const cases = [];
  const names = new Set();
  const add = (name, input, output, tsTitle) => {
    if (names.has(name)) return;
    names.add(name);
    const c = { name, input, output: output === undefined ? null : output };
    if (tsTitle) c.ts_test = ts(tsTitle);
    cases.push(c);
  };
  const addInput = (label, input, tsPrefix) =>
    add(`input:${label}`, input, te.estimateInputTokens(input), tsPrefix && `${tsPrefix}${JSON.stringify(input)} -> ${te.estimateInputTokens(input)}`);
  const addOutput = (label, input, tsPrefix) =>
    add(`output:${label}`, input, te.estimateOutputTokens(input), tsPrefix && `${tsPrefix}${JSON.stringify(input)} -> ${te.estimateOutputTokens(input)}`);

  cases.push({ name: "divisor", output: te.TOKEN_ESTIMATE_BYTES_PER_TOKEN, ts_test: ts("estimateTokens > pins the divisor") });

  // estimateTokens
  for (const n of [0, 1, 7, 13, 4096, 7516192765]) {
    add(`bytes:${n}`, n, te.estimateTokens(n), `estimateTokens > bytes ${n} -> ${te.estimateTokens(n)} tokens`);
  }
  for (const n of [2 ** 53, 2 ** 31 - 1, 2 ** 31, 7516192764, 7516192766, 3, 4, 8, 14]) {
    add(`bytes:${n}`, n, te.estimateTokens(n));
  }

  // utf8ByteLength of well-formed strings
  for (const s of ["", "hello", "café", "你好", "😀"]) {
    add(`utf8:${JSON.stringify(s)}`, s, te.utf8ByteLength(s), `utf8ByteLength > ${JSON.stringify(s)} -> ${te.utf8ByteLength(s)} bytes`);
  }
  for (const s of ["\u007f", "\u0080", "߿", "ࠀ", "￿", "\u{10000}", "\u{10ffff}", "a😀b", "😀😀"]) {
    add(`utf8:${JSON.stringify(s)}`, s, te.utf8ByteLength(s));
  }

  // estimateInputTokens
  for (const input of [
    { q: "hello" },
    {},
    { name: "café" },
    { html: "<a>&</a>" },
    { t: "你好" },
    { ids: [1, 2, 3], opts: { deep: true, n: null } },
  ]) {
    addInput(JSON.stringify(input), input, IN);
  }
  add("input:null", null, undefined, `${IN}omits the field for absent arguments`);
  addInput("floats", { x: 1.0, y: 1e21, z: 0.1 });
  addInput("q-hello-world", { q: "hello world" });
  addInput("emoji", { q: "héllo 😀 世界" });
  addInput("negative-zero", { n: -0 });
  addInput("integer-keys", { b: 1, 10: 2, 2: 3, a: 4, 4294967295: 5, "01": 6 });
  addInput("string", "text");
  addInput("number", 7);
  addInput("boolean", false);
  addInput("array", [1, "a", null, { k: [] }]);
  addInput("empty-array", []);
  addInput("empty-string", "");
  addInput("big-number", 2 ** 64);
  addInput("long-string", { s: "x".repeat(1000) });
  addInput("control-characters", { s: "\u0000\u001f\u007f  " });

  // estimateOutputTokens: the cases of the TS test, then blocks of every type
  const img = { type: "image", data: "QUJD", mimeType: "image/png" };
  const res = (resource) => ({ type: "resource", resource });
  for (const response of [
    { content: [textBlock("hello world")] },
    { content: [textBlock("abcd"), textBlock("e")] },
    { content: [textBlock("")] },
    { content: [img] },
    { content: [res({ uri: "file:///a", text: "resource body" })] },
    { content: [res({ uri: "file:///a", blob: "QUJD" })] },
    { content: [textBlock("hi")], structuredContent: { big: "y".repeat(1000) } },
    { content: [textBlock("x".repeat(4096))] },
    { content: [textBlock("hi")], isError: true },
    { content: [], structuredContent: { result: "ok" } },
    { content: [] },
    { content: [], structuredContent: null },
    { content: [img], structuredContent: { result: "ok" } },
  ]) {
    addOutput(JSON.stringify(response).slice(0, 80) + (JSON.stringify(response).length > 80 ? `#${JSON.stringify(response).length}` : ""), response, OUT);
  }
  add("output:no-content-list", { result: "ok" }, te.estimateOutputTokens({ result: "ok" }), `${OUT}falls back to the whole response when there is no content list`);
  add("output:null", null, undefined, `${OUT}omits the field for an absent response`);
  const nonString = { content: [{ type: "text", text: 42 }, null, "str"] };
  add("output:non-string-text-blocks", nonString, te.estimateOutputTokens(nonString), `${OUT}ignores blocks whose text is not a string`);

  const more = {
    "doc-example-abcdefg": { content: [textBlock("abcdefg")] },
    "doc-example-structured": { content: [], structuredContent: { a: 1 } },
    "doc-example-foo-bar": { foo: "bar" },
    "doc-example-resources": { content: [res({ uri: "u", text: "1234567" }), res({ uri: "u", blob: "QUJD" })] },
    "mixed-blocks": { content: [textBlock("aé"), img, res({ uri: "u", text: "世😀" }), { type: "audio", data: "x" }] },
    "text-multibyte": { content: [textBlock("你好世界😀")] },
    "text-not-a-string-number": { content: [{ type: "text", text: 5 }] },
    "text-null": { content: [{ type: "text", text: null }] },
    "text-array": { content: [{ type: "text", text: ["a"] }] },
    "text-object": { content: [{ type: "text", text: { text: "a" } }] },
    "text-block-without-type": { content: [{ text: "abc" }] },
    "type-not-a-string": { content: [{ type: 1, text: "abc" }] },
    "type-case": { content: [{ type: "Text", text: "abc" }] },
    "resource-is-a-string": { content: [{ type: "resource", resource: "abc" }] },
    "resource-is-an-array": { content: [{ type: "resource", resource: ["abc"] }] },
    "resource-without-text": { content: [{ type: "resource", resource: { uri: "u" } }] },
    "resource-null": { content: [{ type: "resource", resource: null }] },
    "resource-text-not-a-string": { content: [{ type: "resource", resource: { text: 1 } }] },
    "resource-text-empty": { content: [{ type: "resource", resource: { text: "" } }] },
    "resource-with-top-level-text": { content: [{ type: "resource", text: "abc", resource: { uri: "u" } }] },
    "text-block-with-resource": { content: [{ type: "text", text: "ab", resource: { text: "cd" } }] },
    "structured-ignored-with-content": { content: [textBlock("ab")], structuredContent: [1, 2, 3] },
    "structured-empty-object": { content: [], structuredContent: {} },
    "structured-array": { content: [], structuredContent: [1, "two", null] },
    "structured-null-explicit": { content: [], structuredContent: null },
    "structured-string": { content: [], structuredContent: "héllo" },
    "structured-zero": { content: [], structuredContent: 0 },
    "structured-false": { content: [], structuredContent: false },
    "content-not-an-array-string": { content: "abc" },
    "content-null": { content: null },
    "content-object": { content: { 0: textBlock("abc"), length: 1 } },
    "content-null-with-structured": { content: null, structuredContent: { a: 1 } },
    "blocks-null-and-primitives": { content: [null, 1, "x", true, [], textBlock("abcd")] },
    "response-string": "some text",
    "response-number": 12345,
    "response-boolean": true,
    "response-array": [textBlock("abc")],
    "response-empty-array": [],
    "response-empty-object": {},
    "response-content-key-order": { isError: false, content: [textBlock("x".repeat(7))], meta: { a: 1 } },
    "response-floats": { a: 1.0, b: 1e21, c: 0.1 },
    "response-integer-keys": { b: 1, 10: 2, 2: 3, "01": 4 },
  };
  for (const [label, response] of Object.entries(more)) addOutput(label, response);

  // random inputs
  const rand = seeded(5);
  const randomValue = (depth) => {
    const kind = pick(rand, ["s", "n", "b", "z", "a", "o"]);
    if (depth === 0 || kind === "s") return text(rand, ALPHABET, 0, 12);
    if (kind === "n") return pick(rand, [0, 1, -1, 1.5, 0.1 + 0.2, 1e21, 1e-7, 2 ** 53, 123456789]);
    if (kind === "b") return rand() < 0.5;
    if (kind === "z") return null;
    if (kind === "a") return Array.from({ length: int(rand, 0, 3) }, () => randomValue(depth - 1));
    return Object.fromEntries(Array.from({ length: int(rand, 0, 3) }, () => [pick(rand, ["a", "b", "10", "2", "id", "text", "content"]), randomValue(depth - 1)]));
  };
  const randomBlock = () => {
    switch (int(rand, 0, 4)) {
      case 0: return textBlock(text(rand, ALPHABET, 0, 40));
      case 1: return res({ uri: "u", text: text(rand, ALPHABET, 0, 40) });
      case 2: return img;
      case 3: return randomValue(1);
      default: return { type: pick(rand, ["text", "resource"]), text: randomValue(1), resource: randomValue(1) };
    }
  };
  for (let i = 0; i < 10; i++) addInput(`seeded-${i}`, { arg: randomValue(3), other: randomValue(2) });
  for (let i = 0; i < 20; i++) {
    const response = { content: Array.from({ length: int(rand, 0, 5) }, randomBlock) };
    if (rand() < 0.5) response.structuredContent = randomValue(2);
    if (rand() < 0.3) response.isError = rand() < 0.5;
    addOutput(`seeded-${i}`, response);
  }

  return { cases };
}

// Vectors for the tag validation of the TS SDK (src/modules/validation.ts, validateTags), computed
// by the TS function itself together with the log lines it writes.
//
// Case fields: input: the tags object (or input_text: its JSON text, when the key order of the
// input matters: a JS object literal in this generator already moves array-index keys first);
// output: the validated object, null when TS returns null ("no tags"); logs: the log lines written
// (head stripped).
import { loadCore } from "oracle/bundle";
import { captureLogs } from "../../logs.mjs";

import { pick, seeded, text, tsKey } from "../support.mjs";

const T = "validateTags > ";
const ts = (title) => [tsKey("validation", `${T}${title}`)];

const KEY_CHARS = "abcXYZ019$_.:- ";
const SEEDED_KEYS = [...KEY_CHARS, "!", "é", "/", "世", "😀", "\n", "\t"];
const SEEDED_VALUES = ["a", "bc", "é", "世", "😀", "x\ny", "x\ry", " "];

export async function generate() {
  const core = await loadCore();
  const cases = [];
  const names = new Set();
  const run = (input) => captureLogs(core, () => core.validation.validateTags(input));
  const finish = async (name, fields, captured, tsTest) => {
    const { value, logs } = await captured;
    if (names.has(name)) throw new Error(`duplicate case ${name}`);
    names.add(name);
    cases.push({ name, ...fields, output: value ?? null, logs, ...(tsTest && { ts_test: ts(tsTest) }) });
  };
  const add = async (name, input, tsTest) => await finish(name, { input }, run(input), tsTest);
  // `inputText` is the JSON text of the tags; TS runs on JSON.parse of it.
  const addText = async (name, inputText, tsTest) => await finish(name, { input_text: inputText }, run(JSON.parse(inputText)), tsTest);

  // The tests of validation.test.ts.
  await add("test:pass-through", { env: "production", trace_id: "abc-123", region: "us-east-1" }, "should pass through valid tags unchanged");
  await add("test:empty", {}, "should return null for empty object");
  await add("test:invalid-key-chars", { valid_key: "value", "invalid!key": "value", "good.key": "value" }, "should drop keys with invalid characters");
  await add("test:long-key", { ["a".repeat(33)]: "value", short: "value" }, "should drop keys longer than 32 characters");
  await add("test:long-value", { key1: "a".repeat(201), key2: "short" }, "should drop values longer than 200 characters");
  await add("test:newline", { key1: "has\nnewline", key2: "clean" }, "should drop values containing newlines");
  await add("test:non-string", { key1: 123, key2: "valid" }, "should drop non-string values at runtime");
  await add("test:empty-key", { "": "empty-key-value", valid: "value" }, "should drop empty string keys");
  const sixty = {};
  for (let i = 0; i < 60; i++) sixty[`key${String(i).padStart(3, "0")}`] = `value${i}`;
  await add("test:sixty-entries", sixty, "should keep only first 50 entries when exceeding limit");
  await add("test:all-invalid", { "!!!": "value", "###": "value" }, "should return null when all entries are invalid");
  await add("test:special-chars", {
    "my.tag": "value", "my:tag": "value", "my-tag": "value", "my tag": "value", $ai_trace_id: "trace-1", my$key: "value",
  }, "should allow keys with periods, colons, dashes, spaces, and dollar signs");

  // The Appendix C example (docs/research/01-ts-sdk-behavior.md): an integer-like key comes first.
  await addText(
    "appendix-c",
    `{"ok":"v","":"e","a/b":"x","${"k".repeat(33)}":"v","n":5,"nl":"a\\nb","cr":"a\\rb","123":"first?"}`,
  );

  // The cap: 50 valid entries among invalid ones, and the order of the first 50.
  const mixed = {};
  for (let i = 0; i < 50; i++) {
    mixed[`ok${i}`] = `v${i}`;
    mixed[`bad!${i}`] = "x";
    mixed[`num${i}`] = i;
  }
  await add("cap:fifty-valid-among-invalid", mixed);
  const fiftyOne = {};
  for (let i = 0; i < 51; i++) fiftyOne[`k${i}`] = "v";
  await add("cap:fifty-one", fiftyOne);
  const fifty = {};
  for (let i = 0; i < 50; i++) fifty[`k${i}`] = "v";
  await add("cap:fifty", fifty);
  // Sixty string keys, then ten index-like keys: TS puts the index keys first, so they win.
  const orderEntries = [];
  for (let i = 0; i < 60; i++) orderEntries.push(`${JSON.stringify(`s${i}`)}:"v${i}"`);
  for (let i = 9; i >= 0; i--) orderEntries.push(`"${i * 7}":"i${i}"`);
  await addText("order:index-keys-after-string-keys", `{${orderEntries.join(",")}}`);
  await addText("order:index-key-above-u32", '{"b":"1","4294967296":"2","3":"3","4294967294":"4","a":"5"}');
  await addText("order:index-keys-not-canonical", '{"b":"1","01":"2","4294967295":"3","4294967294":"4","2":"5","a":"6","10":"7"}');

  // `__proto__` passes the key pattern; Object.fromEntries makes it an own key, so TS keeps it.
  await addText("key:proto", '{"a":"1","__proto__":"v","b":"2"}');

  // Each allowed special character, alone and in a key.
  for (const ch of "$_.:- ") await add(`key:allowed:${JSON.stringify(ch)}`, { [`a${ch}b`]: "v", [ch]: "w" });
  await add("key:digits-and-case", { "0123456789": "v", ABCxyz: "w" });
  await add("key:e-acute", { café: "v", ok: "w" });
  await add("key:cjk", { 世界: "v" });
  await add("key:emoji", { "😀": "v" });
  await add("key:newline", { "a\nb": "v" });
  await add("key:trailing-newline", { "a\n": "v" });
  await add("key:slash-and-bracket", { "a/b": "v", "a[0]": "w", "a@b": "x", "a,b": "y" });
  await add("key:32-chars", { ["k".repeat(32)]: "v" });
  await add("key:33-chars", { ["k".repeat(33)]: "v" });
  await add("key:32-chars-spaces", { [" ".repeat(32)]: "v" });
  await add("key:invalid-and-long", { ["!".repeat(40)]: "v" });

  // Value lengths in UTF-16 units.
  await add("value:200", { k: "v".repeat(200) });
  await add("value:201", { k: "v".repeat(201) });
  await add("value:empty", { k: "" });
  await add("value:100-emoji", { k: "😀".repeat(100) });
  await add("value:101-emoji", { k: "😀".repeat(101) });
  await add("value:199-plus-emoji", { k: `${"v".repeat(199)}😀` });
  await add("value:cr", { k: "a\rb" });
  await add("value:lf", { k: "a\nb" });
  await add("value:crlf", { k: "a\r\nb" });
  await add("value:line-separator", { k: "a b" });
  await add("value:paragraph-separator", { k: "a b" });
  await add("value:long-with-newline", { k: `${"v".repeat(201)}\n` });

  // Value types.
  await add("value:number", { k: 1 });
  await add("value:float", { k: 1.5 });
  await add("value:null", { k: null });
  await add("value:true", { k: true });
  await add("value:false", { k: false });
  await add("value:array", { k: ["a"] });
  await add("value:empty-array", { k: [] });
  await add("value:object", { k: { a: "b" } });
  await add("value:empty-object", { k: {} });
  await add("value:mixed", { a: 1, b: null, c: true, d: [], e: {}, f: "ok" });

  // Random inputs.
  const rand = seeded(6);
  for (let i = 0; i < 30; i++) {
    const input = {};
    const n = 1 + Math.floor(rand() * 12);
    for (let j = 0; j < n; j++) {
      const key = rand() < 0.7 ? text(rand, KEY_CHARS, 0, 40) : text(rand, SEEDED_KEYS, 0, 8);
      const kind = rand();
      input[key] = kind < 0.6 ? text(rand, SEEDED_VALUES, 0, 3).repeat(Math.floor(rand() * 90)) : pick(rand, [1, null, true, [], {}, "ok"]);
    }
    await add(`seeded-${i}`, input);
  }

  return { cases };
}

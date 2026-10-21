// The strict encoding of vector files and oracle replies: markers for values plain JSON loses, and an
// error with the path for values JSON cannot carry.
import assert from "node:assert/strict";
import { test } from "node:test";

import { encodeStrict, expand, isPlainNumber } from "../src/strict-json.mjs";

test("a string that is not well-formed UTF-16 becomes a $utf16 marker; others stay", () => {
  assert.deepEqual(encodeStrict({ s: "a\ud83d" }), { s: { $utf16: [97, 0xd83d] } });
  assert.deepEqual(encodeStrict(["\ude00b"]), [{ $utf16: [0xde00, 98] }]);
  assert.equal(encodeStrict("😀"), "😀");
  assert.equal(encodeStrict("é"), "é");
});

test("-0 and numbers that are not plain become $f64 markers; plain numbers stay", () => {
  assert.deepEqual(encodeStrict(-0), { $f64: "8000000000000000" });
  assert.deepEqual(encodeStrict(0.1 + 0.2), { $f64: "3fd3333333333334" });
  for (const n of [0.1, 1e21, 2 ** 53, -12, -1.5]) assert.equal(encodeStrict(n), n, String(n));
  for (const n of [1e23, 2 ** 53 + 2]) {
    const out = encodeStrict(n);
    assert.equal(typeof out?.$f64, "string", String(n));
    assert.match(out.$f64, /^[0-9a-f]{16}$/);
  }
});

test("values JSON cannot carry throw with their path", () => {
  for (const [value, message] of [
    [{ a: [1, undefined] }, "strict-json: $.a[1]: undefined"],
    [{ a: undefined }, "strict-json: $.a: undefined"],
    [[, 1], "strict-json: $[0]: undefined"],
    [{ f() {} }, "strict-json: $.f: function"],
    [Symbol(), "strict-json: $: symbol"],
    [1n, "strict-json: $: bigint"],
    [NaN, "strict-json: $: NaN"],
    [Infinity, "strict-json: $: Infinity"],
    [{ x: [-Infinity] }, "strict-json: $.x[0]: -Infinity"],
    [new Date(0), "strict-json: $: not a plain object (Date)"],
    [/x/, "strict-json: $: not a plain object (RegExp)"],
    [new Map(), "strict-json: $: not a plain object (Map)"],
    [{ b: Buffer.from("x") }, "strict-json: $.b: not a plain object (Buffer)"],
    [{ $utf16: [1] }, "strict-json: $: reserved key $utf16"],
    [{ n: { $f64: "8000000000000000" } }, "strict-json: $.n: reserved key $f64"],
    [{ $repeat: ["ab", -1] }, "strict-json: $: malformed $repeat macro"],
    [{ $repeat: ["ab", 1.5] }, "strict-json: $: malformed $repeat macro"],
    [{ $repeat: ["ab", 3], x: 1 }, "strict-json: $: malformed $repeat macro"],
    [{ $repeat: ["\ud83d", 3] }, "strict-json: $: malformed $repeat macro"],
    [{ "a b": { "10": [undefined] } }, 'strict-json: $["a b"]["10"][0]: undefined'],
    [{ [Symbol("k")]: 1 }, "strict-json: $: symbol key"],
    [{ ["\ud83d"]: 1 }, 'strict-json: $: key "\\ud83d" is not well-formed UTF-16'],
  ]) {
    assert.throws(() => encodeStrict(value), { message }, message);
  }
});

test("a well-formed $repeat macro passes unchanged", () => {
  assert.deepEqual(encodeStrict({ $repeat: ["ab", 3] }), { $repeat: ["ab", 3] });
  assert.deepEqual(encodeStrict({ s: { $repeat: ["😀", 0] } }), { s: { $repeat: ["😀", 0] } });
});

test("a __proto__ key stays an own key", () => {
  const out = encodeStrict(JSON.parse('{"__proto__": {"x": 1}, "a": 2}'));
  assert.equal(JSON.stringify(out), '{"__proto__":{"x":1},"a":2}');
  assert.equal(Object.getPrototypeOf(out), Object.prototype);
});

test("expand replaces every $repeat macro by its string", () => {
  assert.deepEqual(expand({ a: [{ $repeat: ["ab", 3] }] }), { a: ["ababab"] });
  assert.deepEqual(expand([{ $repeat: ["x", 0] }, 1, "s"]), ["", 1, "s"]);
  assert.equal(JSON.stringify(expand(JSON.parse('{"__proto__": {"$repeat": ["a", 2]}}'))), '{"__proto__":"aa"}');
});

test("isPlainNumber is true exactly where serde_json parses the JSON text to the same f64", () => {
  for (const n of [0, 12, 0.1, 1.5e-7, 1e21, 123456789012345, 2 ** 53, -(2 ** 53), -0.5, 1e22, 1e-22]) {
    assert.equal(isPlainNumber(n), true, String(n));
  }
  for (const n of [-0, 0.1 + 0.2, 1e23, 5e-324, 2 ** 53 + 2, 1e-23, NaN, Infinity]) {
    assert.equal(isPlainNumber(n), false, String(n));
  }
});

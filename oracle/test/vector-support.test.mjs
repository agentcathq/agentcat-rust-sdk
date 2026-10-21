// Helpers every vector generator shares: the parity key of a TS test, and a seeded random stream.
import assert from "node:assert/strict";
import { test } from "node:test";

import { int, pick, seeded, text, tsKey } from "../src/vectors/support.mjs";

test("tsKey builds the parity key of a v1 or a v2 TS test", () => {
  assert.equal(tsKey("constants", "a > b"), "v1|src/tests/constants.test.ts|a > b");
  assert.equal(tsKey("v2/hooks", "x"), "v2|src/tests/v2/hooks.test.ts|x");
});

test("two seeded streams with the same seed give the same values, all in [0, 1)", () => {
  const a = seeded(7);
  const b = seeded(7);
  const first = Array.from({ length: 5 }, () => a());
  assert.deepEqual(Array.from({ length: 5 }, () => b()), first);
  assert.notDeepEqual(Array.from({ length: 5 }, seeded(8)), first);
  for (const v of first) assert.ok(v >= 0 && v < 1, String(v));
});

test("pick, int and text draw from their range only", () => {
  const rand = seeded(1);
  const ints = new Set(Array.from({ length: 200 }, () => int(rand, 2, 4)));
  assert.deepEqual([...ints].sort(), [2, 3, 4]);
  for (let i = 0; i < 50; i += 1) {
    assert.ok(["a", "b"].includes(pick(rand, ["a", "b"])));
    const s = text(rand, "x😀", 1, 3);
    const chars = [...s];
    assert.ok(chars.length >= 1 && chars.length <= 3, s);
    assert.ok(chars.every((c) => c === "x" || c === "😀"), s);
  }
});

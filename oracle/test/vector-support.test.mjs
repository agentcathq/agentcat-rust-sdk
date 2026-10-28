// Helpers every vector generator shares: the parity key of a TS test, and a seeded random stream.
import assert from "node:assert/strict";
import { mock, test } from "node:test";

import { int, pick, seeded, text, tsKey, withWatchdog } from "../src/vectors/support.mjs";

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

test("mulberry32 gives its known answers: seed 1 starts 0.6270739405881613", () => {
  const rand = seeded(1);
  assert.deepEqual([rand(), rand(), rand()], [0.6270739405881613, 0.002735721180215478, 0.5274470399599522]);
  // A seed beyond 32 bits wraps, as `>>> 0` does.
  assert.equal(seeded(2 ** 32 + 1)(), 0.6270739405881613);
});

test("withWatchdog passes a value through and rejects a promise that does not settle, with the message", async () => {
  assert.equal(await withWatchdog(Promise.resolve(5), 1_000, "late"), 5);
  await assert.rejects(withWatchdog(Promise.reject(new Error("own")), 1_000, "late"), /own/);
  await assert.rejects(withWatchdog(new Promise(() => {}), 20, "oracle: nothing settled within 20 ms"), {
    message: "oracle: nothing settled within 20 ms",
  });
});

test("withWatchdog runs on the real clock, also while node:test mock timers are on", async (t) => {
  mock.timers.enable({ apis: ["setTimeout"] });
  t.after(() => mock.timers.reset());
  await assert.rejects(withWatchdog(new Promise(() => {}), 20, "real clock"), { message: "real clock" });
});

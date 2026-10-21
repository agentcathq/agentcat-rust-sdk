// Vectors for `normalize` of the TS SDK (src/modules/truncation.ts), computed by the TS function.
//
// Case fields: input (or input_text: its JSON text, when key order matters or the input holds a
// `__proto__` own key); limits: [depth, breadth, max_string] (absent: the TS defaults); output.
// `deviation: "RD-24"` marks a case whose TS output lacks the `__proto__` key that Rust keeps.
import { loadCore } from "oracle/bundle";

import { int, pick, seeded, text, tsKey } from "../support.mjs";

const ts = (group, title) => [tsKey("truncation", `normalize - ${group} > ${title}`)];

export async function generate() {
  const core = await loadCore();
  const cases = [];
  const names = new Set();
  const finish = (name, fields, input, limits, tsTest, deviation) => {
    if (names.has(name)) throw new Error(`duplicate case ${name}`);
    names.add(name);
    let output = core.truncation.normalize(input, ...(limits ?? []));
    // An assigned `__proto__` leaves a non-plain object; a JSON round trip makes it plain.
    if (deviation === "RD-24") output = JSON.parse(JSON.stringify(output));
    cases.push({
      name,
      ...fields,
      ...(limits && { limits }),
      output,
      ...(tsTest && { ts_test: tsTest }),
      ...(deviation && { deviation }),
    });
  };
  const add = (name, input, limits, tsTest) => finish(name, { input }, input, limits, tsTest);
  const addText = (name, inputText, limits, tsTest, deviation) =>
    finish(name, { input_text: inputText }, JSON.parse(inputText), limits, tsTest, deviation);

  // The tests of truncation.test.ts that JSON can carry.
  add("test:short-string", "hello", undefined, ts("string truncation", "should leave short strings unchanged"));
  add("test:long-string", "a".repeat(33000), undefined, ts("string truncation", "should truncate strings exceeding maxStringLength with '...'"));
  add("test:exact-string", "a".repeat(32768), undefined, ts("string truncation", "should leave strings at exactly maxStringLength unchanged"));
  add("test:number", 42, undefined, ts("non-serializable values", "should pass through numbers unchanged"));
  add("test:zero", 0, undefined);
  add("test:negative-float", -3.14, undefined);
  add("test:true", true, undefined, ts("non-serializable values", "should pass through booleans unchanged"));
  add("test:false", false, undefined);
  add("test:null", null, undefined, ts("non-serializable values", "should pass through null as null"));
  let deep = { value: "deep" };
  for (let i = 0; i < 15; i++) deep = { nested: deep };
  add("test:object-depth-5", deep, [5], ts("depth limiting", "should collapse objects beyond max depth to '[Object]'"));
  let deepArr = ["leaf"];
  for (let i = 0; i < 15; i++) deepArr = [deepArr];
  add("test:array-depth-3", deepArr, [3], ts("depth limiting", "should collapse arrays beyond max depth to '[Array]'"));
  add("test:object-depth-0", { a: 1 }, [0]);
  add("test:array-depth-0", [1, 2], [0], ts("depth limiting", "should handle depth=0 by collapsing top-level objects"));
  add("test:string-depth-0", "hello", [0]);
  add("test:number-depth-0", 42, [0], ts("depth limiting", "should not collapse primitives regardless of depth"));
  const wide = {};
  for (let i = 0; i < 150; i++) wide[`key${i}`] = i;
  add("test:wide-object", wide, [10, 5], ts("breadth limiting", "should limit object properties to maxBreadth"));
  add("test:wide-array", Array.from({ length: 150 }, (_, i) => i), [10, 5], ts("breadth limiting", "should limit array elements to maxBreadth"));
  add("test:within-breadth", { a: 1, b: 2, c: 3 }, [10, 100], ts("breadth limiting", "should leave objects within breadth limit unchanged"));
  add("test:same-object-twice", { a: { x: 1 }, b: { x: 1 } });

  // Appendix C: a 12-level object and a 102-element array.
  let twelve = "leaf";
  for (let i = 0; i < 12; i++) twelve = { n: twelve };
  add("appendix-c:twelve-levels", twelve);
  add("appendix-c:102-elements", Array.from({ length: 102 }, (_, i) => i));

  // Depth 0, 1 and 2.
  const nested = { a: [1, { b: [2, { c: 3 }] }], d: { e: [4] }, f: "s" };
  for (const d of [0, 1, 2, 3]) {
    add(`depth:object-${d}`, nested, [d]);
    add(`depth:array-${d}`, [nested, [nested], 1], [d]);
  }
  add("depth:empty-containers-0", [], [0]);
  add("depth:empty-containers-1", { a: [], b: {} }, [1]);

  // Breadth around the limit.
  for (const n of [99, 100, 101, 102]) {
    add(`breadth:array-${n}`, Array.from({ length: n }, (_, i) => i));
    const o = {};
    for (let i = 0; i < n; i++) o[`k${i}`] = i;
    add(`breadth:object-${n}`, o);
  }
  const dots = [];
  for (let i = 0; i < 120; i++) dots.push(i === 4 ? '"...":"mine"' : `"k${i}":${i}`);
  addText("breadth:dots-key", `{${dots.join(",")}}`);
  addText("breadth:dots-key-within-limit", '{"...":1,"a":2}', [10, 5, 100]);
  // Index-like keys come first in JS order, so they win the breadth limit.
  const idx = [];
  for (let i = 0; i < 12; i++) idx.push(`"s${i}":"v"`);
  for (let i = 9; i >= 0; i--) idx.push(`"${i * 3}":${i}`);
  addText("breadth:index-keys-after-string-keys", `{${idx.join(",")}}`, [10, 5, 100]);
  addText("breadth:index-keys-not-canonical", '{"b":1,"01":2,"4294967295":3,"4294967294":4,"2":5,"a":6,"10":7}', [10, 3, 100]);

  // String lengths, in UTF-16 units.
  add("string:32768", "x".repeat(32768));
  add("string:32769", "x".repeat(32769));
  add("string:emoji-at-limit", `${"x".repeat(32767)}😀tail`);
  add("string:emoji-just-inside", `${"x".repeat(32766)}😀tail`);
  add("string:emoji-exact-end", `${"x".repeat(32766)}😀`);
  add("string:key-not-cut", { ["k".repeat(40000)]: "v" });
  add("string:in-array", ["a".repeat(6), "a".repeat(5), "😀😀😀"], [5, 10, 5]);

  // Small limits on mixed values.
  const mixed = { s: "abcdefgh", n: 1, z: null, t: true, a: [1, 2, 3, 4, 5, [6, 7, 8]], o: { p: { q: { r: 1 } }, x: "yyyyyyy", y: 2, z: 3 } };
  add("small:mixed", mixed, [3, 2, 5]);
  add("small:mixed-deeper", mixed, [5, 4, 5]);
  add("small:zero-breadth", mixed, [3, 0, 5]);
  add("small:zero-string", mixed, [3, 10, 0]);

  // RD-24: TS drops the `__proto__` key (the assignment sets the prototype); Rust keeps it.
  addText("proto:kept", '{"a":1,"__proto__":{"x":"y"},"b":2}', undefined, undefined, "RD-24");
  addText("proto:kept-string", '{"__proto__":"text","b":2}', undefined, undefined, "RD-24");
  addText("proto:counts-toward-breadth", '{"__proto__":{"x":1},"a":1,"b":2,"c":3}', [10, 2, 100], undefined, "RD-24");

  // Random values with small limits.
  const rand = seeded(7);
  const word = () => text(rand, [..."abcé世", "😀"], 0, 9);
  const gen = (d) => {
    const r = rand();
    if (d <= 0 || r < 0.35) return pick(rand, [word(), int(rand, -5, 99), null, true, false, 1.5]);
    if (r < 0.65) return Array.from({ length: int(rand, 0, 5) }, () => gen(d - 1));
    const o = {};
    for (let i = 0, n = int(rand, 0, 5); i < n; i++) o[pick(rand, ["a", "b", "c", "d", "...", "1", "0", "zz"])] = gen(d - 1);
    return o;
  };
  for (let i = 0; i < 30; i++) add(`seeded-${i}`, gen(4), [int(rand, 0, 4), int(rand, 0, 4), int(rand, 0, 6)]);

  return { cases };
}

// Vectors for the size guard of `truncateEvent` (src/modules/truncation.ts): an event whose JSON is
// over 102 400 bytes gets its parameters, response, identifyActorData and error normalized again
// at depth 9 down to 1, and then its longest strings cut. Case fields as in truncate.mjs; the
// pathological-payload test of truncation.test.ts is here, where the 2 MiB cap leaves room for it.
// Every string is ASCII: no cut drops half a character (RD-22), so Rust measures the bytes TS
// measures.
import { expand } from "../../strict-json.mjs";
import { bytes, check, collector, makeEvent, rep, tsTest } from "./truncate.mjs";

const MAX_EVENT_BYTES = 102_400;

/** `leaf` inside `levels` objects with the one key `n`. */
const nest = (levels, leaf) => {
  let value = leaf;
  for (let i = 0; i < levels; i++) value = { n: value };
  return value;
};

/** The value at `levels` keys `n` down from `value`. */
const dig = (value, levels) => {
  let at = value;
  for (let i = 0; i < levels; i++) at = at.n;
  return at;
};

/**
 * The `remaining` budget left after the first last-resort round of TS's truncateLargestFields on
 * `event` (its normalized fields hold no string over 100 units, so normalizing them at depth 1
 * leaves those strings as they are), when a reduction under 10 skips a string in that round; else
 * null. Used only to pick an input; the output comes from TS.
 */
function firstRoundSkip(event) {
  const lengths = [];
  const walk = (v) => {
    if (typeof v === "string") {
      if (v.length > 100) lengths.push(v.length);
    } else if (v !== null && typeof v === "object") {
      Object.values(v).forEach(walk);
    }
  };
  walk(event);
  lengths.sort((a, b) => b - a);
  let remaining = bytes(event) - MAX_EVENT_BYTES + 200;
  for (const length of lengths) {
    if (remaining <= 0) break;
    const reduction = Math.min(remaining, Math.floor(length / 2));
    if (reduction < 10) return remaining;
    remaining -= reduction;
  }
  return null;
}

/** `count` strings of `length(i)` units in an object with keys k0, k1, .... */
const strings = (count, length) =>
  Object.fromEntries(Array.from({ length: count }, (_, i) => [`k${i}`, String.fromCharCode(97 + (i % 26)).repeat(length(i))]));

export async function generate() {
  const { cases, add, addText } = collector();
  const size = (event) => bytes(expand(event));

  // The pathological-payload test of truncation.test.ts.
  const fifty = Object.fromEntries(Array.from({ length: 50 }, (_, i) => [`key${i}`, rep("v", 3000)]));
  add("test:pathological", makeEvent({ parameters: fifty, response: { data: rep("r", 30000) } }), {
    tsTest: tsTest("size targeting", "should guarantee 100KB max for pathological payloads"),
  });

  // The limit itself: a filler string in properties (never normalized) makes the event exactly
  // 102 400 bytes (kept as it is) and 102 401 bytes (cut by the last resort).
  const filled = (n) => makeEvent({ properties: { filler: rep("f", n) } });
  const fill = MAX_EVENT_BYTES - size(filled(0));
  check(size(filled(fill)) === MAX_EVENT_BYTES, "size:102400");
  check(bytes(add("size:102400", filled(fill)).value) === MAX_EVENT_BYTES, "size:102400 is kept");
  add("size:102401", filled(fill + 1));

  // One byte over at depth 10 (and so at 9 too: nothing is deeper); the strings 9 levels down in
  // each normalized field go at depth 9.
  const deep9 = (pad) =>
    makeEvent({
      parameters: nest(9, { a: rep("a", 9000) }),
      response: { structuredContent: nest(8, { b: rep("b", 9000) }) },
      identifyActorData: nest(9, { c: rep("c", 9000) }),
      error: { message: "m", data: nest(8, { d: rep("d", 9000) }) },
      properties: { pad: rep("p", pad) },
    });
  const pad9 = MAX_EVENT_BYTES + 1 - size(deep9(0));
  const at9 = add("size:fits-at-depth-9", deep9(pad9)).value;
  check(size(deep9(pad9)) === MAX_EVENT_BYTES + 1 && dig(at9.parameters, 9) === "[Object]", "size:fits-at-depth-9");

  // Exactly 102 400 bytes at depth 5, far over at depth 6.
  const deep5 = (pad) =>
    makeEvent({ parameters: nest(5, { a: rep("a", 30000), b: rep("b", 30000) }), properties: { pad: rep("p", pad) } });
  const pad5 = MAX_EVENT_BYTES - bytes(makeEvent({ parameters: nest(5, "[Object]"), properties: { pad: "" } }));
  const at5 = add("size:fits-at-depth-5", deep5(pad5)).value;
  check(bytes(at5) === MAX_EVENT_BYTES && dig(at5.parameters, 5) === "[Object]", "size:fits-at-depth-5");

  // 102 401 bytes at every depth from 10 to 2; the strings one level down go at depth 1.
  const deep1 = (pad) =>
    makeEvent({
      parameters: { n: { a: rep("a", 20000) }, kept: "k" },
      response: { structuredContent: { t: rep("t", 20000) }, isError: false },
      properties: { pad: rep("p", pad) },
    });
  const pad1 = MAX_EVENT_BYTES + 1 - size(deep1(0));
  const at1 = add("size:fits-at-depth-1", deep1(pad1)).value;
  check(size(deep1(pad1)) === MAX_EVENT_BYTES + 1 && at1.parameters.n === "[Object]" && at1.response.structuredContent === "[Object]", "size:fits-at-depth-1");

  // The last resort: one huge string (three rounds); two strings of equal length, where the one
  // first in JS order (the index key "1", second in the text) gets the first cut (the sort is
  // stable); and strings of 101 to 120 units, where a round's budget runs below 10 and the rest
  // of the round skips every string.
  add("size:last-resort-one-huge", makeEvent({ properties: { blob: rep("h", 300_000) } }));
  const equalText = `${JSON.stringify(makeEvent()).slice(0, -1)},"properties":{"b":${JSON.stringify(rep("b", 80000))},"1":${JSON.stringify(rep("a", 80000))}}}`;
  const equal = addText("size:last-resort-two-equal", equalText).value;
  check(equal.properties["1"].length === 40003 && equal.properties.b.length > 40003, "size:last-resort-two-equal");
  let count = 820;
  const short = (n) => makeEvent({ properties: strings(n, (i) => 101 + (i % 20)) });
  while (bytes(short(count)) <= MAX_EVENT_BYTES || firstRoundSkip(short(count)) === null) count++;
  add("size:last-resort-short-strings", short(count));

  // Gives up: no string over 100 units (the oversized event comes back), and ten rounds that halve
  // one string each time while 100-unit strings keep the event over the limit.
  const hundreds = strings(3000, () => 100);
  check(bytes(add("size:no-candidate", makeEvent({ properties: hundreds })).value) > MAX_EVENT_BYTES, "size:no-candidate");
  const rounds = add("size:ten-rounds", makeEvent({ tags: { huge: rep("h", 200_000) }, properties: strings(1000, () => 100) })).value;
  check(bytes(rounds) > MAX_EVENT_BYTES && rounds.tags.huge.length < 400, `size:ten-rounds ${rounds.tags.huge.length}`);

  return { cases };
}

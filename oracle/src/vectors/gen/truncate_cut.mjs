// Vectors for a cut of the last resort of `truncateEvent` (src/modules/truncation.ts) that ends inside
// an emoji. Case fields as in truncate.mjs. The event holds strings of 200 UTF-16 units with an emoji
// at units 99 and 100; the round cuts each of the first strings in JS order by 100 units, to the high
// half of the emoji. TS keeps that half (a `$utf16` marker in the file), Rust drops the whole emoji
// (RD-22), and the vector compares them through assert_js_eq. The event is over the size limit by
// 1 000 to 1 200 bytes, so one round is enough and the lone half (6 bytes in TS, none in Rust) does
// not change the number of rounds. The cases are in a file of their own: the 2 MiB cap leaves
// truncate_size.json no room for them, because a string with a lone surrogate is written as a
// marker with one number per unit.
import { bytes, check, collector, makeEvent } from "./truncate.mjs";
import { expand } from "../../strict-json.mjs";

const MAX_EVENT_BYTES = 102_400;
const HIGH_SURROGATE = /[\ud800-\udbff]$/;

export async function generate() {
  const { cases, add } = collector();
  const emoji = `${"a".repeat(99)}\u{1F600}${"a".repeat(99)}`;
  const filled = (count) =>
    makeEvent({ properties: Object.fromEntries(Array.from({ length: count }, (_, i) => [`k${i}`, emoji])) });
  let count = 400;
  while (bytes(expand(filled(count))) <= MAX_EVENT_BYTES + 1000) count++;

  const reply = add("cut:emoji-straddles", filled(count)).value;
  const cutStrings = Object.values(reply.properties).filter((text) => text.endsWith("..."));
  const straddling = cutStrings.filter((text) => HIGH_SURROGATE.test(text.slice(0, -3)));
  check(bytes(reply) <= MAX_EVENT_BYTES, `cut:emoji-straddles: ${bytes(reply)} bytes`);
  check(straddling.length >= 8 && straddling.length < cutStrings.length, `cut:emoji-straddles: ${straddling.length} of ${cutStrings.length}`);
  check(cutStrings.slice(-1)[0].slice(0, -3).includes("\u{1F600}"), "cut:emoji-straddles: the last cut keeps its emoji");
  return { cases };
}

// The structural diff of normalized documents, shared by diff.mjs (two whole documents) and matrix.mjs
// (the published events of two tracked runs, decision C46).
//
// structuralDiff(a, b) lists one entry {path, a, b} per difference. Object keys compare unordered, so key
// order never shows; a key on one side only gives "<absent>" on the other. Arrays compare in order (content-
// block order is part of the contract); arrays of different length give one entry per element past the
// shorter one, "<missing>" on that side. Paths look like $.events[3].body.parameters.extra.<keys>.
//
// compareEvents(a, b, rulings) compares the `events` and `nonPublishRequests` of two normalized documents
// and gives each entry the RD ids of the rulings whose whole-path regex matches it and whose pinned values
// (`ts` for the first side `a`, `rust` for the second side `b`, `rustKeys`) allow its two values (`ruledBy`;
// each hit counts in the ruling's `matched`, see rulings.mjs).
import { clip, matchRulings } from "./rulings.mjs";

/** The parts of a normalized document that the event comparison reads. */
export const EVENT_PARTS = ["events", "nonPublishRequests"];

const kind = (v) => (Array.isArray(v) ? "array" : v === null ? "null" : typeof v);

/** One entry {path, a, b} per difference between two JSON values. */
export function structuralDiff(x, y, path = "$", out = []) {
  if (JSON.stringify(x) === JSON.stringify(y)) return out;
  const tx = kind(x);
  if (tx !== kind(y)) {
    out.push({ path, a: x, b: y });
    return out;
  }
  if (tx === "array") {
    for (let i = 0; i < Math.max(x.length, y.length); i++) {
      if (i >= y.length) out.push({ path: `${path}[${i}]`, a: x[i], b: "<missing>" });
      else if (i >= x.length) out.push({ path: `${path}[${i}]`, a: "<missing>", b: y[i] });
      else structuralDiff(x[i], y[i], `${path}[${i}]`, out);
    }
    return out;
  }
  if (tx === "object") {
    for (const k of new Set([...Object.keys(x), ...Object.keys(y)])) {
      if (!Object.hasOwn(x, k)) out.push({ path: `${path}.${k}`, a: "<absent>", b: y[k] });
      else if (!Object.hasOwn(y, k)) out.push({ path: `${path}.${k}`, a: x[k], b: "<absent>" });
      else structuralDiff(x[k], y[k], `${path}.${k}`, out);
    }
    return out;
  }
  out.push({ path, a: x, b: y });
  return out;
}

/** The events and non-publish requests of a normalized document (an absent part is an empty array). */
const eventParts = (doc) => Object.fromEntries(EVENT_PARTS.map((k) => [k, Array.isArray(doc?.[k]) ? doc[k] : []]));

/** One entry {path, a, b, ruledBy} per difference in the events and non-publish requests of `a` and `b`. */
export function compareEvents(a, b, rulings = []) {
  return structuralDiff(eventParts(a), eventParts(b)).map((d) => ({ ...d, ruledBy: matchRulings(rulings, d.path, d).map((r) => r.deviation) }));
}

/**
 * Writes one line per entry: `RULED <RD-xx,...> <path>: <a> != <b>` when a ruling matched, else
 * `<unruled><path>: <a> != <b>`. Returns {ruled, unexpected}.
 */
export function printDiffs(entries, write, unruled) {
  let ruled = 0;
  let unexpected = 0;
  for (const d of entries) {
    if (d.ruledBy.length > 0) {
      ruled += 1;
      write(`RULED ${d.ruledBy.join(",")} ${d.path}: ${clip(d.a)} != ${clip(d.b)}\n`);
    } else {
      unexpected += 1;
      write(`${unruled}${d.path}: ${clip(d.a)} != ${clip(d.b)}\n`);
    }
  }
  return { ruled, unexpected };
}

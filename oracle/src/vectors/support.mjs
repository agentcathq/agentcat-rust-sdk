// Helpers every vector generator shares.

/**
 * The key of a TS test in `parity/ts-tests.json`: `v1|src/tests/<file>.test.ts|<title>`, or
 * `v2|src/tests/v2/<name>.test.ts|<title>` when `file` is `v2/<name>`.
 */
export function tsKey(file, title) {
  const project = file.startsWith("v2/") ? "v2" : "v1";
  return `${project}|src/tests/${file}.test.ts|${title}`;
}

/** A deterministic random stream (mulberry32) with values in [0, 1), so that a rerun writes the same file. */
export function seeded(seed) {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** One item of `items`. */
export function pick(rand, items) {
  return items[Math.floor(rand() * items.length)];
}

/** An integer in [lo, hi]. */
export function int(rand, lo, hi) {
  return lo + Math.floor(rand() * (hi - lo + 1));
}

/** A string of lo to hi characters, each one an item of `alphabet` (a string splits into code points). */
export function text(rand, alphabet, lo, hi) {
  const chars = [...alphabet];
  return Array.from({ length: int(rand, lo, hi) }, () => pick(rand, chars)).join("");
}

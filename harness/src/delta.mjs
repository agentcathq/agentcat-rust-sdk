// The MCP-wire delta: what tracking changes on the wire, and whether TS and Rust change it alike.
//
// transcriptDelta(untracked, tracked) lists the ops that turn the untracked transcript into the tracked
// one: {op: "add" | "remove" | "change", path, value?}. Transcript entries pair by index. Objects compare
// by key. Arrays of equal length compare by index; arrays of different length align by the longest common
// subsequence under deep equality, so a prepended block is one `add` at its index in the tracked array.
// `add` and `change` carry the tracked value; `remove` carries none (the untracked value is the host's).
// Paths look like $.delta.transcript[4].result.content[0].
//
// documentDelta(untrackedDoc, trackedDoc) adds the tracked document's `violations` as `add` ops at
// $.delta.violations[<rule>]@<violation path> (stable: it does not depend on the array index), value =
// the violation object. The untracked document's violations are ignored (they are the host's).
//
// compareDeltas(ts, rust, rulings) returns one entry per path whose ops differ between the two sides.
//
// usage: node harness/src/delta.mjs <ts-untracked> <ts-tracked> <rust-untracked> <rust-tracked> [--rulings <file>]
//   the four files are normalized documents (driver output *.normalized.json). Output: `DELTA <path>: ts <x> != rust <y>`
//   per unruled difference, `RULED RD-xx ...` per ruled one, the ruling lines of diff.mjs, a JSON summary.
//   rulings: as in diff.mjs (harness/rulings.json for Rust against TS, harness/rulings-ts.json for TS against TS).
// Exit 0: every difference is ruled and every ruling matched something.
// Exit 1: an unruled difference, or a ruling that matched nothing.
// Exit 2: bad arguments or a malformed rulings file.
import { fileURLToPath } from "node:url";

import { clip, loadRulings, matchRulings, readJson, rulingLines } from "./rulings.mjs";

const kind = (v) => (Array.isArray(v) ? "array" : v === null ? "null" : typeof v);

/** A string that is equal for deeply equal JSON values, whatever the key order. */
function canon(v) {
  if (Array.isArray(v)) return `[${v.map(canon).join(",")}]`;
  if (v !== null && typeof v === "object") {
    return `{${Object.keys(v).sort().map((k) => `${JSON.stringify(k)}:${canon(v[k])}`).join(",")}}`;
  }
  return JSON.stringify(v) ?? "undefined";
}

const keyPath = (p, k) => (/^[A-Za-z0-9_$-]+$/.test(k) ? `${p}.${k}` : `${p}[${JSON.stringify(k)}]`);

/** Index pairs [i, j] of a longest common subsequence of two arrays, under deep equality. */
function lcs(u, t) {
  const cu = u.map(canon);
  const ct = t.map(canon);
  const table = Array.from({ length: u.length + 1 }, () => new Array(t.length + 1).fill(0));
  for (let i = u.length - 1; i >= 0; i--) {
    for (let j = t.length - 1; j >= 0; j--) {
      table[i][j] = cu[i] === ct[j] ? table[i + 1][j + 1] + 1 : Math.max(table[i + 1][j], table[i][j + 1]);
    }
  }
  const pairs = [];
  let i = 0;
  let j = 0;
  while (i < u.length && j < t.length) {
    if (cu[i] === ct[j]) pairs.push([i++, j++]);
    else if (table[i + 1][j] >= table[i][j + 1]) i++;
    else j++;
  }
  return pairs;
}

function walk(u, t, path, ops) {
  if (canon(u) === canon(t)) return;
  const ku = kind(u);
  if (ku !== kind(t)) return void ops.push({ op: "change", path, value: t });
  if (ku === "object") {
    for (const k of Object.keys(u)) if (!Object.hasOwn(t, k)) ops.push({ op: "remove", path: keyPath(path, k) });
    for (const k of Object.keys(t)) {
      if (!Object.hasOwn(u, k)) ops.push({ op: "add", path: keyPath(path, k), value: t[k] });
      else walk(u[k], t[k], keyPath(path, k), ops);
    }
    return;
  }
  if (ku === "array") {
    if (u.length === t.length) {
      u.forEach((x, i) => walk(x, t[i], `${path}[${i}]`, ops));
      return;
    }
    const matched = lcs(u, t);
    const inU = new Set(matched.map(([i]) => i));
    const inT = new Set(matched.map(([, j]) => j));
    u.forEach((_, i) => inU.has(i) || ops.push({ op: "remove", path: `${path}[${i}]` }));
    t.forEach((x, j) => inT.has(j) || ops.push({ op: "add", path: `${path}[${j}]`, value: x }));
    return;
  }
  ops.push({ op: "change", path, value: t });
}

export function transcriptDelta(untracked, tracked) {
  const ops = [];
  for (let i = 0; i < Math.max(untracked.length, tracked.length); i++) {
    const path = `$.delta.transcript[${i}]`;
    if (i >= untracked.length) ops.push({ op: "add", path, value: tracked[i] });
    else if (i >= tracked.length) ops.push({ op: "remove", path });
    else walk(untracked[i], tracked[i], path, ops);
  }
  return ops;
}

export function documentDelta(untracked, tracked) {
  const ops = transcriptDelta(untracked.transcript ?? [], tracked.transcript ?? []);
  for (const v of Array.isArray(tracked.violations) ? tracked.violations : []) {
    ops.push({ op: "add", path: `$.delta.violations[${v.rule}]@${v.path}`, value: v });
  }
  return ops;
}

const show = (ops) => (ops.length === 0 ? "<none>" : ops.map((o) => (Object.hasOwn(o, "value") ? `${o.op} ${clip(o.value)}` : o.op)).join("; "));

function byPath(ops) {
  const map = new Map();
  for (const o of ops) map.set(o.path, [...(map.get(o.path) ?? []), o]);
  return map;
}

/**
 * One entry per path whose ops differ: {path, ts, rust, ruledBy}. `ts` and `rust` are the ops as text;
 * `ruledBy` lists the RD ids of the rulings that match the path (each hit counts in the ruling's `matched`).
 */
export function compareDeltas(ts, rust, rulings = []) {
  const a = byPath(ts);
  const b = byPath(rust);
  const entries = [];
  for (const path of new Set([...a.keys(), ...b.keys()])) {
    const x = a.get(path) ?? [];
    const y = b.get(path) ?? [];
    if (canon(x) === canon(y)) continue;
    entries.push({ path, ts: show(x), rust: show(y), ruledBy: matchRulings(rulings, path).map((r) => r.deviation) });
  }
  return entries;
}

/** Writes the DELTA / RULED lines of `entries`; returns {ruled, unexpected}. */
export function printEntries(entries, write) {
  let ruled = 0;
  let unexpected = 0;
  for (const e of entries) {
    if (e.ruledBy.length > 0) {
      ruled += 1;
      write(`RULED ${e.ruledBy.join(",")} ${e.path}: ts ${e.ts} != rust ${e.rust}\n`);
    } else {
      unexpected += 1;
      write(`DELTA ${e.path}: ts ${e.ts} != rust ${e.rust}\n`);
    }
  }
  return { ruled, unexpected };
}

function main() {
  const usage = (message) => {
    if (message) process.stderr.write(`delta: ${message}\n`);
    process.stderr.write(
      "usage: node harness/src/delta.mjs <ts-untracked> <ts-tracked> <rust-untracked> <rust-tracked> [--rulings <file>]\n",
    );
    process.exit(2);
  };
  const args = process.argv.slice(2);
  const paths = args.slice(0, 4);
  const rest = args.slice(4);
  if (paths.length < 4 || paths.some((p) => p.startsWith("--"))) usage();
  let rulingsPath;
  if (rest.length > 0) {
    if (rest.length !== 2 || rest[0] !== "--rulings") usage();
    rulingsPath = rest[1];
  }
  const [tsU, tsT, rustU, rustT] = paths.map((p) => readJson(p, usage));
  const rulings = loadRulings(rulingsPath, usage);
  const entries = compareDeltas(documentDelta(tsU, tsT), documentDelta(rustU, rustT), rulings);
  const { ruled, unexpected } = printEntries(entries, (line) => process.stdout.write(line));
  for (const line of rulingLines(rulings)) process.stdout.write(line);
  const unused = rulings.filter((r) => r.matched === 0);
  process.stdout.write(`${JSON.stringify({ total: entries.length, ruled, unexpected, unusedRulings: unused.length })}\n`);
  process.exit(unexpected > 0 || unused.length > 0 ? 1 : 0);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) main();

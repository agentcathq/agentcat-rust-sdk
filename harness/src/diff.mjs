// Structural diff of two normalized runs. Object keys compare unordered;
// arrays compare in order (content-block order is part of the contract).
// Every violation of an ordered check (normalized `violations`) is a diff
// entry, on either side, even when both sides share it.
//
// usage: node harness/src/diff.mjs <a.normalized.json> <b.normalized.json> [--rulings <file>]
// rulings: JSON array of {"path": "<regex>", "reason": "...", "deviation": "RD-xx"}; a
// difference whose whole path matches a ruling's regex is allowed (each regex is
// anchored as ^(?:<regex>)$). The output gives each ruling's match count. A ruling
// may also carry "pending": true (a boolean): a provisional ruling the owner has not
// answered yet (R17); it allows differences like any other.
// Exit 0: every difference is ruled and every ruling matched something.
// Exit 1: an unruled difference, or a ruling that matched nothing.
// Exit 2: bad arguments or a malformed rulings file.
import { readFileSync } from "node:fs";

function usage(message) {
  if (message) process.stderr.write(`diff: ${message}\n`);
  process.stderr.write("usage: node harness/src/diff.mjs <a.normalized.json> <b.normalized.json> [--rulings <file>]\n");
  process.exit(2);
}

const args = process.argv.slice(2);
const [aPath, bPath, ...rest] = args;
if (!aPath || !bPath || aPath.startsWith("--") || bPath.startsWith("--")) usage();
let rulingsPath;
if (rest.length > 0) {
  if (rest.length !== 2 || rest[0] !== "--rulings") usage();
  rulingsPath = rest[1];
}

function readJson(path) {
  try {
    return JSON.parse(readFileSync(path, "utf8"));
  } catch (err) {
    return usage(`${path}: ${err.message}`);
  }
}

function loadRulings(path) {
  if (path === undefined) return [];
  const list = readJson(path);
  if (!Array.isArray(list)) usage(`${path}: must be a JSON array`);
  return list.map((r, i) => {
    const where = `${path}[${i}]`;
    if (r === null || typeof r !== "object" || Array.isArray(r)) usage(`${where}: must be an object`);
    const extra = Object.keys(r).filter((k) => !["path", "reason", "deviation", "pending"].includes(k));
    if (extra.length > 0) usage(`${where}: unknown key ${JSON.stringify(extra[0])}`);
    if (Object.hasOwn(r, "pending") && typeof r.pending !== "boolean") usage(`${where}: "pending" must be a boolean`);
    if (typeof r.path !== "string" || r.path === "") usage(`${where}: "path" must be a non-empty regex string`);
    if (typeof r.reason !== "string" || r.reason.trim() === "") usage(`${where}: "reason" must be a non-empty string`);
    if (typeof r.deviation !== "string" || !/^RD-\d+$/.test(r.deviation)) usage(`${where}: "deviation" must look like RD-<n>`);
    let re;
    try {
      new RegExp(r.path); // the pattern must be valid on its own, not only once wrapped
      re = new RegExp(`^(?:${r.path})$`);
    } catch (err) {
      usage(`${where}: bad regex: ${err.message}`);
    }
    return { ...r, re, matched: 0 };
  });
}

const a = readJson(aPath);
const b = readJson(bPath);
const rulings = loadRulings(rulingsPath);

const diffs = [];
const kind = (v) => (Array.isArray(v) ? "array" : v === null ? "null" : typeof v);
const has = (o, k) => Object.hasOwn(o, k);

function walk(x, y, p) {
  if (JSON.stringify(x) === JSON.stringify(y)) return;
  const tx = kind(x);
  const ty = kind(y);
  if (tx !== ty) return void diffs.push({ path: p, a: x, b: y });
  if (tx === "array") {
    if (x.length !== y.length) diffs.push({ path: `${p}.length`, a: x.length, b: y.length });
    for (let i = 0; i < Math.min(x.length, y.length); i++) walk(x[i], y[i], `${p}[${i}]`);
    return;
  }
  if (tx === "object") {
    for (const k of new Set([...Object.keys(x), ...Object.keys(y)])) {
      if (!has(x, k)) diffs.push({ path: `${p}.${k}`, a: "<absent>", b: y[k] });
      else if (!has(y, k)) diffs.push({ path: `${p}.${k}`, a: x[k], b: "<absent>" });
      else walk(x[k], y[k], `${p}.${k}`);
    }
    return;
  }
  diffs.push({ path: p, a: x, b: y });
}

const strip = ({ violations, ...doc }) => doc;
walk(strip(a), strip(b), "$");

// Ordered-check violations: one entry per (rule, path), on either side.
const violationKey = (v) => `${v.path}.<${v.rule}>`;
const describe = ({ rule, path, ...detail }) => ({ violated: rule, ...detail });
const violations = new Map();
for (const [side, doc] of [["a", a], ["b", b]]) {
  for (const v of Array.isArray(doc.violations) ? doc.violations : []) {
    const key = violationKey(v);
    const entry = violations.get(key) ?? { path: key, a: "<ok>", b: "<ok>" };
    entry[side] = describe(v);
    violations.set(key, entry);
  }
}
diffs.push(...violations.values());

const clip = (v) => {
  const text = JSON.stringify(v) ?? "undefined";
  return text.length > 120 ? `${text.slice(0, 117)}...` : text;
};
let unexpected = 0;
let ruled = 0;
for (const d of diffs) {
  const hits = rulings.filter((r) => r.re.test(d.path));
  for (const r of hits) r.matched += 1;
  if (hits.length > 0) {
    ruled += 1;
    process.stdout.write(`RULED ${hits.map((r) => r.deviation).join(",")} ${d.path}: ${clip(d.a)} != ${clip(d.b)}\n`);
  } else {
    unexpected += 1;
    process.stdout.write(`DIFF  ${d.path}: ${clip(d.a)} != ${clip(d.b)}\n`);
  }
}
for (const r of rulings) {
  const what = r.matched === 0 ? "UNUSED RULING" : "RULING";
  process.stdout.write(`${what} ${r.deviation} matched ${r.matched}: ${JSON.stringify(r.path)} (${r.reason})\n`);
}
const unused = rulings.filter((r) => r.matched === 0);
process.stdout.write(`${JSON.stringify({ total: diffs.length, ruled, unexpected, unusedRulings: unused.length })}\n`);
process.exit(unexpected > 0 || unused.length > 0 ? 1 : 0);

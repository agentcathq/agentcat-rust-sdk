// Structural diff of two normalized runs. Object keys compare unordered;
// arrays compare in order (content-block order is part of the contract).
// usage: node diff.mjs <a.normalized.json> <b.normalized.json> [--allow <file>]
// --allow: JSON list of {path, reason} entries for ruled differences; any
// unlisted difference fails (exit 1).
import { readFileSync } from "node:fs";

const [aPath, bPath, flag, allowPath] = process.argv.slice(2);
const a = JSON.parse(readFileSync(aPath, "utf8"));
const b = JSON.parse(readFileSync(bPath, "utf8"));
const allow = flag === "--allow" ? JSON.parse(readFileSync(allowPath, "utf8")) : [];

const diffs = [];
function walk(x, y, p) {
  if (JSON.stringify(x) === JSON.stringify(y)) return;
  const tx = Array.isArray(x) ? "array" : x === null ? "null" : typeof x;
  const ty = Array.isArray(y) ? "array" : y === null ? "null" : typeof y;
  if (tx !== ty) return diffs.push({ path: p, a: x, b: y });
  if (tx === "array") {
    if (x.length !== y.length) diffs.push({ path: `${p}.length`, a: x.length, b: y.length });
    for (let i = 0; i < Math.min(x.length, y.length); i++) walk(x[i], y[i], `${p}[${i}]`);
    return;
  }
  if (tx === "object") {
    for (const k of new Set([...Object.keys(x), ...Object.keys(y)])) {
      if (!(k in x)) diffs.push({ path: `${p}.${k}`, a: "<absent>", b: y[k] });
      else if (!(k in y)) diffs.push({ path: `${p}.${k}`, a: x[k], b: "<absent>" });
      else walk(x[k], y[k], `${p}.${k}`);
    }
    return;
  }
  diffs.push({ path: p, a: x, b: y });
}
walk(a, b, "$");

const isAllowed = (d) => allow.some((r) => new RegExp(r.path).test(d.path));
const unexpected = diffs.filter((d) => !isAllowed(d));
const clip = (v) => (typeof v === "string" && v.length > 120 ? v.slice(0, 117) + "..." : v);
for (const d of diffs) {
  console.log(`${isAllowed(d) ? "ALLOWED " : "DIFF    "} ${d.path}: ${JSON.stringify(clip(d.a))} != ${JSON.stringify(clip(d.b))}`);
}
console.log(JSON.stringify({ total: diffs.length, unexpected: unexpected.length }));
process.exit(unexpected.length ? 1 : 0);

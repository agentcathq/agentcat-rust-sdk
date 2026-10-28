// Structural diff of two normalized runs (structuralDiff in events.mjs). Object
// keys compare unordered; arrays compare in order (content-block order is part
// of the contract). Every violation of an ordered check (normalized
// `violations`) is a diff entry, on either side, even when both sides share it.
//
// usage: node harness/src/diff.mjs <a.normalized.json> <b.normalized.json> [--rulings <file>]
// rulings: JSON array of {"path": "<regex>", "reason": "...", "deviation": "RD-xx"}; a
// difference whose whole path matches a ruling's regex is allowed (each regex is
// anchored as ^(?:<regex>)$). The output gives each ruling's match count. A ruling
// may also carry "pending": true (a boolean): a provisional ruling the owner has not
// answered yet (R17); it allows differences like any other, and its line reads
// "RULING (pending) <RD-xx> matched <n>". A ruling's "scenario", "transports" and "lanes" (matrix.mjs
// scopes by them) are ignored here: a document carries no combination, so every ruling matches by path
// alone. Its pinned values hold here as in the event comparison: "ts" must equal the value of <a>, "rust"
// the value of <b>, and with "rustKeys" every key of <b> must match (rulings.mjs).
// Exit 0: every difference is ruled and every ruling matched something.
// Exit 1: an unruled difference, or a ruling that matched nothing.
// Exit 2: bad arguments or a malformed rulings file.
import { printDiffs, structuralDiff } from "./events.mjs";
import { loadRulings, matchRulings, readJson, rulingLines } from "./rulings.mjs";

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

const a = readJson(aPath, usage);
const b = readJson(bPath, usage);
const rulings = loadRulings(rulingsPath, usage);

const strip = ({ violations, ...doc }) => doc;
const diffs = structuralDiff(strip(a), strip(b));

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

const entries = diffs.map((d) => ({ ...d, ruledBy: matchRulings(rulings, d.path, d).map((r) => r.deviation) }));
const { ruled, unexpected } = printDiffs(entries, (line) => process.stdout.write(line), "DIFF  ");
for (const line of rulingLines(rulings)) process.stdout.write(line);
const unused = rulings.filter((r) => r.matched === 0);
process.stdout.write(`${JSON.stringify({ total: diffs.length, ruled, unexpected, unusedRulings: unused.length })}\n`);
process.exit(unexpected > 0 || unused.length > 0 ? 1 : 0);

// Compares freshly generated oracle output with the committed files, for the nightly TS drift sweep.
//   node oracle/src/drift/compare.mjs <vectors dir> <parity dir>
// <vectors dir> holds the output of `vectors/run.mjs --out`; <parity dir> holds `ts-tests.json` from
// `parity/list-tests.mjs --out`. They are compared with agentcat-sdk/tests/vectors/ and parity/.
// Only the `cases` arrays of the vector files and the `tests` array of ts-tests.json count (R-6):
// `ts_sha`, `count` and MANIFEST.sha256 differ by design when the nightly builds TS `main`.
// Prints a Markdown summary and exits 1 on any difference; exits 2 on bad arguments.
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const MAX_LISTED = 20;

function readArray(file, field) {
  if (!existsSync(file)) return null;
  const doc = JSON.parse(readFileSync(file, "utf8"));
  if (!Array.isArray(doc[field])) throw new Error(`${file}: "${field}" is not an array`);
  return doc[field];
}

function jsonFiles(dir) {
  return existsSync(dir) ? readdirSync(dir).filter((f) => f.endsWith(".json")) : [];
}

/** Items of `committed` and `fresh` (arrays of JSON values) that the other side lacks, by `id(item)`. */
function compareItems(committed, fresh, id) {
  const index = (items) => new Map(items.map((item, i) => [id(item, i), JSON.stringify(item)]));
  const a = index(committed);
  const b = index(fresh);
  return {
    removed: [...a.keys()].filter((k) => !b.has(k)),
    added: [...b.keys()].filter((k) => !a.has(k)),
    changed: [...a.keys()].filter((k) => b.has(k) && a.get(k) !== b.get(k)),
  };
}

function list(title, ids) {
  if (ids.length === 0) return [];
  const shown = ids.slice(0, MAX_LISTED).map((k) => `  - \`${k}\``);
  const more = ids.length > MAX_LISTED ? [`  - ... and ${ids.length - MAX_LISTED} more`] : [];
  return [`- ${title} (${ids.length}):`, ...shown, ...more];
}

/** The Markdown lines that describe the differences; empty when there are none. */
export function compare({ vectorsDir, parityDir, committedVectors, committedParity }) {
  const lines = [];
  for (const name of [...new Set([...jsonFiles(vectorsDir), ...jsonFiles(committedVectors)])].sort()) {
    const fresh = readArray(join(vectorsDir, name), "cases");
    const committed = readArray(join(committedVectors, name), "cases");
    if (fresh === null || committed === null) {
      lines.push(`- \`${name}\` exists only ${fresh === null ? "in the committed vectors" : "in the fresh vectors"}`);
      continue;
    }
    // A case is identified by its position unless it has a name.
    const id = (c, i) => (typeof c?.name === "string" ? c.name : `#${i}`);
    const d = compareItems(committed, fresh, id);
    const part = [
      ...list("cases only in the committed file", d.removed),
      ...list("cases only in the fresh output", d.added),
      ...list("cases that changed", d.changed),
    ];
    if (part.length > 0) lines.push(`### Vectors: \`${name}\``, ...part);
  }
  const fresh = readArray(join(parityDir, "ts-tests.json"), "tests");
  const committed = readArray(committedParity, "tests");
  if (fresh === null || committed === null) {
    lines.push("- `ts-tests.json` is missing on one side");
  } else {
    const d = compareItems(committed, fresh, (t) => t.key);
    const part = [
      ...list("tests removed from TS", d.removed),
      ...list("tests added to TS", d.added),
      ...list("tests whose entry changed", d.changed),
    ];
    if (part.length > 0) lines.push("### TS test list (`parity/ts-tests.json`)", ...part);
  }
  return lines;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [vectorsDir, parityDir] = process.argv.slice(2);
  if (!vectorsDir || !parityDir || process.argv.length !== 4) {
    console.error("usage: node oracle/src/drift/compare.mjs <vectors dir> <parity dir>");
    process.exit(2);
  }
  let lines;
  try {
    lines = compare({
      vectorsDir: resolve(vectorsDir),
      parityDir: resolve(parityDir),
      committedVectors: join(ROOT, "agentcat-sdk", "tests", "vectors"),
      committedParity: join(ROOT, "parity", "ts-tests.json"),
    });
  } catch (err) {
    // On stdout, so the nightly summary (which tees stdout) carries the reason.
    console.log(`- The drift comparison could not run: ${err.message}`);
    process.exit(2);
  }
  if (lines.length === 0) {
    console.log("No drift: vector cases and the TS test list match the committed files.");
  } else {
    console.log(lines.join("\n"));
    process.exit(1);
  }
}

// Writes the Rust test vectors: one file per generator in `genDir`, plus MANIFEST.sha256.
// A generator is <genDir>/<name>.mjs exporting `generate(): Promise<{cases}>`. Every set passes
// checkSet, and every file is the strict encoding (strict-json.mjs) of {ts_sha, generator, cases}.
import { createHash } from "node:crypto";
import { mkdirSync, readdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

import { lock } from "../paths.mjs";
import { encodeStrict } from "../strict-json.mjs";

const MAX_FILE_BYTES = 2 * 1024 * 1024;

/** Writes every vector file and MANIFEST.sha256 into `outDir`; returns the case counts by name. */
export async function writeAll({ outDir, genDir }) {
  const { sha } = lock();
  mkdirSync(outDir, { recursive: true });
  const names = readdirSync(genDir)
    .filter((f) => f.endsWith(".mjs"))
    .map((f) => f.slice(0, -4))
    .sort();
  const manifest = [];
  const counts = {};
  for (const name of names) {
    const { generate } = await import(pathToFileURL(join(genDir, `${name}.mjs`)).href);
    const { cases } = await generate();
    const body = checkedFile(name, cases, sha);
    writeFileSync(join(outDir, `${name}.json`), body);
    manifest.push(`${createHash("sha256").update(body).digest("hex")}  ${name}.json\n`);
    counts[name] = cases.length;
  }
  writeFileSync(join(outDir, "MANIFEST.sha256"), manifest.join(""));
  return counts;
}

/**
 * Throws, naming the generator and the case, unless the set of generator `name` follows the rules:
 * at least one case; each case an object with a non-empty, well-formed string `name`, unique in the
 * set; `ts_test`, when present, a non-empty array of non-empty strings; every value strictly
 * encodable; and the encoded file at most 2 MiB.
 */
export function checkSet(name, cases) {
  checkedFile(name, cases, lock().sha);
}

/** The file text of the set, after checkSet's checks. */
function checkedFile(name, cases, sha) {
  const generator = `oracle/src/vectors/gen/${name}.mjs`;
  const fail = (message) => {
    throw new Error(`${generator}: ${message}`);
  };
  if (!Array.isArray(cases) || cases.length === 0) fail("the set has no cases");
  const seen = new Map();
  cases.forEach((c, i) => {
    if (c === null || typeof c !== "object" || Array.isArray(c)) fail(`case #${i}: not an object`);
    if (typeof c.name !== "string" || c.name === "" || !c.name.isWellFormed()) {
      fail(`case #${i}: "name" must be a non-empty, well-formed string`);
    }
    const label = `case ${JSON.stringify(c.name)} (#${i})`;
    if (seen.has(c.name)) fail(`${label}: the name is already used by case #${seen.get(c.name)}`);
    seen.set(c.name, i);
    const keys = c.ts_test;
    const keysOk = Array.isArray(keys) && keys.length > 0 && keys.every((k) => typeof k === "string" && k !== "");
    if (keys !== undefined && !keysOk) fail(`${label}: "ts_test" must be a non-empty array of non-empty strings`);
  });

  let doc;
  try {
    doc = encodeStrict({ ts_sha: sha, generator, cases });
  } catch (err) {
    const at = /^strict-json: \$\.cases\[(\d+)\]/.exec(err.message);
    if (at === null) throw err;
    const i = Number(at[1]);
    fail(`case ${JSON.stringify(cases[i].name)} (#${i}): ${err.message}`);
  }
  const body = `${JSON.stringify(doc, null, 2)}\n`;
  const size = Buffer.byteLength(body);
  if (size > MAX_FILE_BYTES) {
    const sizes = doc.cases.map((c) => Buffer.byteLength(JSON.stringify(c)));
    const i = sizes.reduce((largest, n, j) => (n > sizes[largest] ? j : largest), 0);
    fail(
      `the file is ${size} bytes, over the limit of ${MAX_FILE_BYTES}; ` +
        `the largest case is ${JSON.stringify(cases[i].name)} (#${i})`,
    );
  }
  return body;
}

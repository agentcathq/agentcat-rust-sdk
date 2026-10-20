// Generates the Rust test vectors by running the real TS modules.
//   node oracle/src/vectors/run.mjs               write agentcat-sdk/tests/vectors/
//   node oracle/src/vectors/run.mjs --out <dir>   write there instead
//   node oracle/src/vectors/run.mjs --check       regenerate to a temp dir; exit 1 on any byte difference
// A generator is oracle/src/vectors/gen/<name>.mjs exporting `generate(): Promise<{cases}>`.
import { createHash } from "node:crypto";
import { mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { lock } from "../paths.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const GEN_DIR = join(HERE, "gen");
const DEFAULT_OUT = resolve(HERE, "../../../agentcat-sdk/tests/vectors");

/** Writes every vector file and MANIFEST.sha256 into `dir`; returns the case counts by name. */
async function writeAll(dir) {
  const { sha } = lock();
  mkdirSync(dir, { recursive: true });
  const names = readdirSync(GEN_DIR)
    .filter((f) => f.endsWith(".mjs"))
    .map((f) => f.slice(0, -4))
    .sort();
  const manifest = [];
  const counts = {};
  for (const name of names) {
    const { generate } = await import(pathToFileURL(join(GEN_DIR, `${name}.mjs`)).href);
    const { cases } = await generate();
    const body = JSON.stringify(
      { ts_sha: sha, generator: `oracle/src/vectors/gen/${name}.mjs`, cases },
      null,
      2,
    ) + "\n";
    writeFileSync(join(dir, `${name}.json`), body);
    manifest.push(`${createHash("sha256").update(body).digest("hex")}  ${name}.json\n`);
    counts[name] = cases.length;
  }
  writeFileSync(join(dir, "MANIFEST.sha256"), manifest.join(""));
  return counts;
}

function listFiles(dir) {
  return readdirSync(dir).filter((f) => f.endsWith(".json") || f === "MANIFEST.sha256").sort();
}

async function check() {
  const tmp = mkdtempSync(join(tmpdir(), "vectors-"));
  try {
    await writeAll(tmp);
    const fresh = listFiles(tmp);
    const have = listFiles(DEFAULT_OUT);
    const bad = [];
    for (const f of new Set([...fresh, ...have])) {
      let a;
      let b;
      try { a = readFileSync(join(tmp, f)); } catch { a = null; }
      try { b = readFileSync(join(DEFAULT_OUT, f)); } catch { b = null; }
      if (a === null || b === null || !a.equals(b)) bad.push(f);
    }
    if (bad.length) {
      process.stderr.write(`vectors out of date: ${bad.join(", ")}\n`);
      return 1;
    }
    process.stdout.write("vectors up to date\n");
    return 0;
  } finally {
    rmSync(tmp, { recursive: true, force: true });
  }
}

const args = process.argv.slice(2);
if (args.includes("--check")) {
  process.exitCode = await check();
} else {
  const i = args.indexOf("--out");
  if (i >= 0 && !args[i + 1]) {
    process.stderr.write("--out needs a directory\n");
    process.exit(2);
  }
  const out = i >= 0 ? resolve(args[i + 1]) : DEFAULT_OUT;
  const counts = await writeAll(out);
  for (const [name, n] of Object.entries(counts)) process.stdout.write(`${name}: ${n} cases -> ${out}\n`);
}

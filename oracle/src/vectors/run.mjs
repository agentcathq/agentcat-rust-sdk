// Generates the Rust test vectors by running the real TS modules.
//   node oracle/src/vectors/run.mjs               write agentcat-sdk/tests/vectors/
//   node oracle/src/vectors/run.mjs --out <dir>   write there instead
//   node oracle/src/vectors/run.mjs --check       regenerate to a temp dir; exit 1 on any byte difference
// Any other argument: the usage line and exit 2. The writing itself is in write.mjs, which runs every
// generator in gen/ (each <name>.mjs writes <name>.json). The egress guard is installed first, so no
// generator reaches the network: they run TS code (the queue, track on an MCP server) in this process.
import "../egress-guard.mjs";

import { mkdtempSync, readFileSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { traceOverride } from "../paths.mjs";
import { writeAll } from "./write.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const GEN_DIR = join(HERE, "gen");
const DEFAULT_OUT = resolve(HERE, "../../../agentcat-sdk/tests/vectors");

function listFiles(dir) {
  return readdirSync(dir).filter((f) => f.endsWith(".json") || f === "MANIFEST.sha256").sort();
}

async function check() {
  const tmp = mkdtempSync(join(tmpdir(), "vectors-"));
  try {
    await writeAll({ outDir: tmp, genDir: GEN_DIR });
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
const outAt = args.indexOf("--out");
if (outAt >= 0 && !args[outAt + 1]) {
  process.stderr.write("--out needs a directory\n");
  process.exit(2);
}
// Anything else is a usage error: a misspelled --check must not regenerate and exit 0.
const known = new Set(["--check", "--out"]);
const unknown = args.filter((arg, i) => !known.has(arg) && !(outAt >= 0 && i === outAt + 1));
if (unknown.length > 0 || args.length > (outAt >= 0 ? 2 : 1)) {
  process.stderr.write("usage: node oracle/src/vectors/run.mjs [--check | --out <dir>]\n");
  process.exit(2);
}
traceOverride();
if (args.includes("--check")) {
  process.exitCode = await check();
} else {
  const out = outAt >= 0 ? resolve(args[outAt + 1]) : DEFAULT_OUT;
  const counts = await writeAll({ outDir: out, genDir: GEN_DIR });
  for (const [name, n] of Object.entries(counts)) process.stdout.write(`${name}: ${n} cases -> ${out}\n`);
}

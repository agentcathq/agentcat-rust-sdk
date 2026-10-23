// The L3 check of the last matrix run (scripts/check.sh and the CI job harness-l3 run it after the A/A
// and the canary matrices).
//
// usage: node harness/src/l3-expect.mjs <none|caught> [matrix.json]   (default harness/out/matrix.json)
//   none    the A/A run: at least one combination, every driver exited 0, and every delta has no
//           difference at all, ruled or not (delta.total 0).
//   caught  the canary run (HARNESS_CANARY=1 on the second server of `--scenarios basic`): at least one
//           combination, every driver exited 0, and every combination has an unruled difference, each at
//           a path under CANARY_PREFIX (the injected context description of the tracked tools/list, the
//           only thing the canary changes). A canary caught anywhere else is a different failure.
// Exit 0: as expected. Exit 1: not as expected (one line per bad combination). Exit 2: bad arguments or an
// unreadable matrix.json.
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { readJson } from "./rulings.mjs";

/** Where the canary differences of basic.json sit: transcript[1] is its tracked tools/list. */
export const CANARY_PREFIX = "$.delta.transcript[1].result.tools[";

const DEFAULT = join(dirname(fileURLToPath(import.meta.url)), "..", "out", "matrix.json");

function usage(message) {
  process.stderr.write(`l3-expect: ${message}\nusage: node harness/src/l3-expect.mjs <none|caught> [matrix.json]\n`);
  process.exit(2);
}

/** The problem of one combination under `mode`, or undefined when it is as expected. */
function problem(c, mode) {
  const failed = Object.entries(c.exitCodes ?? {}).filter(([, code]) => code !== 0);
  if (c.exitCodes === undefined || failed.length > 0) return `driver exit ${JSON.stringify(c.exitCodes)}`;
  if (c.delta === undefined) return "no delta";
  if (mode === "none") return c.delta.total === 0 ? undefined : `delta ${JSON.stringify(c.delta)}`;
  const diffs = c.differences ?? [];
  if (c.delta.unexpected === 0 || diffs.length === 0) return `no unruled difference: delta ${JSON.stringify(c.delta)}`;
  const elsewhere = diffs.filter((d) => typeof d.path !== "string" || !d.path.startsWith(CANARY_PREFIX));
  return elsewhere.length === 0 ? undefined : `a difference outside the canary paths: ${elsewhere[0].path}`;
}

const [mode, path = DEFAULT, ...rest] = process.argv.slice(2);
if (!["none", "caught"].includes(mode) || rest.length > 0) usage(`expected none or caught, then an optional path; got ${JSON.stringify(process.argv.slice(2))}`);
const doc = readJson(path, usage);
if (!Array.isArray(doc)) usage(`${path}: must be a JSON array`);
const what = mode === "none" ? "A/A" : "canary";
const bad = doc.map((c) => [c, problem(c, mode)]).filter(([, p]) => p !== undefined);
if (doc.length === 0) {
  process.stderr.write(`L3 ${what} failed: no combination ran\n`);
  process.exit(1);
}
if (bad.length > 0) {
  for (const [c, p] of bad) process.stderr.write(`L3 ${what} failed: ${c.scenario} ${c.transport} ${c.lane}: ${p}\n`);
  process.exit(1);
}
process.stdout.write(`L3 ${what} ok: ${doc.length} combination(s)\n`);

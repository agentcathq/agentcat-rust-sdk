// The L3 check of the last matrix run (scripts/check.sh and the CI job harness-l3 run it after the A/A
// and the canary matrices).
//
// usage: node harness/src/l3-expect.mjs <none|caught|caught-call|caught-event> [matrix.json]
//          (default harness/out/matrix.json)
//   none          the A/A run: at least one combination, every driver exited 0, and every delta and every
//                 event comparison has no difference at all, ruled or not (delta.total 0, events.total 0).
//   caught        the context canary run (HARNESS_CANARY=1 on the second server of `--scenarios basic`): at
//                 least one combination, every driver exited 0, every combination has an unruled delta
//                 difference, each at a path under CANARY_PREFIX (the injected context description of the
//                 tracked tools/list, the only thing the canary changes), and no event difference (events.total
//                 0: the canary does not change what the servers publish). A canary caught anywhere else is a
//                 different failure.
//   caught-call   the call canary run (HARNESS_CANARY=call: an outer handler around the tracked Rust server
//                 changes the issued mint-back text): as `caught`, each unruled delta difference at a result,
//                 its first content block or that block's text (CALL_CANARY_PATH: a call the untracked server
//                 answered with an error shows the whole tracked result as added), with the canary's change
//                 alone (the Rust text is the TS text with MINT_BACK made CHANGED), and no event difference (the
//                 event records the result before the outer handler changes it).
//   caught-event  the event canary run (HARNESS_CANARY=event: a redact_event hook renames every event that
//                 runs it): at least one combination, every driver exited 0, no delta difference at all
//                 (delta.total 0: the hook runs after the call), and an unruled event difference in every
//                 combination, each at an event's resource_name (EVENT_CANARY_PATH).
// Exit 0: as expected. Exit 1: not as expected (one line per bad combination). Exit 2: bad arguments or an
// unreadable matrix.json.
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { readJson } from "./rulings.mjs";

/** Where the canary differences of basic.json sit: transcript[1] is its tracked tools/list. */
export const CANARY_PREFIX = "$.delta.transcript[1].result.tools[";

/** Where the call canary's differences sit: a result, its first content block (the mint-back block) or its text. */
export const CALL_CANARY_PATH = /^\$\.delta\.transcript\[\d+\]\.result(?:\.content\[0\](?:\.text)?)?$/;

/** The start of the mint-back text, and what the call canary makes of it. */
const MINT_BACK = "[session_id issued";
const CHANGED = "[session_id lssued";

/** Whether delta difference `d` shows the call canary's change alone: same text but for the mint-back start. */
const callCanaryChange = (d) =>
  typeof d.ts === "string" && d.ts.includes(MINT_BACK) && d.rust === d.ts.replace(MINT_BACK, CHANGED);

/** Where the event canary's differences sit: the resource name of an event. */
export const EVENT_CANARY_PATH = /^\$\.events\[\d+\]\.body\.resource_name$/;

/** The modes, and the name of each in the report lines. */
const MODES = { none: "A/A", caught: "canary", "caught-call": "call canary", "caught-event": "event canary" };

const DEFAULT = join(dirname(fileURLToPath(import.meta.url)), "..", "out", "matrix.json");

function usage(message) {
  process.stderr.write(`l3-expect: ${message}\nusage: node harness/src/l3-expect.mjs <none|caught|caught-call|caught-event> [matrix.json]\n`);
  process.exit(2);
}

/** Whether a delta difference sits where the canary of `mode` (caught or caught-call) changes the wire. */
const atCanary = (mode, path) =>
  typeof path === "string" && (mode === "caught" ? path.startsWith(CANARY_PREFIX) : CALL_CANARY_PATH.test(path));

/** The problem of one combination under `mode`, or undefined when it is as expected. */
function problem(c, mode) {
  const failed = Object.entries(c.exitCodes ?? {}).filter(([, code]) => code !== 0);
  if (c.exitCodes === undefined || failed.length > 0) return `driver exit ${JSON.stringify(c.exitCodes)}`;
  if (c.delta === undefined) return "no delta";
  if (c.events === undefined) return "no event comparison";
  if (mode === "none") {
    if (c.delta.total !== 0) return `delta ${JSON.stringify(c.delta)}`;
    return c.events.total === 0 ? undefined : `events ${JSON.stringify(c.events)}`;
  }
  if (mode === "caught-event") {
    if (c.delta.total !== 0) return `a delta difference: delta ${JSON.stringify(c.delta)}`;
    const diffs = c.eventDifferences ?? [];
    if (c.events.unexpected === 0 || diffs.length === 0) return `no unruled event difference: events ${JSON.stringify(c.events)}`;
    const elsewhere = diffs.filter((d) => typeof d.path !== "string" || !EVENT_CANARY_PATH.test(d.path));
    return elsewhere.length === 0 ? undefined : `an event difference outside the canary paths: ${elsewhere[0].path}`;
  }
  if (c.events.total !== 0) {
    const first = (c.eventDifferences ?? [])[0];
    return first ? `an event difference: ${first.path}` : `events ${JSON.stringify(c.events)}`;
  }
  const diffs = c.differences ?? [];
  if (c.delta.unexpected === 0 || diffs.length === 0) return `no unruled difference: delta ${JSON.stringify(c.delta)}`;
  const elsewhere = diffs.filter((d) => !atCanary(mode, d.path));
  if (elsewhere.length > 0) return `a difference outside the canary paths: ${elsewhere[0].path}`;
  const other = mode === "caught-call" ? diffs.find((d) => !callCanaryChange(d)) : undefined;
  return other === undefined ? undefined : `not the call canary's change at ${other.path}: ts ${other.ts} rust ${other.rust}`;
}

const [mode, path = DEFAULT, ...rest] = process.argv.slice(2);
if (!Object.hasOwn(MODES, mode) || rest.length > 0) {
  usage(`expected ${Object.keys(MODES).join(", ")}, then an optional path; got ${JSON.stringify(process.argv.slice(2))}`);
}
const doc = readJson(path, usage);
if (!Array.isArray(doc)) usage(`${path}: must be a JSON array`);
const what = MODES[mode];
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

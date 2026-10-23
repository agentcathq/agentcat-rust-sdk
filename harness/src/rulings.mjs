// Rulings shared by diff.mjs, delta.mjs and matrix.mjs: the file format, the matching and the report lines.
// A rulings file is a JSON array of
// {"path": "<regex>", "reason", "deviation": "RD-xx", "pending"?: true, "scenario"?: "<scenario name>",
//  "transports"?: ["<transport>", ...], "lanes"?: ["<lane>", ...]};
// each regex is anchored as ^(?:<regex>)$. `scenario`, `transports` and `lanes` scope a delta ruling to the
// matching combinations in matrix.mjs (see rulingsFor); diff.mjs and delta.mjs know no combination and
// match by path alone. `fail(message)` reports a bad file and must not return.
import { readFileSync } from "node:fs";

/** The form of a scenario name: harness/scenarios/<name>.json. */
export const SCENARIO_NAME = /^[A-Za-z0-9_-]+$/;

/** The transports of the matrix (driver.mjs runs each). */
export const TRANSPORTS = ["stdio", "http", "http-stateless"];

/** The protocol lanes of the matrix (driver.mjs --lane). */
export const LANES = ["2025-11-25", "2026-07-28"];

/** The keys a ruling may have. */
const KEYS = ["path", "reason", "deviation", "pending", "scenario", "transports", "lanes"];

/** Fails unless `r[key]`, when present, is a non-empty array of names from `allowed`. */
function checkNames(r, key, allowed, what, where, fail) {
  if (!Object.hasOwn(r, key)) return;
  const names = r[key];
  if (!Array.isArray(names) || names.length === 0 || names.some((n) => typeof n !== "string")) {
    fail(`${where}: "${key}" must be a non-empty array of ${what} names`);
  }
  for (const n of names) {
    if (!allowed.includes(n)) fail(`${where}: "${key}" names an unknown ${what} ${JSON.stringify(n)}; expected one of ${allowed.join(", ")}`);
  }
}

export function readJson(path, fail) {
  try {
    return JSON.parse(readFileSync(path, "utf8"));
  } catch (err) {
    return fail(`${path}: ${err.message}`);
  }
}

export function loadRulings(path, fail) {
  if (path === undefined) return [];
  const list = readJson(path, fail);
  if (!Array.isArray(list)) fail(`${path}: must be a JSON array`);
  return list.map((r, i) => {
    const where = `${path}[${i}]`;
    if (r === null || typeof r !== "object" || Array.isArray(r)) fail(`${where}: must be an object`);
    const extra = Object.keys(r).filter((k) => !KEYS.includes(k));
    if (extra.length > 0) fail(`${where}: unknown key ${JSON.stringify(extra[0])}`);
    if (Object.hasOwn(r, "pending") && typeof r.pending !== "boolean") fail(`${where}: "pending" must be a boolean`);
    if (Object.hasOwn(r, "scenario") && (typeof r.scenario !== "string" || !SCENARIO_NAME.test(r.scenario))) {
      fail(`${where}: "scenario" must be a scenario name (letters, digits, _ and -)`);
    }
    checkNames(r, "transports", TRANSPORTS, "transport", where, fail);
    checkNames(r, "lanes", LANES, "lane", where, fail);
    if (typeof r.path !== "string" || r.path === "") fail(`${where}: "path" must be a non-empty regex string`);
    if (typeof r.reason !== "string" || r.reason.trim() === "") fail(`${where}: "reason" must be a non-empty string`);
    if (typeof r.deviation !== "string" || !/^RD-\d+$/.test(r.deviation)) fail(`${where}: "deviation" must look like RD-<n>`);
    let re;
    try {
      new RegExp(r.path); // the pattern must be valid on its own, not only once wrapped
      re = new RegExp(`^(?:${r.path})$`);
    } catch (err) {
      fail(`${where}: bad regex: ${err.message}`);
    }
    return { ...r, re, matched: 0 };
  });
}

/** The rulings whose whole-path regex matches; each hit adds one to the ruling's count. */
export function matchRulings(rulings, path) {
  const hits = rulings.filter((r) => r.re.test(path));
  for (const r of hits) r.matched += 1;
  return hits;
}

/**
 * The rulings that apply to one combination of the matrix, {scenario, transport, lane}: each ruling whose
 * `scenario`, `transports` and `lanes` are absent or name that combination's. The same objects, so a match
 * still counts in the ruling's `matched`.
 */
export function rulingsFor(rulings, { scenario, transport, lane }) {
  return rulings.filter(
    (r) =>
      (r.scenario === undefined || r.scenario === scenario) &&
      (r.transports === undefined || r.transports.includes(transport)) &&
      (r.lanes === undefined || r.lanes.includes(lane)),
  );
}

/**
 * Whether a ruling can apply to a matrix run over `scenarios`, `transports` and `lanes` (a cross
 * product): its scenario, one of its transports and one of its lanes are in the run, where it names them.
 */
export function rulingInRun(r, { scenarios, transports, lanes }) {
  return (
    (r.scenario === undefined || scenarios.includes(r.scenario)) &&
    (r.transports === undefined || r.transports.some((t) => transports.includes(t))) &&
    (r.lanes === undefined || r.lanes.some((l) => lanes.includes(l)))
  );
}

/**
 * One line per ruling: RULING / RULING (pending) / UNUSED RULING, with the match count and the scope of
 * a scoped ruling: `in <scenario>`, `on <transport>,...`, `at <lane>,...`.
 */
export function rulingLines(rulings) {
  return rulings.map((r) => {
    const what = `${r.matched === 0 ? "UNUSED RULING" : "RULING"}${r.pending ? " (pending)" : ""}`;
    const scope = [
      r.scenario === undefined ? "" : ` in ${r.scenario}`,
      r.transports === undefined ? "" : ` on ${r.transports.join(",")}`,
      r.lanes === undefined ? "" : ` at ${r.lanes.join(",")}`,
    ].join("");
    return `${what} ${r.deviation} matched ${r.matched}${scope}: ${JSON.stringify(r.path)} (${r.reason})\n`;
  });
}

export const clip = (v) => {
  const text = JSON.stringify(v) ?? "undefined";
  return text.length > 120 ? `${text.slice(0, 117)}...` : text;
};

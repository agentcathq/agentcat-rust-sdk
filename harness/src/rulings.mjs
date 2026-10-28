// Rulings shared by diff.mjs, delta.mjs, events.mjs and matrix.mjs: the file format, the matching and the
// report lines. Delta rulings live in harness/rulings.json (rulings-ts.json for TS against TS), event rulings
// in harness/rulings-events.json (rulings-events-ts.json). A rulings file is a JSON array of
// {"path": "<regex>", "reason", "deviation": "RD-xx", "pending"?: true, "scenario"?: "<scenario name>",
//  "transports"?: ["<transport>", ...], "lanes"?: ["<lane>", ...], "ts"?: <value>, "rust"?: <value>,
//  "rustKeys"?: "<regex>"};
// each regex is anchored as ^(?:<regex>)$. `scenario`, `transports` and `lanes` scope a delta or event ruling
// to the matching combinations in matrix.mjs (see rulingsFor); diff.mjs and delta.mjs know no combination and
// match by path alone. `ts`, `rust` and `rustKeys` pin the values a ruling allows (see valuesHold): they apply
// where a difference carries its two values, `a` (the first server, TS) and `b` (the second, Rust), that is
// in the event comparison and diff.mjs; a delta difference carries none, so a pinned ruling never matches it.
// `fail(message)` reports a bad file and must not return.
import { readFileSync } from "node:fs";

/** The form of a scenario name: harness/scenarios/<name>.json. */
export const SCENARIO_NAME = /^[A-Za-z0-9_-]+$/;

/** The transports of the matrix (driver.mjs runs each). */
export const TRANSPORTS = ["stdio", "http", "http-stateless"];

/** The protocol lanes of the matrix (driver.mjs --lane). */
export const LANES = ["2025-11-25", "2026-07-28"];

/** The keys a ruling may have. */
const KEYS = ["path", "reason", "deviation", "pending", "scenario", "transports", "lanes", "ts", "rust", "rustKeys"];

const isObject = (v) => v !== null && typeof v === "object" && !Array.isArray(v);

/** A string that is equal for deeply equal JSON values, whatever the key order. */
export function canon(v) {
  if (Array.isArray(v)) return `[${v.map(canon).join(",")}]`;
  if (isObject(v)) return `{${Object.keys(v).sort().map((k) => `${JSON.stringify(k)}:${canon(v[k])}`).join(",")}}`;
  return JSON.stringify(v) ?? "undefined";
}

/** Compiles a ruling regex, anchored as ^(?:<regex>)$; `what` names it in the failure. */
function anchored(text, what, where, fail) {
  try {
    new RegExp(text); // the pattern must be valid on its own, not only once wrapped
    return new RegExp(`^(?:${text})$`);
  } catch (err) {
    return fail(`${where}: bad ${what}regex: ${err.message}`);
  }
}

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

/** The keys that pin a value; only an event ruling carries values to pin. */
const PIN_KEYS = ["ts", "rust", "rustKeys"];

/**
 * Loads and checks a rulings file. `pins: false` (a delta rulings file: delta.mjs, matrix.mjs --rulings)
 * refuses the keys that pin values, `ts`, `rust` and `rustKeys`: a delta difference carries no values, so
 * such a ruling would never match.
 */
export function loadRulings(path, fail, { pins = true } = {}) {
  if (path === undefined) return [];
  const list = readJson(path, fail);
  if (!Array.isArray(list)) fail(`${path}: must be a JSON array`);
  return list.map((r, i) => {
    const where = `${path}[${i}]`;
    if (r === null || typeof r !== "object" || Array.isArray(r)) fail(`${where}: must be an object`);
    const extra = Object.keys(r).filter((k) => !KEYS.includes(k));
    if (extra.length > 0) fail(`${where}: unknown key ${JSON.stringify(extra[0])}`);
    const pinned = pins ? undefined : PIN_KEYS.find((k) => Object.hasOwn(r, k));
    if (pinned !== undefined) fail(`${where}: "${pinned}" pins a value, which only an event ruling can do`);
    if (Object.hasOwn(r, "pending") && typeof r.pending !== "boolean") fail(`${where}: "pending" must be a boolean`);
    if (Object.hasOwn(r, "scenario") && (typeof r.scenario !== "string" || !SCENARIO_NAME.test(r.scenario))) {
      fail(`${where}: "scenario" must be a scenario name (letters, digits, _ and -)`);
    }
    checkNames(r, "transports", TRANSPORTS, "transport", where, fail);
    checkNames(r, "lanes", LANES, "lane", where, fail);
    if (typeof r.path !== "string" || r.path === "") fail(`${where}: "path" must be a non-empty regex string`);
    if (typeof r.reason !== "string" || r.reason.trim() === "") fail(`${where}: "reason" must be a non-empty string`);
    if (typeof r.deviation !== "string" || !/^RD-\d+$/.test(r.deviation)) fail(`${where}: "deviation" must look like RD-<n>`);
    if (Object.hasOwn(r, "rustKeys") && (typeof r.rustKeys !== "string" || r.rustKeys === "")) {
      fail(`${where}: "rustKeys" must be a non-empty regex string`);
    }
    const re = anchored(r.path, "", where, fail);
    const rustKeysRe = Object.hasOwn(r, "rustKeys") ? anchored(r.rustKeys, "rustKeys ", where, fail) : undefined;
    return { ...r, re, rustKeysRe, matched: 0 };
  });
}

/**
 * Whether the values of a difference, `values` = {a, b} (the first server's and the second's; absent for a
 * delta difference), are the ones a ruling allows: `ts` deep-equals `a` and `rust` deep-equals `b` (key order
 * aside; a side that does not have the key is the text "<absent>"), and with `rustKeys`, `b` is an object of
 * at least one key, each matching the anchored regex. A ruling without these keys allows any values; a
 * ruling with them never allows a difference without values.
 */
function valuesHold(r, values) {
  const pinsTs = Object.hasOwn(r, "ts");
  const pinsRust = Object.hasOwn(r, "rust");
  if (!pinsTs && !pinsRust && r.rustKeysRe === undefined) return true;
  if (values === undefined) return false;
  if (pinsTs && canon(values.a) !== canon(r.ts)) return false;
  if (pinsRust && canon(values.b) !== canon(r.rust)) return false;
  if (r.rustKeysRe !== undefined) {
    const keys = isObject(values.b) ? Object.keys(values.b) : [];
    if (keys.length === 0 || !keys.every((k) => r.rustKeysRe.test(k))) return false;
  }
  return true;
}

/**
 * The rulings whose whole-path regex matches and whose pinned values (valuesHold) allow `values`; each hit
 * adds one to the ruling's count.
 */
export function matchRulings(rulings, path, values) {
  const hits = rulings.filter((r) => r.re.test(path) && valuesHold(r, values));
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
 * One line per ruling: RULING / RULING (pending) / UNUSED RULING, with the match count, the scope of
 * a scoped ruling: `in <scenario>`, `on <transport>,...`, `at <lane>,...`, and its pinned values:
 * `ts=<value>`, `rust=<value>`, `rustKeys="<regex>"`. Each line starts with `prefix`
 * (matrix.mjs: `EVENT ` for the event rulings).
 */
export function rulingLines(rulings, prefix = "") {
  return rulings.map((r) => {
    const what = `${r.matched === 0 ? "UNUSED RULING" : "RULING"}${r.pending ? " (pending)" : ""}`;
    const scope = [
      r.scenario === undefined ? "" : ` in ${r.scenario}`,
      r.transports === undefined ? "" : ` on ${r.transports.join(",")}`,
      r.lanes === undefined ? "" : ` at ${r.lanes.join(",")}`,
      Object.hasOwn(r, "ts") ? ` ts=${clip(r.ts)}` : "",
      Object.hasOwn(r, "rust") ? ` rust=${clip(r.rust)}` : "",
      Object.hasOwn(r, "rustKeys") ? ` rustKeys=${JSON.stringify(r.rustKeys)}` : "",
    ].join("");
    return `${prefix}${what} ${r.deviation} matched ${r.matched}${scope}: ${JSON.stringify(r.path)} (${r.reason})\n`;
  });
}

export const clip = (v) => {
  const text = JSON.stringify(v) ?? "undefined";
  return text.length > 120 ? `${text.slice(0, 117)}...` : text;
};

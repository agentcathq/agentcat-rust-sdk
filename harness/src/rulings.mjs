// Rulings shared by diff.mjs, delta.mjs and matrix.mjs: the file format, the matching and the report lines.
// A rulings file is a JSON array of
// {"path": "<regex>", "reason", "deviation": "RD-xx", "pending"?: true, "scenario"?: "<scenario name>"};
// each regex is anchored as ^(?:<regex>)$. `scenario` scopes a delta ruling to that scenario's combinations
// in matrix.mjs (see rulingsForScenario); diff.mjs and delta.mjs know no scenario and match by path alone.
// `fail(message)` reports a bad file and must not return.
import { readFileSync } from "node:fs";

/** The form of a scenario name: harness/scenarios/<name>.json. */
export const SCENARIO_NAME = /^[A-Za-z0-9_-]+$/;

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
    const extra = Object.keys(r).filter((k) => !["path", "reason", "deviation", "pending", "scenario"].includes(k));
    if (extra.length > 0) fail(`${where}: unknown key ${JSON.stringify(extra[0])}`);
    if (Object.hasOwn(r, "pending") && typeof r.pending !== "boolean") fail(`${where}: "pending" must be a boolean`);
    if (Object.hasOwn(r, "scenario") && (typeof r.scenario !== "string" || !SCENARIO_NAME.test(r.scenario))) {
      fail(`${where}: "scenario" must be a scenario name (letters, digits, _ and -)`);
    }
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
 * The rulings that apply to one scenario's combinations: the unscoped ones and the ones scoped to it.
 * The same objects, so a match still counts in the ruling's `matched`.
 */
export function rulingsForScenario(rulings, scenario) {
  return rulings.filter((r) => r.scenario === undefined || r.scenario === scenario);
}

/**
 * One line per ruling: RULING / RULING (pending) / UNUSED RULING, with the match count and, for a
 * scenario-scoped ruling, `in <scenario>`.
 */
export function rulingLines(rulings) {
  return rulings.map((r) => {
    const what = `${r.matched === 0 ? "UNUSED RULING" : "RULING"}${r.pending ? " (pending)" : ""}`;
    const scope = r.scenario === undefined ? "" : ` in ${r.scenario}`;
    return `${what} ${r.deviation} matched ${r.matched}${scope}: ${JSON.stringify(r.path)} (${r.reason})\n`;
  });
}

export const clip = (v) => {
  const text = JSON.stringify(v) ?? "undefined";
  return text.length > 120 ? `${text.slice(0, 117)}...` : text;
};

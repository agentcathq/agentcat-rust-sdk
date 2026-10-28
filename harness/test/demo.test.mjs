// The dashboard demo (demo/run.mjs, decision C58). The --prod refusals must happen before any server process
// starts or any network call: each refusal test runs under the egress guard and asserts that the runner never
// printed its "starting" line. No test runs --prod with the demo project and CI unset.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { test } from "node:test";

import { GUARD, HARNESS, run } from "./support.mjs";

const RUN = join(HARNESS, "demo", "run.mjs");
const PROD_PROJECT = "proj_2yAR5QHJ3CE3icTpJlKJqu1ZiXG";

/** Runs the demo under the egress guard with the CI variable `ci` (undefined: unset). */
function demo(args, { ci } = {}) {
  const env = { ...process.env };
  delete env.CI;
  if (ci !== undefined) env.CI = ci;
  return run(process.execPath, ["--import", pathToFileURL(GUARD).href, RUN, ...args], { env, timeoutMs: 280_000 });
}

/** Asserts a refusal: exit 2, a reason on stderr, and no sign that a server or a request started. */
function assertRefused(result, reason) {
  assert.equal(result.code, 2, result.stderr);
  assert.match(result.stderr, reason);
  assert.doesNotMatch(result.stderr, /starting/);
  assert.equal(result.stdout, "");
}

test("--prod without --project is refused", { timeout: 60_000 }, async () => {
  assertRefused(await demo(["--prod"]), /--project/);
});

test("--prod with another project is refused", { timeout: 60_000 }, async () => {
  assertRefused(await demo(["--prod", "--project", "proj_someone_else"]), /only the project/);
});

test("--prod with CI set is refused, even for the demo project", { timeout: 60_000 }, async () => {
  assertRefused(await demo(["--prod", "--project", PROD_PROJECT], { ci: "true" }), /CI is set/);
});

test("no mode, and both modes, are refused", { timeout: 60_000 }, async () => {
  assertRefused(await demo([]), /usage/);
  assertRefused(await demo(["--mock", "--prod"]), /usage/);
});

test("--mock runs both servers and the mock receives the same event count from each", { timeout: 290_000 }, async () => {
  const result = await demo(["--mock"], { ci: "true" });
  assert.equal(result.code, 0, result.stderr);
  const summary = JSON.parse(result.stdout.trim().split("\n").at(-1));
  assert.ok(summary.rust.count > 0 && summary.ts.count > 0, result.stdout);
  assert.equal(summary.rust.count, summary.ts.count);
  assert.deepEqual(summary.rust.types, summary.ts.types);
  // Three echo calls give three custom events (the note in scenario.json).
  assert.equal(summary.rust.types["agentcat:custom"], 3);
});

test("the scenario names three echo calls, one with secret-token", () => {
  const scenario = JSON.parse(readFileSync(join(HARNESS, "demo", "scenario.json"), "utf8"));
  const calls = scenario.sessions.flatMap((session) => session.steps).filter((step) => step.tool);
  assert.equal(calls.filter((c) => c.tool === "echo").length, 3);
  assert.equal(calls.filter((c) => JSON.stringify(c.arguments).includes("secret-token")).length, 1);
  assert.match(scenario._comment, /three custom events/);
});

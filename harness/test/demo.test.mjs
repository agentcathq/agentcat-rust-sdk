// The dashboard demo (demo/run.mjs, decision C58). The --prod refusals must happen before any server process
// starts or any network call: each refusal test runs under the egress guard and asserts that the runner never
// printed its "starting" line. No test runs --prod with the demo project and CI unset.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { test } from "node:test";

import { compareEvents, compareTools } from "../demo/compare.mjs";
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

// ---- the comparison of the two servers' publishes (demo/compare.mjs) ----

const SESSIONS = ["ses_one", "ses_two"];
/** A publish as the mock records it. */
const publish = (body, rawBody = JSON.stringify(body)) => ({ body, rawBody });
const toolEvent = (name, session, extra = {}) =>
  publish({
    event_type: "mcp:tools/call", resource_name: name, session_id: session, identify_actor_given_id: "demo-user",
    identify_actor_name: "Demo User", tags: { example: "hooks" }, properties: { source: "hooks-example", redacted_by: "example" },
    is_error: false, ...extra,
  });
const custom = (session) =>
  publish({ event_type: "agentcat:custom", resource_name: "echo-called", session_id: session, properties: { redacted_by: "example" } });
const secretEcho = () => toolEvent("echo", "ses_one", { parameters: { text: "my [REDACTED] is here" } });

/** Rust and TS lists as the demo produces them: RD-39 makes the TS custom event of session two sessionless. */
function lists({ tsSecond = "" } = {}) {
  const shared = [toolEvent("add", "ses_one"), secretEcho(), toolEvent("fail", "ses_one", { is_error: true, error: { type: "T", message: "m", platform: "rust" } })];
  const rustRequests = [...shared, custom("ses_one"), custom("ses_one"), custom("ses_two")];
  const tsRequests = [
    ...shared.slice(0, 2),
    toolEvent("fail", "ses_one", { is_error: true, error: { type: "T", message: "m", platform: "javascript", stack: "x" } }),
    custom("ses_one"), custom("ses_one"), custom(tsSecond),
  ];
  return { rust: { requests: rustRequests, sessions: SESSIONS }, ts: { requests: tsRequests, sessions: SESSIONS } };
}

test("compareEvents: identical lists, with the RD-39 difference, pass", () => {
  const { failures, compared } = compareEvents({ ...lists(), secretSteps: 1 });
  assert.deepEqual(failures, []);
  assert.equal(compared, 6);
});

test("compareEvents: a changed tag fails", () => {
  const input = lists();
  input.ts.requests[0] = toolEvent("add", "ses_one", { tags: { example: "other" } });
  const { failures } = compareEvents({ ...input, secretSteps: 1 });
  assert.ok(failures.some((f) => /only in (rust|ts)/.test(f)), failures.join("\n"));
});

test("compareEvents: a changed error message or user name fails", () => {
  for (const change of [{ error: { type: "T", message: "other" } }, { identify_actor_name: "Someone" }]) {
    const input = lists();
    input.ts.requests[2] = toolEvent("fail", "ses_one", { is_error: true, error: { type: "T", message: "m" }, ...change });
    assert.ok(compareEvents({ ...input, secretSteps: 1 }).failures.length > 0);
  }
});

test("compareEvents: the RD-39 difference gone fails, and so does one that moved", () => {
  const gone = compareEvents({ ...lists({ tsSecond: "ses_two" }), secretSteps: 1 });
  assert.ok(gone.failures.some((f) => /RD-39/.test(f)), gone.failures.join("\n"));
  const input = lists({ tsSecond: "ses_two" });
  input.ts.requests[3] = custom(""); // the sessionless one is now a first-session event
  const moved = compareEvents({ ...input, secretSteps: 1 });
  assert.ok(moved.failures.length > 0, "a moved difference must fail");
});

test("compareEvents: a Rust custom event without a session fails", () => {
  const input = lists();
  input.rust.requests[5] = custom("");
  assert.ok(compareEvents({ ...input, secretSteps: 1 }).failures.some((f) => /rust: 1 custom/.test(f)));
});

test("compareEvents: redaction is checked per publish", () => {
  const leak = lists();
  leak.ts.requests[1] = toolEvent("echo", "ses_one", { parameters: { text: "secret-token and [REDACTED]" } });
  assert.ok(compareEvents({ ...leak, secretSteps: 1 }).failures.some((f) => /ts: a publish still holds secret-token/.test(f)));
  const unredacted = compareEvents({ ...lists(), secretSteps: 2 });
  assert.ok(unredacted.failures.some((f) => /expected 2/.test(f)));
});

test("compareTools: key order does not matter, a description or schema change fails", () => {
  const tool = (extra = {}) => ({ name: "add", description: "Adds", inputSchema: { type: "object", properties: { a: { type: "number", format: "double" } } }, ...extra });
  const reordered = tool({ inputSchema: { properties: { a: { format: "double", type: "number" } }, type: "object" } });
  assert.deepEqual(compareTools([tool()], [reordered]), []);
  assert.equal(compareTools([tool()], [tool({ description: "Other" })]).length, 1);
  assert.equal(compareTools([tool()], [tool({ inputSchema: { type: "object" } })]).length, 1);
  assert.equal(compareTools([tool()], []).length, 1);
});

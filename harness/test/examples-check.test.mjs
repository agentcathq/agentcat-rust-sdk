// The examples check (src/examples-check.mjs) against a fake example (test/fixtures/example-fake.mjs):
// it passes a well-behaved example and fails on stdout noise, a missing publish, a bad exit status and
// a slow exit. It also refuses to start an example when the API URL is empty.
import assert from "node:assert/strict";
import { join } from "node:path";
import { test } from "node:test";

import { HARNESS } from "./support.mjs";
import { checkExample, exampleEnv } from "../src/examples-check.mjs";

const FAKE = [process.execPath, join(HARNESS, "test", "fixtures", "example-fake.mjs")];

/** Runs the check on the fake with FAKE_MODE `mode`. */
function check(mode, options = {}) {
  process.env.FAKE_MODE = mode;
  return checkExample("stdio", { command: FAKE, ...options });
}

test("a well-behaved example passes", { timeout: 60_000 }, async () => {
  const { failures } = await check("ok");
  assert.deepEqual(failures, []);
});

test("a stdout line that is not JSON-RPC fails", { timeout: 60_000 }, async () => {
  const { failures } = await check("junk");
  assert.equal(failures.length, 1, failures.join("\n"));
  assert.match(failures[0], /stdout line is not JSON-RPC: "starting up"/);
});

test("a call without a publish fails", { timeout: 60_000 }, async () => {
  const { failures } = await check("no-publish");
  assert.ok(failures.some((f) => /publishes for the tool calls/.test(f)), failures.join("\n"));
});

test("a non-zero exit status fails", { timeout: 60_000 }, async () => {
  const { failures } = await check("exit1");
  assert.ok(failures.some((f) => /exited with status 1/.test(f)), failures.join("\n"));
});

test("an exit later than the budget fails", { timeout: 60_000 }, async () => {
  const { failures } = await check("slow", { exitBudgetMs: 500 });
  assert.ok(failures.some((f) => /did not exit within 500 ms/.test(f)), failures.join("\n"));
});

test("an empty API URL refuses to start", () => {
  assert.throws(() => exampleEnv({ mockUrl: "", home: "/tmp/h" }), /AGENTCAT_API_URL is empty/);
  const env = exampleEnv({ mockUrl: "http://127.0.0.1:1", home: "/tmp/h" });
  assert.equal(env.AGENTCAT_API_URL, "http://127.0.0.1:1");
  assert.equal(env.AGENTCAT_PROJECT_ID, "proj_examples");
  assert.equal(env.HOME, "/tmp/h");
  assert.equal(env.USERPROFILE, "/tmp/h");
  assert.equal(env.AGENTCAT_ENABLED, undefined);
});

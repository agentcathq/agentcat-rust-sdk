// captureLogs puts the earlier diagnostics sink back; isolateLogFile removes its temp directory when the
// process ends, also by SIGINT or SIGTERM.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { existsSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { createInterface } from "node:readline";

import { captureLogs } from "../src/logs.mjs";

const HEAD = "[2026-10-23T12:00:00.000Z] [sdk=test] ";

/** A stand-in for the `logging` namespace of the TS core: one sink slot, as TS has. */
function fakeCore() {
  const logging = {
    sink: null,
    setDiagnosticsSink(fn) {
      logging.sink = fn;
    },
    writeToLog(message) {
      logging.sink?.(`${HEAD}${message}`);
    },
  };
  return { logging };
}

test("captureLogs collects the entries of fn, without their head, and clears the sink afterwards", async () => {
  const core = fakeCore();
  const { value, logs } = await captureLogs(core, async () => {
    core.logging.writeToLog("one");
    core.logging.writeToLog("two");
    return 7;
  });
  assert.equal(value, 7);
  assert.deepEqual(logs, ["one", "two"]);
  assert.equal(core.logging.sink, null);
});

test("a capture inside a capture hands the sink back to the outer one, also when fn throws", async () => {
  const core = fakeCore();
  const outer = await captureLogs(core, async () => {
    core.logging.writeToLog("outer");
    const inner = await captureLogs(core, async () => core.logging.writeToLog("inner"));
    assert.deepEqual(inner.logs, ["inner"]);
    await assert.rejects(
      captureLogs(core, async () => {
        throw new Error("boom");
      }),
      /boom/,
    );
    core.logging.writeToLog("outer again"); // the outer capture is active again, not a null sink
  });
  assert.deepEqual(outer.logs, ["outer", "outer again"]);
  assert.equal(core.logging.sink, null);
});

const CHILD = `
import { homedir } from "node:os";
import { appendFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { isolateLogFile } from ${JSON.stringify(new URL("../src/logs.mjs", import.meta.url).href)};
const core = { logging: { writeToLog() { appendFileSync(join(homedir(), "agentcat.log"), "x\\n"); } } };
isolateLogFile(core);
const dir = readdirSync(process.env.TMPDIR_PROBE).filter((f) => f.startsWith("oracle-log-"));
process.stdout.write(JSON.stringify({ dir: join(process.env.TMPDIR_PROBE, dir[0]) }) + "\\n");
process.stdin.on("end", () => process.exit(0));
process.stdin.resume();
setInterval(() => {}, 1000);
`;

/** Starts the child, resolves with it and the temp directory it made. */
async function startChild(t) {
  const probe = mkdtempSync(join(tmpdir(), "logs-probe-"));
  t.after(() => rmSync(probe, { recursive: true, force: true }));
  const child = spawn(process.execPath, ["--input-type=module", "-e", CHILD], {
    env: { ...process.env, TMPDIR: probe, TEMP: probe, TMP: probe, TMPDIR_PROBE: probe },
    stdio: ["pipe", "pipe", "inherit"],
  });
  t.after(() => child.kill("SIGKILL"));
  const lines = createInterface({ input: child.stdout })[Symbol.asyncIterator]();
  const { value } = await lines.next();
  return { child, dir: JSON.parse(value).dir };
}

// Windows ends a process on a signal without running any handler, so there is nothing to test there.
for (const signal of ["SIGINT", "SIGTERM"]) {
  test(`the oracle-log temp directory is removed when ${signal} ends the process`, { timeout: 3_000, skip: process.platform === "win32" }, async (t) => {
    const { child, dir } = await startChild(t);
    assert.ok(existsSync(dir), "the directory exists while the process runs");
    const exited = new Promise((resolve) => child.once("exit", (code, sig) => resolve({ code, sig })));
    child.kill(signal);
    const { code, sig } = await exited;
    assert.equal(sig, signal, `the signal still ends the process (code ${code})`);
    assert.equal(existsSync(dir), false, "the directory is gone");
  });
}

test("the oracle-log temp directory is removed on a normal exit", { timeout: 3_000 }, async (t) => {
  const { child, dir } = await startChild(t);
  const exited = new Promise((resolve) => child.once("exit", (code) => resolve(code)));
  child.stdin.end();
  assert.equal(await exited, 0);
  assert.equal(existsSync(dir), false, "the directory is gone");
});

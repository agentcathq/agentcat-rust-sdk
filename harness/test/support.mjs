// Shared paths and process helpers for the harness tests (not a test file).
import { spawn } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

export const HARNESS = fileURLToPath(new URL("..", import.meta.url));
export const DRIVER = join(HARNESS, "src", "driver.mjs");
export const DIFF = join(HARNESS, "src", "diff.mjs");
export const DUMP_CONTRACT = join(HARNESS, "src", "dump-contract.mjs");
export const GUARD = join(HARNESS, "src", "egress-guard.mjs");
export const TS_V1 = join(HARNESS, "src", "servers", "ts-v1.mjs");
export const OUT = join(HARNESS, "out");
export const BASIC = join(HARNESS, "scenarios", "basic.json");

/** The TS v1 reference server command, egress guard preloaded. */
export const TS_SERVER = [process.execPath, "--import", pathToFileURL(GUARD).href, TS_V1];

/** Runs a command to completion; resolves with its exit code and output. */
export function run(cmd, args, { env = process.env, timeoutMs = 120_000 } = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(cmd, args, { env, stdio: ["ignore", "pipe", "pipe"] });
    let stdout = "";
    let stderr = "";
    child.stdout.setEncoding("utf8").on("data", (text) => (stdout += text));
    child.stderr.setEncoding("utf8").on("data", (text) => (stderr += text));
    const timer = setTimeout(() => child.kill("SIGKILL"), timeoutMs);
    child.on("error", reject);
    child.on("close", (code, signal) => {
      clearTimeout(timer);
      resolve({ code, signal, stdout, stderr });
    });
  });
}

/** Runs the driver against the TS v1 server. */
export function driver(scenario, label, transport, extra = []) {
  return run(process.execPath, [DRIVER, scenario, label, transport, ...extra, "--", ...TS_SERVER]);
}

/** Runs diff.mjs on two normalized files. */
export function diff(a, b, extra = []) {
  return run(process.execPath, [DIFF, a, b, ...extra]);
}

/** A driver output file: harness/out/<scenario>.<label>.<transport>.<kind>.json. */
export function outFile(scenario, label, transport, kind) {
  return join(OUT, `${scenario}.${label}.${transport}.${kind}.json`);
}

export function readJson(path) {
  return JSON.parse(readFileSync(path, "utf8"));
}

export function writeJson(path, value) {
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`);
  return path;
}

/** A temp directory removed after the test file finishes. */
export function tempDir(t, prefix = "harness-test-") {
  const dir = mkdtempSync(join(tmpdir(), prefix));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  return dir;
}

/** The publish count a scenario expects on a transport. */
export function expectedFor(scenario, transport) {
  const e = scenario.expectPublishes;
  return typeof e === "number" ? e : e[transport];
}

// The log lines of the TS SDK, as the oracle sees them. TS's writeToLog (src/modules/logging.ts)
// hands every entry, `[<ISO timestamp>] [<version prefix>] <message>`, to the diagnostics sink,
// then appends it to a log file that it resolves once, on its first call, from os.homedir().
import { existsSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

/** The head writeToLog puts before each message: the timestamp and getVersionLogPrefix(). */
const HEAD = /^\[\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z\] \[sdk=[^\]]*\] /;

/**
 * Runs `fn()` (and awaits its result) while the diagnostics sink of `core.logging` collects every
 * log entry; resolves with `{value, logs}`, each log line without its head. Throws on an entry
 * without that head. The sink is cleared afterwards, also when `fn` throws.
 */
export async function captureLogs(core, fn) {
  const entries = [];
  core.logging.setDiagnosticsSink((entry) => entries.push(entry));
  let value;
  try {
    value = await fn();
  } finally {
    core.logging.setDiagnosticsSink(null);
  }
  return { value, logs: entries.map(stripHead) };
}

function stripHead(entry) {
  const head = HEAD.exec(entry);
  if (head === null) {
    throw new Error(`oracle: a TS log entry without the "[<timestamp>] [<version prefix>] " head: ${JSON.stringify(entry)}`);
  }
  return entry.slice(head[0].length);
}

const isolated = new WeakSet();

/**
 * Sends the log file of `core.logging` to a fresh temp directory, removed when the process exits,
 * so no oracle run writes to the developer's `~/agentcat.log`. It points HOME and USERPROFILE at
 * the directory for the first writeToLog call, which resolves the file, then restores both. Once
 * per core; throws when the file did not land in the directory (TS resolved it earlier).
 */
export function isolateLogFile(core) {
  if (isolated.has(core.logging)) return;
  const dir = mkdtempSync(join(tmpdir(), "oracle-log-"));
  process.once("exit", () => rmSync(dir, { recursive: true, force: true }));
  const saved = { HOME: process.env.HOME, USERPROFILE: process.env.USERPROFILE };
  process.env.HOME = dir;
  process.env.USERPROFILE = dir;
  try {
    core.logging.writeToLog("oracle: log file isolated");
  } finally {
    for (const [key, value] of Object.entries(saved)) {
      if (value === undefined) delete process.env[key];
      else process.env[key] = value;
    }
  }
  if (!existsSync(join(dir, "agentcat.log"))) {
    throw new Error(`oracle: the TS log file is not in ${dir}; TS resolved its path before isolateLogFile ran`);
  }
  isolated.add(core.logging);
}

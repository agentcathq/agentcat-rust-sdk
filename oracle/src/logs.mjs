// The log lines of the TS SDK, as the oracle sees them. TS's writeToLog (src/modules/logging.ts)
// hands every entry, `[<ISO timestamp>] [<version prefix>] <message>`, to the diagnostics sink,
// then appends it to a log file that it resolves once, on its first call, from os.homedir().
import { existsSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

/** The head writeToLog puts before each message: the timestamp and getVersionLogPrefix(). */
const HEAD = /^\[\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z\] \[sdk=[^\]]*\] /;

/** The sink that captureLogs installed on each `logging` namespace and has not yet taken off. */
const active = new WeakMap();

/**
 * Runs `fn()` (and awaits its result) while the diagnostics sink of `core.logging` collects every
 * log entry; resolves with `{value, logs}`, each log line without its head. Throws on an entry
 * without that head. Afterwards, also when `fn` throws, the sink that an enclosing captureLogs
 * installed is back, or none: TS offers no way to read a sink that set it by another route.
 */
export async function captureLogs(core, fn) {
  const entries = [];
  const earlier = active.get(core.logging) ?? null;
  const sink = (entry) => entries.push(entry);
  active.set(core.logging, sink);
  core.logging.setDiagnosticsSink(sink);
  let value;
  try {
    value = await fn();
  } finally {
    if (earlier === null) active.delete(core.logging);
    else active.set(core.logging, earlier);
    core.logging.setDiagnosticsSink(earlier);
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
/** The temp directories of isolateLogFile that are still to be removed. */
const dirs = new Set();
let removalSet = false;

function removeDirs() {
  for (const dir of dirs) rmSync(dir, { recursive: true, force: true });
  dirs.clear();
}

/**
 * Removes every directory when the process exits, or when SIGINT or SIGTERM arrives. A signal with
 * no other listener would end the process before `exit` fires, so the listener removes the
 * directories and sends the signal again; with its own listener gone, Node's default ends the
 * process. When another listener exists (the TS event queue installs one), that one ends it.
 */
function removeDirsAtEnd() {
  if (removalSet) return;
  removalSet = true;
  process.once("exit", removeDirs);
  for (const signal of ["SIGINT", "SIGTERM"]) {
    const listener = () => {
      removeDirs();
      if (process.listenerCount(signal) === 0) process.kill(process.pid, signal);
    };
    process.once(signal, listener);
  }
}

/**
 * Sends the log file of `core.logging` to a fresh temp directory, removed when the process exits or
 * a SIGINT or SIGTERM ends it,
 * so no oracle run writes to the developer's `~/agentcat.log`. It points HOME and USERPROFILE at
 * the directory for the first writeToLog call, which resolves the file, then restores both. Once
 * per core; throws when the file did not land in the directory (TS resolved it earlier).
 */
export function isolateLogFile(core) {
  if (isolated.has(core.logging)) return;
  const dir = mkdtempSync(join(tmpdir(), "oracle-log-"));
  dirs.add(dir);
  removeDirsAtEnd();
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

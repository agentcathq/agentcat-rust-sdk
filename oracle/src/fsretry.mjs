// File steps that Windows refuses while another process holds the file open: a parallel test
// process that reads it, or a virus scanner that checks a new file. The step fails with EPERM,
// EBUSY or EACCES and succeeds once the other handle closes, so the oracle tries it again.
import { readFileSync, renameSync, rmSync } from "node:fs";

/** The error codes of a file or directory that another process holds open (Windows). */
export const BUSY = new Set(["EPERM", "EBUSY", "EACCES"]);

const DEFAULTS = { attempts: 8, delayMs: 50 };

/**
 * Returns `fn()`. While `fn` throws a BUSY error, runs it again after a delay that starts at
 * `delayMs` and doubles, `attempts` runs in all (by default 8 runs within about 6.4 s); then throws
 * the last error. Throws any other error at once. Like the sync fs calls it wraps, it blocks the
 * thread while it waits.
 */
export function retryBusy(fn, { attempts = DEFAULTS.attempts, delayMs = DEFAULTS.delayMs } = {}) {
  for (let attempt = 1, delay = delayMs; ; attempt += 1, delay *= 2) {
    try {
      return fn();
    } catch (err) {
      if (!BUSY.has(err?.code) || attempt >= attempts) throw err;
    }
    sleepSync(delay);
  }
}

/**
 * renameSync(from, to), run again while it fails with EPERM, EBUSY or EACCES (retryBusy). When it
 * still fails and `to` is a file with the same bytes as `from`, another process has put the same
 * file in place first: removes `from` and returns. Otherwise throws the last error, whose message
 * names both paths (as every rename error of node:fs does), with its code.
 */
export function renameWithRetry(from, to, { attempts = DEFAULTS.attempts, delayMs = DEFAULTS.delayMs } = {}) {
  try {
    retryBusy(() => renameSync(from, to), { attempts, delayMs });
  } catch (err) {
    if (!BUSY.has(err?.code) || !sameBytes(from, to)) throw err;
    try {
      retryBusy(() => rmSync(from, { force: true }), { attempts, delayMs });
    } catch {
      // still open elsewhere: a leftover, not a failure (the file in place is the same)
    }
  }
}

/** Whether `a` and `b` are files with the same bytes; false when either cannot be read (a directory). */
function sameBytes(a, b) {
  try {
    return readFileSync(a).equals(readFileSync(b));
  } catch {
    return false;
  }
}

const sleeper = new Int32Array(new SharedArrayBuffer(4));

function sleepSync(ms) {
  Atomics.wait(sleeper, 0, 0, ms);
}

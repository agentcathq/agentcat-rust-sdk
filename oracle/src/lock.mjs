// A directory lock that serializes clone/install/build across processes.
//
// A process takes the lock by renaming a directory that already holds its
// whole owner record ({pid, host, token}) to the lock path; the rename fails
// while a lock is there. So every lock has a record from its first moment; a
// lock without a readable one is damaged (or left by an older version of this
// file). A lock is stale when its owner is dead: a dead pid on this host, any
// pid from another host (a lock restored from another machine's CI cache), or
// no readable record after `staleMs`.
//
// A waiter breaks a stale lock only while it holds a claim on that very lock:
// the file `break-<token of the judged owner>` inside it, which link() creates
// for one process only. With the claim, the waiter checks that the lock still
// holds the owner it judged stale, and only then moves the lock away. A lock
// that changed hands is never moved, not even for a moment: a holder's lock
// stays on its path until the holder removes it, so nobody can take the path
// meanwhile. The owner removes the lock only while it holds its own token.
import { randomUUID } from "node:crypto";
import {
  existsSync,
  linkSync,
  mkdirSync,
  readFileSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { hostname } from "node:os";
import { dirname, join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";

export const OWNER_FILE = "owner.json";

const DEFAULTS = { waitMs: 30 * 60_000, pollMs: 250, staleMs: 60_000 };

// rename() errors that mean "a directory is in the way" or "in use": a lock is
// there (POSIX: EEXIST, ENOTEMPTY; Windows: EPERM), or a file in the directory
// is open (Windows: EPERM, EBUSY, EACCES). The caller tries again later.
const IN_USE = new Set(["EEXIST", "ENOTEMPTY", "EPERM", "EBUSY", "EACCES"]);

/** Runs `fn` while holding the lock directory `lockDir`; waits up to `waitMs` for it. */
export async function withLock(lockDir, fn, options = {}) {
  const { waitMs, pollMs, staleMs } = { ...DEFAULTS, ...options };
  mkdirSync(dirname(lockDir), { recursive: true });
  const deadline = Date.now() + waitMs;
  let token;
  for (;;) {
    token = tryTake(lockDir);
    if (token) break;
    const owner = readOwner(lockDir);
    if (isStale(lockDir, owner, staleMs) && breakStale(lockDir, owner, staleMs)) continue;
    if (Date.now() >= deadline) {
      throw new Error(`oracle: timed out waiting for ${lockDir}; delete it if no build is running`);
    }
    await sleep(pollMs);
  }
  try {
    return await fn();
  } finally {
    release(lockDir, token);
  }
}

/** The lock's owner record, or null when it is missing, partial or invalid. */
export function readOwner(lockDir) {
  return readRecord(join(lockDir, OWNER_FILE));
}

/**
 * Removes the lock if it is still the one judged stale (`judged` is the owner
 * record read then, or null for a lock without one). Returns whether it did.
 * A lock that changed hands since stays where it is.
 */
export function breakStale(lockDir, judged, staleMs = DEFAULTS.staleMs) {
  if (judged === null && ageMs(lockDir) <= staleMs) return false;
  if (!claim(lockDir, `break-${judged === null ? "none" : judged.token}`)) return false;
  // `judged` was read before the claim was made. So if the lock still holds
  // `judged` now, the lock did not change in between, the claim is inside this
  // very lock, and no other process holds a claim on it: only this process
  // moves it. (A lock without a record carries no token to tell two of them
  // apart; only a damaged or older lock lacks one.)
  const found = readOwner(lockDir);
  const same = judged === null ? found === null : found?.token === judged.token;
  if (!same) return false; // another waiter broke it and took a new lock
  const grave = `${lockDir}.stale-${randomUUID()}`;
  try {
    renameSync(lockDir, grave);
  } catch (err) {
    if (err.code === "ENOENT" || IN_USE.has(err.code)) return false;
    throw err;
  }
  removeTree(grave);
  return true;
}

/** Whether a process with this pid runs on this host. */
export function isAlive(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch (err) {
    return err.code === "EPERM";
  }
}

function tryTake(lockDir) {
  if (existsSync(lockDir)) return null; // spares a staged directory per poll while a lock is held
  const token = randomUUID();
  const staged = `${lockDir}.new-${token}`;
  mkdirSync(staged);
  try {
    writeFileSync(join(staged, OWNER_FILE), record(token));
    renameSync(staged, lockDir);
    return token;
  } catch (err) {
    if (IN_USE.has(err.code)) return null;
    throw err;
  } finally {
    removeTree(staged); // nothing there once the rename took the lock
  }
}

/**
 * Makes this process the one breaker of the lock: creates `lockDir/<name>`
 * with this process's record, or finds it made by this process before. The
 * claim of a breaker that died (or ran on another host) passes on to
 * `<name>.<its token>`, which again only one process creates. Returns false
 * while another live process holds the claim, or when the lock is gone.
 */
function claim(lockDir, name) {
  const mine = record(randomUUID());
  const first = join(lockDir, name);
  const seen = new Set();
  for (let file = first; !seen.has(file); ) {
    seen.add(file);
    const made = createOnce(file, mine);
    if (made !== "exists") return made === "made";
    const holder = readRecord(file);
    if (holder === null) return false; // a damaged claim: leave the lock to a person
    if (holder.host === hostname()) {
      // breakStale runs to its end before this process starts another one.
      if (holder.pid === process.pid) return true;
      if (isAlive(holder.pid)) return false;
    }
    file = `${first}.${holder.token}`;
  }
  return false; // claims that point back to each other: damaged
}

/** Creates `file` with `text`, whole or not at all: "made", "exists" or "gone" (its directory). */
function createOnce(file, text) {
  const tmp = `${file}.${randomUUID()}.tmp`;
  try {
    writeFileSync(tmp, text);
    linkSync(tmp, file);
    return "made";
  } catch (err) {
    if (err.code === "EEXIST") return "exists";
    if (err.code === "ENOENT") return "gone"; // the lock was removed or replaced meanwhile
    throw err;
  } finally {
    removeTree(tmp);
  }
}

function record(token) {
  return JSON.stringify({ pid: process.pid, host: hostname(), token });
}

function readRecord(file) {
  try {
    const owner = JSON.parse(readFileSync(file, "utf8"));
    if (
      Number.isInteger(owner?.pid) &&
      owner.pid > 0 &&
      typeof owner.host === "string" &&
      typeof owner.token === "string" &&
      /^[\w-]{1,64}$/.test(owner.token) // it names claim files
    ) {
      return owner;
    }
  } catch {
    // missing, empty or half-written
  }
  return null;
}

function isStale(lockDir, owner, staleMs) {
  if (owner === null) return ageMs(lockDir) > staleMs;
  if (owner.host !== hostname()) return true;
  return !isAlive(owner.pid);
}

function ageMs(path) {
  try {
    return Date.now() - statSync(path).mtimeMs;
  } catch {
    return 0;
  }
}

function release(lockDir, token) {
  if (readOwner(lockDir)?.token !== token) return; // no longer ours
  const grave = `${lockDir}.done-${token}`;
  try {
    renameSync(lockDir, grave);
  } catch {
    return;
  }
  removeTree(grave);
}

/** Removes a file or directory that no process uses any more; what stays is litter, not a lock. */
function removeTree(path) {
  try {
    rmSync(path, { recursive: true, force: true });
  } catch {
    // Windows: a file in it is still open
  }
}

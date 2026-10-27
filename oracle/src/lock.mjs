// A directory lock that serializes clone/install/build across processes.
//
// A process takes the lock by renaming a directory that already holds its
// whole owner record ({pid, host, token}) to the lock path; the rename fails
// while a lock is there. So every lock has a record from its first moment; a
// lock without a readable one is on its way out (moveLock), damaged, or left
// by an older version of this file. While it holds the lock, the owner
// refreshes the time of the lock directory (the heartbeat). A lock is stale
// when its owner is dead (isLive): a dead pid; a record with another host name
// and an old lock (a lock restored from another machine's CI cache, whose pid
// means nothing here; an owner on this machine whose host name changed since,
// as macOS does, keeps its lock fresh); or no readable record after `staleMs`.
//
// A waiter breaks a stale lock only while it holds a claim on that very lock:
// the file `break-<token of the judged owner>` inside it, which link() creates
// for one process only. With the claim, the waiter checks that the lock still
// holds the owner it judged stale, and only then moves the lock away. A lock
// that changed hands is never moved, not even for a moment: a holder's lock
// stays on its path until the holder removes it, so nobody can take the path
// meanwhile. The owner removes the lock only while it holds its own token and
// that same claim, so the owner and a waiter that judged it stale never both
// move it.
//
// Windows refuses a rename or a link while another process (a virus scanner)
// holds a file in it open: those steps run again after a delay (fsretry.mjs),
// and a step that still fails leaves the lock to the next poll. It also refuses
// to rename a directory while any file in it is open, and every waiter reads
// the owner record at each poll; so whoever moves a lock first moves its record
// aside (moveLock). The holder sweeps what failed removals leave next to the
// lock.
import { randomUUID } from "node:crypto";
import {
  existsSync,
  linkSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  rmSync,
  statSync,
  utimesSync,
  writeFileSync,
} from "node:fs";
import { hostname } from "node:os";
import { basename, dirname, join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";

import { BUSY, renameWithRetry, retryBusy } from "./fsretry.mjs";

export const OWNER_FILE = "owner.json";

/** The owner record's name while its lock moves away (moveLock). */
const MOVING_FILE = "owner.moving";

const DEFAULTS = { waitMs: 30 * 60_000, pollMs: 250, staleMs: 60_000 };

// rename() errors that mean "a directory is in the way" or "in use": a lock is
// there (POSIX: EEXIST, ENOTEMPTY; Windows: EPERM), or a file in the directory
// is open (Windows: EPERM, EBUSY, EACCES). The caller tries again later.
const IN_USE = new Set(["EEXIST", "ENOTEMPTY", ...BUSY]);

// The token of this process's claims. A claim with it is this process's own, whatever pid it
// holds: another process can have the same pid (in another pid namespace). One per module instance.
const SELF = randomUUID();

// What a lock leaves next to its path when a removal fails (Windows: a file in it is open):
// staged directories (tryTake), broken locks (breakStale) and released ones (release).
const LEFTOVERS = [".new-", ".stale-", ".done-"];

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
    if (!isLive(owner, lockDir, staleMs) && breakStale(lockDir, owner, staleMs)) continue;
    if (Date.now() >= deadline) {
      throw new Error(`oracle: timed out waiting for ${lockDir}; delete it if no build is running`);
    }
    await sleep(pollMs);
  }
  const heartbeat = setInterval(() => touch(lockDir, token), Math.max(1, Math.floor(staleMs / 4)));
  heartbeat.unref();
  try {
    sweep(lockDir, staleMs);
    return await fn();
  } finally {
    clearInterval(heartbeat);
    release(lockDir, token, staleMs);
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
  if (!claim(lockDir, `break-${judged === null ? "none" : judged.token}`, staleMs)) return false;
  // `judged` was read before the claim was made. So if the lock still holds
  // `judged` now, the lock did not change in between, the claim is inside this
  // very lock, and no other process holds a claim on it: only this process
  // moves it. (A lock without a record carries no token to tell two of them
  // apart; only a damaged or older lock, or one whose mover died, lacks one.)
  const found = readOwner(lockDir);
  const same = judged === null ? found === null : found?.token === judged.token;
  if (!same) return false; // another waiter broke it and took a new lock
  const grave = `${lockDir}.stale-${randomUUID()}`;
  try {
    moveLock(lockDir, grave);
  } catch (err) {
    if (err.code === "ENOENT" || IN_USE.has(err.code)) return false; // the claim stays: the next poll goes on
    throw err;
  }
  removeTree(grave);
  return true;
}

/**
 * Removes the lock if it still holds the owner `token`. First it takes the claim
 * that a waiter which judged the lock stale takes (`break-<token>`); while a
 * live waiter holds that claim, the waiter moves the lock and release leaves it.
 * Exported for its test.
 */
export function release(lockDir, token, staleMs = DEFAULTS.staleMs) {
  if (readOwner(lockDir)?.token !== token) return; // no longer ours
  if (!claim(lockDir, `break-${token}`, staleMs)) return; // a breaker moves it
  const grave = `${lockDir}.done-${token}`;
  try {
    moveLock(lockDir, grave);
  } catch {
    return; // still in use: a waiter breaks it once this process has ended
  }
  removeTree(grave);
}

/**
 * Renames the lock directory to `grave`. Windows refuses to rename a directory
 * while a file in it is open, and every waiter reads the owner record at each
 * poll. So the record first moves aside, inside the lock (Windows renames a
 * file that others hold open, since Node opens files with delete sharing). A
 * waiter then finds a fresh lock without a record, which it leaves alone
 * (isLive), and the directory moves once the reads in progress have ended.
 * When the directory cannot move, the record goes back and the error is thrown.
 */
function moveLock(lockDir, grave) {
  const record = join(lockDir, OWNER_FILE);
  const aside = join(lockDir, MOVING_FILE);
  let moved = false;
  try {
    renameWithRetry(record, aside);
    moved = true;
    touchDir(lockDir); // fresh, so no waiter takes the lock without a record for a stale one
  } catch (err) {
    if (err.code !== "ENOENT") throw err; // ENOENT: a lock without a record, or none at all
  }
  try {
    renameWithRetry(lockDir, grave);
  } catch (err) {
    if (moved) {
      try {
        renameWithRetry(aside, record);
      } catch {
        // the lock stays without a record: stale after staleMs
      }
    }
    throw err;
  }
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
    renameWithRetry(staged, lockDir);
    return token;
  } catch (err) {
    if (IN_USE.has(err.code)) return null;
    throw err;
  } finally {
    removeTree(staged); // nothing there once the rename took the lock
  }
}

/**
 * Whether the owner or claim record `record` (null when unreadable) belongs to
 * a live process. `path` is the lock directory or the claim file; its time
 * gives the record's age.
 */
function isLive(record, path, staleMs) {
  if (record === null) return ageMs(path) <= staleMs;
  if (!isAlive(record.pid)) return false;
  // Another host name: another machine (a CI cache restored here; the pid means
  // nothing here), or this machine before a host rename (macOS). A live owner
  // keeps its lock fresh and a breaker holds its claim for seconds, so only an
  // old record is dead.
  return record.host === hostname() || ageMs(path) <= staleMs;
}

/**
 * Makes this process the one holder of the claim `lockDir/<name>`: creates it
 * with this process's record, or finds it made by this process before (token
 * SELF: breakStale and release run to their end before this process starts
 * another one). The claim of a dead holder (isLive) passes on to
 * `<name>.<its token>`, or to `<that file>.none` for an unreadable one, which
 * again only one process creates. Returns false while another live process
 * holds the claim, while a claim file cannot be read or made (Windows: open
 * elsewhere), or when the lock is gone.
 */
function claim(lockDir, name, staleMs) {
  const mine = record(SELF);
  const first = join(lockDir, name);
  const seen = new Set();
  for (let file = first; !seen.has(file); ) {
    seen.add(file);
    const made = createOnce(file, mine);
    if (made !== "exists") return made === "made";
    // A claim file is whole from its first moment (link), so every reader of
    // its text decides the same; one that cannot read it now tries later.
    const text = readText(file);
    if (text === null) return false;
    const holder = parseRecord(text);
    if (holder?.token === SELF) return true;
    if (isLive(holder, file, staleMs)) return false;
    file = holder === null ? `${file}.none` : `${first}.${holder.token}`;
  }
  return false; // claims that point back to each other: damaged
}

/**
 * Creates `file` with `text`, whole or not at all: "made", "exists", "gone"
 * (its directory) or "busy" (Windows still refuses it after retries).
 */
function createOnce(file, text) {
  const tmp = `${file}.${randomUUID()}.tmp`;
  try {
    retryBusy(() => writeFileSync(tmp, text));
    retryBusy(() => linkSync(tmp, file));
    return "made";
  } catch (err) {
    if (err.code === "EEXIST") return "exists";
    if (err.code === "ENOENT") return "gone"; // the lock was removed or replaced meanwhile
    if (BUSY.has(err.code)) return "busy";
    throw err;
  } finally {
    removeTree(tmp);
  }
}

function record(token) {
  return JSON.stringify({ pid: process.pid, host: hostname(), token });
}

function readRecord(file) {
  const text = readText(file);
  return text === null ? null : parseRecord(text);
}

/** The text of `file`, or null when it cannot be read now (missing, or open elsewhere on Windows). */
function readText(file) {
  try {
    return readFileSync(file, "utf8");
  } catch {
    return null;
  }
}

/** The owner or claim record in `text`, or null when `text` is not a whole, valid one. */
function parseRecord(text) {
  try {
    const owner = JSON.parse(text);
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
    // empty or half-written
  }
  return null;
}

/** The heartbeat: refreshes the time of the lock while it holds the owner `token`. */
function touch(lockDir, token) {
  if (readOwner(lockDir)?.token === token) touchDir(lockDir);
}

/** Sets the time of `dir` to now, if it can. */
function touchDir(dir) {
  const now = new Date();
  try {
    utimesSync(dir, now, now);
  } catch {
    // open elsewhere (Windows): a rename inside it has set its time too, or the next beat comes
  }
}

/**
 * Removes the leftovers of `lockDir` (LEFTOVERS) that are older than `staleMs`.
 * A younger one may still be in use: a staged directory right before its
 * rename, or a moved lock while its remover runs.
 */
function sweep(lockDir, staleMs) {
  const parent = dirname(lockDir);
  const prefixes = LEFTOVERS.map((kind) => `${basename(lockDir)}${kind}`);
  let entries;
  try {
    entries = readdirSync(parent);
  } catch {
    return;
  }
  for (const entry of entries) {
    const path = join(parent, entry);
    if (prefixes.some((prefix) => entry.startsWith(prefix)) && ageMs(path) > staleMs) removeTree(path);
  }
}

function ageMs(path) {
  try {
    return Date.now() - statSync(path).mtimeMs;
  } catch {
    return 0;
  }
}

/** Removes a file or directory that no process uses any more; what stays is litter, not a lock. */
function removeTree(path) {
  try {
    rmSync(path, { recursive: true, force: true });
  } catch {
    // Windows: a file in it is still open (the holder sweeps it later)
  }
}

// A directory lock that serializes clone/install/build across processes.
//
// `mkdir` takes the lock atomically. The owner then renames a complete record
// ({pid, host, token}) into it, so a reader sees either no record or a whole
// one. A lock is stale when its owner is dead: a dead pid on this host, any
// pid from another host (a lock restored from another machine's CI cache), or
// no readable record after `staleMs` (its creator died before writing one).
// A waiter breaks a stale lock by renaming it to a unique name first, and
// keeps it only if it is still the lock it judged stale; otherwise it gives
// it back. The owner removes the lock only while it holds its own token.
import { randomUUID } from "node:crypto";
import { mkdirSync, readFileSync, renameSync, rmSync, statSync, writeFileSync } from "node:fs";
import { hostname } from "node:os";
import { dirname, join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";

export const OWNER_FILE = "owner.json";

const DEFAULTS = { waitMs: 30 * 60_000, pollMs: 250, staleMs: 60_000 };

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
  try {
    const owner = JSON.parse(readFileSync(join(lockDir, OWNER_FILE), "utf8"));
    if (
      Number.isInteger(owner?.pid) &&
      owner.pid > 0 &&
      typeof owner.host === "string" &&
      typeof owner.token === "string"
    ) {
      return owner;
    }
  } catch {
    // missing, empty or half-written
  }
  return null;
}

/**
 * Removes the lock if it is still the one judged stale (`judged` is the owner
 * record read then, or null for a lock without one). Returns whether it did.
 */
export function breakStale(lockDir, judged, staleMs = DEFAULTS.staleMs) {
  const grave = `${lockDir}.stale-${randomUUID()}`;
  try {
    renameSync(lockDir, grave);
  } catch (err) {
    if (["ENOENT", "EPERM", "EBUSY", "EACCES"].includes(err.code)) return false;
    throw err;
  }
  const found = readOwner(grave);
  const same =
    judged === null ? found === null && ageMs(grave) > staleMs : found?.token === judged.token;
  if (!same) {
    // Another waiter broke the stale lock and took a new one in between.
    try {
      renameSync(grave, lockDir);
    } catch {
      // A third process took the lock meanwhile; the newer lock stands.
    }
    return false;
  }
  rmSync(grave, { recursive: true, force: true });
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
  try {
    mkdirSync(lockDir);
  } catch (err) {
    if (err.code === "EEXIST") return null;
    throw err;
  }
  const token = randomUUID();
  const tmp = join(lockDir, `${OWNER_FILE}.${token}.tmp`);
  try {
    writeFileSync(tmp, JSON.stringify({ pid: process.pid, host: hostname(), token }));
    renameSync(tmp, join(lockDir, OWNER_FILE));
  } catch (err) {
    if (err.code === "ENOENT") return null; // a breaker removed the new lock
    throw err;
  }
  return readOwner(lockDir)?.token === token ? token : null;
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
  rmSync(grave, { recursive: true, force: true });
}

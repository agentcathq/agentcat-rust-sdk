// The build lock in oracle/.cache serializes clone/install/build across
// processes (nextest runs one oracle server per test binary).
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import fs, {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  utimesSync,
  writeFileSync,
} from "node:fs";
import { syncBuiltinESMExports } from "node:module";
import { hostname, tmpdir } from "node:os";
import { basename, join } from "node:path";
import { after, test } from "node:test";
import { setTimeout as sleep } from "node:timers/promises";
import { fileURLToPath, pathToFileURL } from "node:url";

import { breakStale, OWNER_FILE, readOwner, withLock } from "../src/lock.mjs";

const LOCK_MODULE = fileURLToPath(new URL("../src/lock.mjs", import.meta.url));
const scratch = mkdtempSync(join(tmpdir(), "oracle-lock-"));
after(() => rmSync(scratch, { recursive: true, force: true }));

const fast = { pollMs: 10, waitMs: 400 };
let count = 0;
const newLockDir = () => join(scratch, `${++count}.lock`);

/** The pid of a process that has exited. */
function deadPid() {
  return spawnSync(process.execPath, ["-e", ""]).pid;
}

/** A lock directory left by someone else, with the given owner record. */
function plant(lockDir, record) {
  mkdirSync(lockDir);
  writeFileSync(join(lockDir, OWNER_FILE), typeof record === "string" ? record : JSON.stringify(record));
}

/** Sets the time of `path` to five minutes ago: older than the default staleMs. */
function age(path) {
  const old = new Date(Date.now() - 5 * 60_000);
  utimesSync(path, old, old);
}

/**
 * lock.mjs as a second process loads it: its own module instance, so its own claim token (the pid
 * is this process's, as for a process with the same pid in another pid namespace).
 */
async function otherProcess(name) {
  return import(`${pathToFileURL(LOCK_MODULE).href}?${name}`);
}

/**
 * Runs `fn` while `fs[name]` is `wrap(original)`. lock.mjs and fsretry.mjs import from node:fs,
 * and syncBuiltinESMExports makes those imports call the wrapper too.
 */
function stubbed(name, wrap, fn) {
  const original = fs[name];
  fs[name] = wrap(original);
  syncBuiltinESMExports();
  try {
    return fn();
  } finally {
    fs[name] = original;
    syncBuiltinESMExports();
  }
}

/** Whether a rename of `from` is the first step of moving `lockDir`: its record, or itself. */
function movesLock(lockDir, from) {
  return from === lockDir || from === join(lockDir, OWNER_FILE);
}

/** An error Windows gives for a rename while another process holds the file open. */
function refused(code, from, to) {
  return Object.assign(new Error(`${code}: rename '${from}' -> '${to}'`), { code });
}

test("a lock without a readable owner record is stale only once it is old", async () => {
  // A lock gets its record before it reaches its path, so this lock is damaged
  // (or left by an older lock.mjs, whose locks were empty for a moment).
  const lockDir = newLockDir();
  plant(lockDir, "");
  await assert.rejects(withLock(lockDir, async () => {}, fast), /timed out/);

  // Once old, the lock is broken.
  age(lockDir);
  let ran = false;
  await withLock(lockDir, async () => {
    ran = true;
  }, fast);
  assert.ok(ran);
  assert.equal(existsSync(lockDir), false);
});

test("a waiter that breaks a stale lock late leaves the lock another waiter took", async () => {
  const lockDir = newLockDir();
  plant(lockDir, { pid: deadPid(), host: hostname(), token: "dead" });
  const judged = readOwner(lockDir); // waiter A reads the dead owner...
  await withLock(lockDir, async () => {
    // ...but waiter B broke the stale lock and took it first.
    const holder = readOwner(lockDir);
    assert.notEqual(holder?.token, "dead");
    assert.equal(breakStale(lockDir, judged), false);
    assert.deepEqual(readOwner(lockDir), holder);
  }, fast);
  assert.equal(existsSync(lockDir), false);
});

// Interleaving A: a late breaker and a third waiter that takes the path whenever it is free.
test("a waiter that breaks a stale lock late never frees the path of the lock another waiter took", async () => {
  const lockDir = newLockDir();
  plant(lockDir, { pid: deadPid(), host: hostname(), token: "dead" });
  const judged = readOwner(lockDir); // waiters A and B both read the dead owner
  assert.equal(breakStale(lockDir, judged), true); // B breaks the stale lock...
  await withLock(lockDir, async () => {
    // ...and takes it. Then A breaks late, while a third waiter C takes the
    // lock path at any moment the path is free.
    const holder = readOwner(lockDir);
    const third = { pid: process.pid, host: hostname(), token: "third" };
    let thirdTook = false;
    const rename = fs.renameSync;
    fs.renameSync = (from, to) => {
      rename(from, to);
      if (from === lockDir && !existsSync(lockDir)) {
        plant(lockDir, third);
        thirdTook = true;
      }
    };
    syncBuiltinESMExports(); // lock.mjs's `renameSync` import now calls the wrapper
    try {
      assert.equal(breakStale(lockDir, judged), false);
    } finally {
      fs.renameSync = rename;
      syncBuiltinESMExports();
    }
    assert.equal(thirdTook, false, "holders overlap: the late breaker let a third waiter take a held lock");
    assert.deepEqual(readOwner(lockDir), holder);
  }, fast);
  assert.equal(existsSync(lockDir), false);
});

test("interleaving B (release during break) is deterministic", async () => {
  // The owner O runs in another process. A breaker K (this one) took O's lock for a stale one (O's
  // heartbeat stalled), and a waiter W takes the lock path at any moment it is free.
  const owner = await otherProcess("interleaving-b");
  const judged = { pid: process.pid, host: hostname(), token: "owner" };
  const waiter = { pid: process.pid, host: hostname(), token: "waiter" };

  // B1: O releases after K claimed the lock and checked its owner, right before K moves it.
  const lockDir = newLockDir();
  plant(lockDir, judged);
  let waiterTook = false;
  const broke = stubbed("renameSync", (rename) => {
    let done = false;
    return (from, to) => {
      if (!done && movesLock(lockDir, from)) {
        done = true;
        owner.release(lockDir, judged.token);
        if (!existsSync(lockDir)) {
          plant(lockDir, waiter);
          waiterTook = true;
        }
      }
      rename(from, to);
    };
  }, () => breakStale(lockDir, judged));
  assert.equal(waiterTook, false, "the owner freed the path while a breaker held the claim on its lock");
  assert.equal(broke, true, "the breaker that holds the claim moves the lock");
  assert.equal(existsSync(lockDir), false);

  // B2: K comes while O releases, right before O moves its lock.
  const lockDir2 = newLockDir();
  plant(lockDir2, judged);
  let broke2;
  let waiterTook2 = false;
  stubbed("renameSync", (rename) => {
    let done = false;
    return (from, to) => {
      if (!done && movesLock(lockDir2, from)) {
        done = true;
        broke2 = breakStale(lockDir2, judged);
        if (!existsSync(lockDir2)) {
          plant(lockDir2, waiter);
          waiterTook2 = true;
        }
      }
      rename(from, to);
    };
  }, () => owner.release(lockDir2, judged.token));
  assert.equal(broke2, false, "a breaker moved a lock that its owner was releasing");
  assert.equal(waiterTook2, false, "the path was free while the owner released");
  assert.equal(existsSync(lockDir2), false);

  // B3: O reads its record; then, before O claims, K breaks O's lock and W takes the path.
  const lockDir3 = newLockDir();
  plant(lockDir3, judged);
  let broke3;
  stubbed("readFileSync", (read) => {
    let done = false;
    return (file, ...rest) => {
      const text = read(file, ...rest);
      if (!done && file === join(lockDir3, OWNER_FILE)) {
        done = true;
        broke3 = breakStale(lockDir3, judged);
        plant(lockDir3, waiter);
      }
      return text;
    };
  }, () => owner.release(lockDir3, judged.token));
  assert.equal(broke3, true, "the breaker moves the lock it judged");
  assert.equal(readOwner(lockDir3)?.token, waiter.token, "the owner moved the lock the waiter took after it");
});

test("interleaving C (two breakers)", async () => {
  // Breakers K1 (this process) and K2 (another one) both judged the dead owner's lock stale. K2
  // comes after K1 claimed the lock and checked its owner, right before K1 moves it; a waiter W
  // takes the lock path at any moment it is free.
  const second = await otherProcess("interleaving-c");
  const lockDir = newLockDir();
  plant(lockDir, { pid: deadPid(), host: hostname(), token: "dead" });
  const judged = readOwner(lockDir);
  let secondBroke;
  let waiterTook = false;
  const broke = stubbed("renameSync", (rename) => {
    let done = false;
    return (from, to) => {
      if (!done && movesLock(lockDir, from)) {
        done = true;
        secondBroke = second.breakStale(lockDir, judged);
        if (!existsSync(lockDir)) {
          plant(lockDir, { pid: process.pid, host: hostname(), token: "waiter" });
          waiterTook = true;
        }
      }
      rename(from, to);
    };
  }, () => breakStale(lockDir, judged));
  assert.equal(secondBroke, false, "a second breaker got past the claim of the first");
  assert.equal(waiterTook, false, "the path was free while a breaker held the claim");
  assert.equal(broke, true);
  assert.equal(existsSync(lockDir), false);
});

test("a breaker that died while it broke a stale lock does not block the lock", async () => {
  const lockDir = newLockDir();
  plant(lockDir, { pid: deadPid(), host: hostname(), token: "dead" });
  // The claim of a breaker that died, then the claim of one on another host (restored from a CI
  // cache, so old; its pid, 1, is alive here and means nothing).
  writeFileSync(join(lockDir, "break-dead"), JSON.stringify({ pid: deadPid(), host: hostname(), token: "b1" }));
  writeFileSync(join(lockDir, "break-dead.b1"), JSON.stringify({ pid: 1, host: `not-${hostname()}`, token: "b2" }));
  age(join(lockDir, "break-dead.b1"));
  let ran = false;
  await withLock(lockDir, async () => {
    ran = true;
  }, fast);
  assert.ok(ran);
  assert.equal(existsSync(lockDir), false);
});

test("a breaker at work keeps other breakers off the stale lock", () => {
  const lockDir = newLockDir();
  plant(lockDir, { pid: deadPid(), host: hostname(), token: "dead" });
  const judged = readOwner(lockDir);
  // A live process (the test runner) holds the claim on the stale lock.
  writeFileSync(join(lockDir, "break-dead"), JSON.stringify({ pid: process.ppid, host: hostname(), token: "b1" }));
  assert.equal(breakStale(lockDir, judged), false);
  assert.deepEqual(readOwner(lockDir), judged);
});

test("claim holder is identified by token, not pid", () => {
  // Another breaker holds the claim: its pid is this process's (a second lock.mjs in this process,
  // or a process with the same pid in another pid namespace), its token is not.
  const lockDir = newLockDir();
  plant(lockDir, { pid: deadPid(), host: hostname(), token: "dead" });
  const judged = readOwner(lockDir);
  writeFileSync(join(lockDir, "break-dead"), JSON.stringify({ pid: process.pid, host: hostname(), token: "another" }));
  assert.equal(breakStale(lockDir, judged), false);
  assert.deepEqual(readOwner(lockDir), judged);

  // A claim this process made is its own: a break that stopped after the claim goes on later.
  const lockDir2 = newLockDir();
  plant(lockDir2, { pid: deadPid(), host: hostname(), token: "dead" });
  const judged2 = readOwner(lockDir2);
  const stopped = stubbed("renameSync", (rename) => (from, to) => {
    if (from === lockDir2) throw Object.assign(new Error(`ENOENT: rename '${from}' -> '${to}'`), { code: "ENOENT" });
    rename(from, to);
  }, () => breakStale(lockDir2, judged2));
  assert.equal(stopped, false);
  assert.equal(existsSync(join(lockDir2, "break-dead")), true);
  assert.equal(breakStale(lockDir2, judged2), true);
  assert.equal(existsSync(lockDir2), false);
});

test("unreadable claim is broken after the stale timeout", () => {
  // A claim file is whole from its first moment (link), so an unreadable one is damaged: a power
  // loss, or an older lock.mjs. Its holder cannot be known; after staleMs it counts as dead.
  const lockDir = newLockDir();
  plant(lockDir, { pid: deadPid(), host: hostname(), token: "dead" });
  const judged = readOwner(lockDir);
  writeFileSync(join(lockDir, "break-dead"), "");
  assert.equal(breakStale(lockDir, judged), false);
  assert.deepEqual(readOwner(lockDir), judged);
  age(join(lockDir, "break-dead"));
  assert.equal(breakStale(lockDir, judged), true);
  assert.equal(existsSync(lockDir), false);
});

test("a busy claim file makes the breaker try again, not throw", () => {
  // Windows refuses link() while a virus scanner holds the new file open.
  const lockDir = newLockDir();
  plant(lockDir, { pid: deadPid(), host: hostname(), token: "dead" });
  const judged = readOwner(lockDir);
  let calls = 0;
  const broke = stubbed("linkSync", (link) => (from, to) => {
    if (++calls <= 2) {
      throw Object.assign(new Error(`EPERM: operation not permitted, link '${from}' -> '${to}'`), { code: "EPERM" });
    }
    link(from, to);
  }, () => breakStale(lockDir, judged));
  assert.equal(broke, true);
  assert.equal(calls, 3);
  assert.equal(existsSync(lockDir), false);
});

test("sweep removes .new-*, .stale-* and .done-* leftovers older than the stale timeout", async () => {
  // A removal that failed (Windows: a file in it was open) leaves these next to the lock.
  const lockDir = newLockDir();
  const leftover = (name, old) => {
    const path = join(scratch, name);
    mkdirSync(path);
    writeFileSync(join(path, OWNER_FILE), "{}");
    if (old) age(path);
    return path;
  };
  const base = basename(lockDir);
  const swept = [".new-a", ".stale-b", ".done-c"].map((kind) => leftover(`${base}${kind}`, true));
  const young = [".new-d", ".stale-e", ".done-f"].map((kind) => leftover(`${base}${kind}`, false));
  const others = [leftover(`${base}.other-g`, true), leftover(`other-${base}.new-h`, true)];
  await withLock(lockDir, async () => {}, fast);
  for (const path of swept) assert.equal(existsSync(path), false, `${path} was kept`);
  for (const path of [...young, ...others]) assert.equal(existsSync(path), true, `${path} was removed`);
});

test("a lock moves away while waiters keep reading its record (Windows)", async () => {
  // Windows refuses to rename a directory while a file in it is open, and every waiter reads the
  // owner record at each poll. Here a waiter always holds the record open while it is in place.
  const owner = await otherProcess("windows-readers");
  const readersHoldRecord = (rename) => (from, to) => {
    if (existsSync(join(from, OWNER_FILE))) throw refused("EPERM", from, to);
    rename(from, to);
  };

  // The owner releases its lock.
  const lockDir = newLockDir();
  plant(lockDir, { pid: process.pid, host: hostname(), token: "owner" });
  stubbed("renameSync", readersHoldRecord, () => owner.release(lockDir, "owner"));
  assert.equal(existsSync(lockDir), false, "the owner could not release its lock");

  // A waiter breaks a stale lock.
  const lockDir2 = newLockDir();
  plant(lockDir2, { pid: deadPid(), host: hostname(), token: "dead" });
  const judged = readOwner(lockDir2);
  assert.equal(stubbed("renameSync", readersHoldRecord, () => breakStale(lockDir2, judged)), true);
  assert.equal(existsSync(lockDir2), false);

  // When the directory still cannot move, the record goes back: the lock stays whole. (The error is
  // one that is not retried, so the test does not wait out the retries.)
  const lockDir3 = newLockDir();
  plant(lockDir3, { pid: process.pid, host: hostname(), token: "owner" });
  const directoryStays = (rename) => (from, to) => {
    if (from === lockDir3) throw Object.assign(new Error(`EXDEV: rename '${from}' -> '${to}'`), { code: "EXDEV" });
    rename(from, to);
  };
  stubbed("renameSync", directoryStays, () => owner.release(lockDir3, "owner"));
  assert.equal(readOwner(lockDir3)?.token, "owner");
});

test("a holder never removes a lock that is no longer its own", async () => {
  const lockDir = newLockDir();
  const other = { pid: process.pid, host: hostname(), token: "other" };
  await withLock(lockDir, async () => {
    rmSync(lockDir, { recursive: true });
    plant(lockDir, other);
  }, fast);
  assert.deepEqual(readOwner(lockDir), other);
});

test("a lock restored from another machine's cache is stale", async () => {
  const lockDir = newLockDir();
  // The pid is alive here, but it belongs to another host; the cache kept the lock's old time.
  plant(lockDir, { pid: process.pid, host: `not-${hostname()}`, token: "elsewhere" });
  age(lockDir);
  let ran = false;
  await withLock(lockDir, async () => {
    ran = true;
  }, fast);
  assert.ok(ran);
});

test("a changed host name does not make a live claim stale", async () => {
  // macOS renames hosts: records written before the rename carry the old name.
  const renamed = `old-${hostname()}`;

  // The lock of a live owner.
  const lockDir = newLockDir();
  plant(lockDir, { pid: process.pid, host: renamed, token: "owner" });
  await assert.rejects(withLock(lockDir, async () => {}, fast), /timed out/);
  assert.equal(readOwner(lockDir)?.token, "owner");

  // The claim of a live breaker.
  const lockDir2 = newLockDir();
  plant(lockDir2, { pid: deadPid(), host: hostname(), token: "dead" });
  const judged = readOwner(lockDir2);
  writeFileSync(join(lockDir2, "break-dead"), JSON.stringify({ pid: process.pid, host: renamed, token: "breaker" }));
  assert.equal(breakStale(lockDir2, judged), false);
  assert.deepEqual(readOwner(lockDir2), judged);

  // An owner that holds its lock for longer than staleMs keeps it fresh (the heartbeat, every
  // staleMs / 4: a stall of the event loop under 3/4 of staleMs leaves the lock fresh).
  const lockDir3 = newLockDir();
  const staleMs = 300;
  await withLock(lockDir3, async () => {
    const owner = readOwner(lockDir3);
    writeFileSync(join(lockDir3, OWNER_FILE), JSON.stringify({ ...owner, host: renamed }));
    await sleep(4 * staleMs);
    await assert.rejects(withLock(lockDir3, async () => {}, { pollMs: 10, waitMs: 3 * staleMs, staleMs }), /timed out/);
  }, { pollMs: 10, staleMs });
  assert.equal(existsSync(lockDir3), false);
});

test("rename retries EPERM then succeeds", async () => {
  const { renameWithRetry } = await import("../src/fsretry.mjs");
  const from = join(scratch, "retry-1.tmp");
  const to = join(scratch, "retry-1");
  writeFileSync(from, "bundle");
  let calls = 0;
  stubbed("renameSync", (rename) => (a, b) => {
    if (a === from && ++calls <= 2) throw refused("EPERM", a, b); // a virus scanner holds the file, twice
    rename(a, b);
  }, () => renameWithRetry(from, to, { delayMs: 1 }));
  assert.equal(calls, 3);
  assert.equal(readFileSync(to, "utf8"), "bundle");
  assert.equal(existsSync(from), false);
});

test("a rename that stays refused yields only to the same bytes", async () => {
  const { renameWithRetry } = await import("../src/fsretry.mjs");
  const from = join(scratch, "retry-2.tmp");
  const to = join(scratch, "retry-2");
  const refuseFrom = (rename, seen) => (a, b) => {
    if (a !== from) return rename(a, b);
    seen.push(a);
    throw Object.assign(refused("EBUSY", a, b), { attempt: seen.length });
  };
  const thrice = () => renameWithRetry(from, to, { attempts: 3, delayMs: 1 });

  // Another process renamed the same bundle into place first.
  writeFileSync(from, "bundle");
  writeFileSync(to, "bundle");
  const tries = [];
  stubbed("renameSync", (rename) => refuseFrom(rename, tries), thrice);
  assert.equal(tries.length, 3);
  assert.equal(existsSync(from), false);
  assert.equal(readFileSync(to, "utf8"), "bundle");

  // Other bytes at `to`: the last error, and `from` stays.
  writeFileSync(from, "another bundle");
  const tries2 = [];
  assert.throws(
    () => stubbed("renameSync", (rename) => refuseFrom(rename, tries2), thrice),
    (err) => err.code === "EBUSY" && err.attempt === 3,
  );
  assert.equal(readFileSync(from, "utf8"), "another bundle");

  // Any other error at once, naming both paths.
  const missing = join(scratch, "retry-missing");
  assert.throws(
    () => renameWithRetry(missing, to, { delayMs: 1 }),
    (err) => err.code === "ENOENT" && err.message.includes(missing) && err.message.includes(to),
  );
});

/**
 * Starts 8 processes that find a stale lock at the same moment and hold it in turn for 25 ms; checks
 * that each held it once, one at a time, and that the lock is gone at the end. `nodeArgs` go before
 * each process's script.
 */
async function takeTurns(nodeArgs = []) {
  const lockDir = newLockDir();
  const log = join(scratch, `${basename(lockDir)}.holders.log`);
  plant(lockDir, { pid: deadPid(), host: hostname(), token: "dead" });
  const worker = `
    import { appendFileSync } from "node:fs";
    import { setTimeout as sleep } from "node:timers/promises";
    const { withLock } = await import(${JSON.stringify(pathToFileURL(LOCK_MODULE).href)});
    const [lockDir, log, id, goAt] = process.argv.slice(1);
    await sleep(Math.max(0, Number(goAt) - Date.now()));
    await withLock(lockDir, async () => {
      appendFileSync(log, "enter " + id + "\\n");
      await sleep(25);
      appendFileSync(log, "exit " + id + "\\n");
    }, { pollMs: 5, waitMs: 20000 });
  `;
  const workers = 8;
  const goAt = Date.now() + 750;
  const exits = await Promise.all(
    Array.from({ length: workers }, (_, id) => {
      const child = spawn(
        process.execPath,
        [...nodeArgs, "--input-type=module", "-e", worker, lockDir, log, String(id), String(goAt)],
        { stdio: ["ignore", "ignore", "pipe"] },
      );
      let stderr = "";
      child.stderr.setEncoding("utf8").on("data", (text) => {
        stderr += text;
      });
      return new Promise((resolve) => child.on("close", (code) => resolve({ code, stderr })));
    }),
  );
  assert.deepEqual(exits, Array.from({ length: workers }, () => ({ code: 0, stderr: "" })));
  const lines = readFileSync(log, "utf8").trim().split("\n");
  assert.equal(lines.length, 2 * workers);
  for (let i = 0; i < lines.length; i += 2) {
    const [enter, id] = lines[i].split(" ");
    assert.deepEqual([enter, lines[i + 1]], ["enter", `exit ${id}`], `holders overlap:\n${lines.join("\n")}`);
  }
  assert.equal(existsSync(lockDir), false);
}

test("processes that find a stale lock at the same moment take it one at a time", async () => {
  await takeTurns();
});

test("processes take the lock one at a time where a lock cannot move while waiters read its record", async () => {
  // Windows refuses to rename a directory while a file in it is open; with 5 ms polls, some waiter
  // always has the record of the lock open (CI run 40582589401: 7 of 8 processes timed out).
  const rule = join(scratch, "windows-rename-rule.mjs");
  writeFileSync(
    rule,
    `
    import fs from "node:fs";
    import { join } from "node:path";
    import { syncBuiltinESMExports } from "node:module";
    const rename = fs.renameSync;
    fs.renameSync = (from, to) => {
      if (String(from).endsWith(".lock") && fs.existsSync(join(from, ${JSON.stringify(OWNER_FILE)}))) {
        throw Object.assign(new Error(\`EPERM: rename '\${from}' -> '\${to}'\`), { code: "EPERM" });
      }
      rename(from, to);
    };
    syncBuiltinESMExports();
    `,
  );
  await takeTurns(["--import", pathToFileURL(rule).href]);
});

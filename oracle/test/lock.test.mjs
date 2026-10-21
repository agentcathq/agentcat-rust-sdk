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
import { join } from "node:path";
import { after, test } from "node:test";
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

test("a lock without a readable owner record is stale only once it is old", async () => {
  // A lock gets its record before it reaches its path, so this lock is damaged
  // (or left by an older lock.mjs, whose locks were empty for a moment).
  const lockDir = newLockDir();
  plant(lockDir, "");
  await assert.rejects(withLock(lockDir, async () => {}, fast), /timed out/);

  // Once old, the lock is broken.
  const old = new Date(Date.now() - 5 * 60_000);
  utimesSync(lockDir, old, old);
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

test("a breaker that died while it broke a stale lock does not block the lock", async () => {
  const lockDir = newLockDir();
  plant(lockDir, { pid: deadPid(), host: hostname(), token: "dead" });
  // The claim of a breaker that died, then the claim of one on another host.
  writeFileSync(join(lockDir, "break-dead"), JSON.stringify({ pid: deadPid(), host: hostname(), token: "b1" }));
  writeFileSync(join(lockDir, "break-dead.b1"), JSON.stringify({ pid: 1, host: `not-${hostname()}`, token: "b2" }));
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
  // The pid is alive here, but it belongs to another host.
  plant(lockDir, { pid: process.pid, host: `not-${hostname()}`, token: "elsewhere" });
  let ran = false;
  await withLock(lockDir, async () => {
    ran = true;
  }, fast);
  assert.ok(ran);
});

test("processes that find a stale lock at the same moment take it one at a time", async () => {
  const lockDir = newLockDir();
  const log = join(scratch, "holders.log");
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
        ["--input-type=module", "-e", worker, lockDir, log, String(id), String(goAt)],
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
});

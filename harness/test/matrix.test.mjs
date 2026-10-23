// The matrix runner: arguments, and one A/A combination on the TS v2 server.
import assert from "node:assert/strict";
import { join } from "node:path";
import { before, test } from "node:test";

import { ensureTs } from "oracle/fetch-ts";

import { HARNESS, OUT, readJson, run } from "./support.mjs";

const MATRIX = join(HARNESS, "src", "matrix.mjs");
const matrix = (args, env = {}) => run(process.execPath, [MATRIX, ...args], { env: { ...process.env, ...env }, timeoutMs: 300_000 });

before(async () => {
  await ensureTs();
});

test("an unknown server name exits 2", async () => {
  const res = await matrix(["--servers", "ts-v2,nope"]);
  assert.equal(res.code, 2);
  assert.match(res.stderr, /unknown server "nope"/);
});

test("the rust server without a binary exits 2 and names the expected path", async () => {
  const missing = join(HARNESS, "out", "no-such-rust-server");
  const res = await matrix(["--servers", "ts-v2,rust"], { HARNESS_RUST_SERVER: missing });
  assert.equal(res.code, 2);
  assert.ok(res.stderr.includes(missing), res.stderr);
});

test("an unknown scenario, transport or lane exits 2", async () => {
  assert.equal((await matrix(["--scenarios", "no-such"])).code, 2);
  assert.equal((await matrix(["--transports", "carrier-pigeon"])).code, 2);
  assert.equal((await matrix(["--lanes", "1999-01-01"])).code, 2);
  assert.equal((await matrix(["--jobs", "0"])).code, 2);
});

test("A/A of ts-v2 on basic, stdio, 2025-11-25 has one combination and no delta difference", { timeout: 300_000 }, async () => {
  const res = await matrix(["--servers", "ts-v2,ts-v2", "--scenarios", "basic", "--transports", "stdio", "--lanes", "2025-11-25"]);
  assert.equal(res.code, 0, res.stdout + res.stderr);
  const doc = readJson(join(OUT, "matrix.json"));
  assert.equal(doc.length, 1);
  assert.equal(doc[0].delta.unexpected, 0);
  assert.deepEqual(Object.values(doc[0].exitCodes), [0, 0, 0, 0]);
  assert.match(res.stdout, /^ok .*basic.*stdio.*2025-11-25/m);
});

// The matrix runner: arguments, and one A/A combination on the TS v2 server.
import assert from "node:assert/strict";
import { join } from "node:path";
import { before, test } from "node:test";

import { ensureTs } from "oracle/fetch-ts";

import { HARNESS, OUT, readJson, run, tempDir, writeJson } from "./support.mjs";

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

test("--env2 needs NAME=value", async () => {
  for (const bad of ["HARNESS_CANARY", "=1", "1X=1", "A-B=1"]) {
    const res = await matrix(["--env2", bad]);
    assert.equal(res.code, 2, bad);
    assert.match(res.stderr, /--env2 needs NAME=value/, bad);
  }
});

test("--env2 reaches the second server only: HARNESS_CANARY=1 makes a Rust A/A combination differ", { timeout: 300_000 }, async () => {
  const one = ["--servers", "rust,rust", "--scenarios", "basic", "--transports", "stdio", "--lanes", "2025-11-25"];
  let res = await matrix(one);
  assert.equal(res.code, 0, res.stdout + res.stderr);
  assert.equal(readJson(join(OUT, "matrix.json"))[0].delta.total, 0);

  res = await matrix([...one, "--env2", "HARNESS_CANARY=1"]);
  assert.equal(res.code, 1, res.stdout + res.stderr);
  const [entry] = readJson(join(OUT, "matrix.json"));
  assert.deepEqual(entry.env2, { HARNESS_CANARY: "1" });
  assert.deepEqual(Object.values(entry.exitCodes), [0, 0, 0, 0]);
  assert.ok(entry.delta.unexpected > 0, JSON.stringify(entry.delta));
  // The canary changes the injected context description, which only the tracked listing carries.
  assert.ok(entry.differences.every((d) => d.path.startsWith("$.delta.transcript[1].result.tools[")), res.stdout);
});

test("a ruling whose scenario is not a scenario name, or names no scenario file, exits 2", async (t) => {
  const dir = tempDir(t);
  const ruling = { path: "\\$\\.delta\\.nothing", reason: "test", deviation: "RD-90" };
  for (const [scenario, why] of [
    [5, /"scenario" must be a scenario name/],
    ["../basic", /"scenario" must be a scenario name/],
    ["no-such", /RD-90 names the scenario "no-such"/],
  ]) {
    const rulings = writeJson(join(dir, "rulings.json"), [{ ...ruling, scenario }]);
    const res = await matrix(["--servers", "rust,rust", "--rulings", rulings]);
    assert.equal(res.code, 2, `${scenario}: ${res.stdout}${res.stderr}`);
    assert.match(res.stderr, why);
  }
});

test("a scenario-scoped ruling matches its own scenario's differences only; one whose scenario did not run is not reported", { timeout: 300_000 }, async (t) => {
  const tools = "\\$\\.delta\\.transcript\\[\\d+\\]\\.result\\.tools\\[\\d+\\].*";
  const rulings = writeJson(join(tempDir(t), "rulings.json"), [
    { path: tools, scenario: "basic", reason: "the canary, in basic only", deviation: "RD-90" },
    { path: tools, scenario: "tracing-off", reason: "a scenario this run leaves out", deviation: "RD-91" },
  ]);
  const res = await matrix([
    ...["--servers", "rust,rust", "--scenarios", "basic,schemas", "--transports", "stdio", "--lanes", "2025-11-25"],
    ...["--env2", "HARNESS_CANARY=1", "--rulings", rulings],
  ]);
  // The same canary differences in both scenarios: ruled in basic, unruled in schemas.
  assert.equal(res.code, 1, res.stdout + res.stderr);
  const [basic, schemas] = readJson(join(OUT, "matrix.json"));
  assert.equal(basic.scenario, "basic");
  assert.ok(basic.delta.total > 0 && basic.delta.ruled === basic.delta.total, JSON.stringify(basic.delta));
  assert.equal(schemas.scenario, "schemas");
  assert.ok(schemas.delta.unexpected > 0 && schemas.delta.ruled === 0, JSON.stringify(schemas.delta));
  assert.match(res.stdout, new RegExp(`^RULING RD-90 matched ${basic.delta.ruled} in basic: `, "m"));
  assert.match(res.stdout, /^FAIL schemas stdio 2025-11-25: /m);
  assert.doesNotMatch(res.stdout, /RD-91/);
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

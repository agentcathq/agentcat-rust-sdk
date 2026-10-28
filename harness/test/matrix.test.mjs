// The matrix runner: arguments, and one A/A combination on the TS v2 server.
import assert from "node:assert/strict";
import { readdirSync } from "node:fs";
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
  // The published events stay the same: the driver's arguments do not come from the description.
  assert.deepEqual(entry.events, { total: 0, ruled: 0, unexpected: 0 });
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

test("a ruling whose transports or lanes are malformed or name an unknown transport or lane exits 2", async (t) => {
  const dir = tempDir(t);
  const ruling = { path: "\\$\\.delta\\.nothing", reason: "test", deviation: "RD-90" };
  for (const [scope, why] of [
    [{ transports: "http" }, /\[0\]: "transports" must be a non-empty array of transport names/],
    [{ transports: [] }, /\[0\]: "transports" must be a non-empty array of transport names/],
    [{ transports: [1] }, /\[0\]: "transports" must be a non-empty array of transport names/],
    [{ transports: ["stdio", "carrier-pigeon"] }, /\[0\]: "transports" names an unknown transport "carrier-pigeon"/],
    [{ lanes: "2026-07-28" }, /\[0\]: "lanes" must be a non-empty array of lane names/],
    [{ lanes: [null] }, /\[0\]: "lanes" must be a non-empty array of lane names/],
    [{ lanes: ["1999-01-01"] }, /\[0\]: "lanes" names an unknown lane "1999-01-01"/],
  ]) {
    const rulings = writeJson(join(dir, "rulings.json"), [{ ...ruling, ...scope }]);
    const res = await matrix(["--servers", "rust,rust", "--rulings", rulings]);
    assert.equal(res.code, 2, `${JSON.stringify(scope)}: ${res.stdout}${res.stderr}`);
    assert.match(res.stderr, why);
  }
});

test("a ruling scoped to transports and lanes matches on its own transport and lane only; one whose transport did not run is not reported", { timeout: 300_000 }, async (t) => {
  const tools = "\\$\\.delta\\.transcript\\[\\d+\\]\\.result\\.tools\\[\\d+\\].*";
  const rulings = writeJson(join(tempDir(t), "rulings.json"), [
    { path: tools, transports: ["stdio"], lanes: ["2026-07-28"], reason: "the canary, on stdio at 2026-07-28 only", deviation: "RD-90" },
    { path: tools, transports: ["http-stateless"], reason: "a transport this run leaves out", deviation: "RD-91" },
  ]);
  const res = await matrix([
    ...["--servers", "rust,rust", "--scenarios", "basic", "--transports", "stdio,http", "--lanes", "2025-11-25,2026-07-28"],
    ...["--env2", "HARNESS_CANARY=1", "--rulings", rulings],
  ]);
  // The same canary differences in every combination: ruled on stdio at 2026-07-28 only.
  assert.equal(res.code, 1, res.stdout + res.stderr);
  const doc = readJson(join(OUT, "matrix.json"));
  assert.equal(doc.length, 4);
  const at = (transport, lane) => doc.find((e) => e.transport === transport && e.lane === lane).delta;
  const own = at("stdio", "2026-07-28");
  assert.ok(own.total > 0 && own.ruled === own.total, JSON.stringify(own));
  for (const [transport, lane] of [["stdio", "2025-11-25"], ["http", "2026-07-28"], ["http", "2025-11-25"]]) {
    const other = at(transport, lane);
    assert.ok(other.unexpected > 0 && other.ruled === 0, `${transport} ${lane}: ${JSON.stringify(other)}`);
  }
  assert.match(res.stdout, new RegExp(`^RULING RD-90 matched ${own.ruled} on stdio at 2026-07-28: `, "m"));
  assert.match(res.stdout, /^ok basic stdio 2026-07-28: /m);
  assert.match(res.stdout, /^FAIL basic stdio 2025-11-25: /m);
  assert.match(res.stdout, /^FAIL basic http 2026-07-28: /m);
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

test("a Rust A/A run skips the TS-versus-Rust rulings: it prints no ruling line at all", { timeout: 300_000 }, async () => {
  const res = await matrix(["--servers", "rust,rust", "--scenarios", "basic", "--transports", "stdio", "--lanes", "2025-11-25"]);
  assert.equal(res.code, 0, res.stdout + res.stderr);
  assert.doesNotMatch(res.stdout, /RULING/);
  assert.doesNotMatch(res.stdout, /^EVENT /m);
});

// ---------------------------------------------------------------------------
// Task 12: the event comparison.
// ---------------------------------------------------------------------------
const TS_RUST_ONE = ["--servers", "ts-v2,rust", "--transports", "stdio", "--lanes", "2025-11-25"];

test("an event difference without a ruling exits 1; harness/rulings-events.json rules every one", { timeout: 300_000 }, async (t) => {
  const empty = writeJson(join(tempDir(t), "rulings-events.json"), []);
  let res = await matrix([...TS_RUST_ONE, "--scenarios", "basic", "--event-rulings", empty]);
  assert.equal(res.code, 1, res.stdout + res.stderr);
  let [entry] = readJson(join(OUT, "matrix.json"));
  assert.deepEqual(Object.values(entry.exitCodes), [0, 0, 0, 0]);
  assert.equal(entry.delta.unexpected, 0, JSON.stringify(entry.delta));
  assert.ok(entry.events.unexpected > 0 && entry.events.ruled === 0, JSON.stringify(entry.events));
  assert.equal(entry.eventDifferences.length, entry.events.unexpected);
  assert.ok(entry.eventDifferences.every((d) => d.path.startsWith("$.events[")), JSON.stringify(entry.eventDifferences));
  const { total, ruled, unexpected } = entry.events;
  assert.match(res.stdout, new RegExp(`^FAIL basic stdio 2025-11-25: delta \\{.*\\} events ${total}/${ruled}/${unexpected}$`, "m"));
  assert.match(res.stdout, /^ {2}EVENT \$\.events\[\d+\]\.body\.parameters\.extra\.<keys>: /m);

  // The default event rulings (Rust against TS) rule them all.
  res = await matrix([...TS_RUST_ONE, "--scenarios", "basic"]);
  assert.equal(res.code, 0, res.stdout + res.stderr);
  [entry] = readJson(join(OUT, "matrix.json"));
  assert.ok(entry.events.total > 0 && entry.events.ruled === entry.events.total, JSON.stringify(entry.events));
  assert.equal(entry.eventDifferences, undefined);
});

test("an event ruling scoped to another scenario rules nothing here and, when its scenario does not run, is not reported", { timeout: 300_000 }, async (t) => {
  const any = "\\$\\.events\\[\\d+\\]\\..*";
  const rulings = writeJson(join(tempDir(t), "rulings-events.json"), [
    { path: any, scenario: "basic", reason: "every event difference, in basic only", deviation: "RD-90" },
    { path: any, scenario: "tracing-off", reason: "a scenario this run leaves out", deviation: "RD-91" },
  ]);
  const res = await matrix([...TS_RUST_ONE, "--scenarios", "basic,features-off", "--event-rulings", rulings]);
  assert.equal(res.code, 1, res.stdout + res.stderr);
  const [basic, featuresOff] = readJson(join(OUT, "matrix.json"));
  assert.ok(basic.events.total > 0 && basic.events.ruled === basic.events.total, JSON.stringify(basic.events));
  assert.ok(featuresOff.events.unexpected > 0 && featuresOff.events.ruled === 0, JSON.stringify(featuresOff.events));
  assert.match(res.stdout, new RegExp(`^EVENT RULING RD-90 matched ${basic.events.ruled} in basic: `, "m"));
  assert.match(res.stdout, /^ok basic stdio 2025-11-25: /m);
  assert.match(res.stdout, /^FAIL features-off stdio 2025-11-25: /m);
  assert.doesNotMatch(res.stdout, /RD-91/);
});

test("a whole-matrix run fails on an event ruling that matched nothing and names it", { timeout: 600_000 }, async (t) => {
  // Rust A/A over every scenario, transport and lane: no event difference, so the one event ruling stays
  // unused. No delta rulings: the default RD-29 delta rulings would stay unused too.
  const dir = tempDir(t);
  const noDelta = writeJson(join(dir, "rulings.json"), []);
  const unused = writeJson(join(dir, "rulings-events.json"), [
    { path: "\\$\\.events\\[\\d+\\]\\.body\\.never", reason: "a ruling no difference matches", deviation: "RD-90" },
  ]);
  const res = await run(process.execPath, [MATRIX, "--servers", "rust,rust", "--rulings", noDelta, "--event-rulings", unused], {
    timeoutMs: 600_000,
  });
  assert.equal(res.code, 1, res.stdout + res.stderr);
  const summary = readJson(join(OUT, "matrix.json"));
  // Every scenario file, on 4 transports and 2 lanes.
  const combinations = readdirSync(join(HARNESS, "scenarios")).filter((f) => f.endsWith(".json")).length * 4 * 2;
  assert.equal(summary.length, combinations);
  assert.ok(summary.every((e) => e.events.total === 0 && e.delta.total === 0), JSON.stringify(summary.map((e) => [e.events, e.delta])));
  assert.equal(res.stdout.match(/^ok /gm)?.length, combinations, res.stdout);
  assert.doesNotMatch(res.stdout, /^FAIL /m);
  assert.match(res.stdout, /^EVENT UNUSED RULING RD-90 matched 0: /m);
  assert.equal(res.stdout.match(/UNUSED RULING/g)?.length, 1, res.stdout);
});

test("--event-rulings: a malformed file or a scenario that names no scenario file exits 2", async (t) => {
  const dir = tempDir(t);
  const notArray = writeJson(join(dir, "object.json"), { path: "x" });
  let res = await matrix(["--servers", "rust,rust", "--event-rulings", notArray]);
  assert.equal(res.code, 2, res.stdout + res.stderr);
  const noScenario = writeJson(join(dir, "no-scenario.json"), [{ path: "\\$\\.events", reason: "test", deviation: "RD-90", scenario: "no-such" }]);
  res = await matrix(["--servers", "rust,rust", "--event-rulings", noScenario]);
  assert.equal(res.code, 2, res.stdout + res.stderr);
  assert.match(res.stderr, /RD-90 names the scenario "no-such"/);
});

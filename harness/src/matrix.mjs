// The matrix runner: for each scenario, transport and lane it runs two servers untracked and tracked
// through the driver, then compares the two MCP-wire deltas (see delta.mjs).
//
// usage: node harness/src/matrix.mjs [--servers <a>,<b>] [--scenarios <names>] [--transports <list>]
//          [--lanes <list>] [--jobs <n>] [--rulings <file>]
//   --servers     two names from ts-v1, ts-v2, rust (default ts-v2,rust). `rust` runs HARNESS_RUST_SERVER,
//                 else target/debug/harness-ref-server; a missing binary exits 2 before any driver starts.
//   --scenarios   names resolved as harness/scenarios/<name>.json (default: every scenario file)
//   --transports  default stdio,http,http-stateless
//   --lanes       default 2025-11-25,2026-07-28
//   --jobs        drivers running at the same time (default 4)
//   --rulings     default harness/rulings-ts.json when both servers are TS, else harness/rulings.json
// Per combination: both servers untracked (HARNESS_NO_TRACK=1, --expect-publishes 0) and tracked (the
// `rust` server tracked also gets --expect-publishes 0 until plan 4 publishes). Labels are
// <server>-<lane>-<u|t>; when both servers have the same name the second one's labels end in -b.
// Output: harness/out/matrix.json (one entry per combination: driver exit codes, delta summary) and one
// line per combination: `ok|FAIL <scenario> <transport> <lane>: ...`.
// Exit 0: every driver passed and every delta difference is ruled (unused rulings count only when the
//         whole matrix ran, that is, without --scenarios, --transports and --lanes).
// Exit 1: a driver failed, an unruled delta difference, or (whole matrix) an unused ruling.
// Exit 2: bad arguments.
import { spawn } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, writeFileSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { compareDeltas, documentDelta, printEntries } from "./delta.mjs";
import { loadRulings, readJson, rulingLines } from "./rulings.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const HARNESS = join(HERE, "..");
const REPO = join(HARNESS, "..");
const OUT = join(HARNESS, "out");
const GUARD = pathToFileURL(join(HERE, "egress-guard.mjs")).href;
const TRANSPORTS = ["stdio", "http", "http-stateless"];
const LANES = ["2025-11-25", "2026-07-28"];
const SERVERS = ["ts-v1", "ts-v2", "rust"];
const USAGE =
  "usage: node harness/src/matrix.mjs [--servers <a>,<b>] [--scenarios <names>] [--transports <list>] [--lanes <list>] [--jobs <n>] [--rulings <file>]";

function usage(message) {
  process.stderr.write(`matrix: ${message}\n${USAGE}\n`);
  process.exit(2);
}

function parseArgs(argv) {
  const opts = {};
  for (let i = 0; i < argv.length; i += 2) {
    const flag = argv[i];
    if (!["--servers", "--scenarios", "--transports", "--lanes", "--jobs", "--rulings"].includes(flag)) usage(`unknown argument ${JSON.stringify(flag)}`);
    if (i + 1 >= argv.length) usage(`${flag} needs a value`);
    opts[flag.slice(2)] = argv[i + 1];
  }
  return opts;
}

const list = (text, flag) => {
  const items = text.split(",");
  if (items.some((s) => s === "")) usage(`${flag}: empty item in ${JSON.stringify(text)}`);
  return items;
};

function oneOf(items, allowed, what) {
  for (const x of items) if (!allowed.includes(x)) usage(`unknown ${what} ${JSON.stringify(x)}; expected one of ${allowed.join(", ")}`);
  return items;
}

/** The server command, or a bad-arguments exit for an unknown name or a missing Rust binary. */
function serverCommand(name) {
  if (name === "rust") {
    const path = process.env.HARNESS_RUST_SERVER || join(REPO, "target", "debug", "harness-ref-server");
    if (!existsSync(path)) usage(`the rust server binary is missing: expected ${path} (build it, or set HARNESS_RUST_SERVER)`);
    return [path];
  }
  return [process.execPath, "--import", GUARD, join(HERE, "servers", `${name}.mjs`)];
}

/** Runs the driver; resolves {code, tail} (the last lines of its output). */
function runDriver({ scenarioPath, label, transport, lane, expectPublishes, tracked, command }) {
  const args = [join(HERE, "driver.mjs"), scenarioPath, label, transport, "--lane", lane];
  if (expectPublishes !== undefined) args.push("--expect-publishes", String(expectPublishes));
  args.push("--", ...command);
  const env = { ...process.env };
  if (tracked) delete env.HARNESS_NO_TRACK;
  else env.HARNESS_NO_TRACK = "1";
  return new Promise((resolveRun) => {
    const child = spawn(process.execPath, args, { env, stdio: ["ignore", "pipe", "pipe"] });
    let text = "";
    for (const stream of [child.stdout, child.stderr]) stream.setEncoding("utf8").on("data", (d) => (text += d));
    child.on("error", (err) => resolveRun({ code: 1, tail: err.message }));
    child.on("close", (code) => resolveRun({ code: code ?? 1, tail: text.trim().split("\n").slice(-20).join("\n") }));
  });
}

/** Runs `tasks` (functions returning promises) with at most `limit` in flight. */
async function pool(tasks, limit) {
  const results = new Array(tasks.length);
  let next = 0;
  const worker = async () => {
    while (next < tasks.length) {
      const i = next++;
      results[i] = await tasks[i]();
    }
  };
  await Promise.all(Array.from({ length: Math.min(limit, tasks.length) }, worker));
  return results;
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  const names = list(opts.servers ?? "ts-v2,rust", "--servers");
  if (names.length !== 2) usage("--servers needs exactly two names");
  for (const n of names) if (!SERVERS.includes(n)) usage(`unknown server ${JSON.stringify(n)}; expected one of ${SERVERS.join(", ")}`);
  const transports = oneOf(list(opts.transports ?? TRANSPORTS.join(","), "--transports"), TRANSPORTS, "transport");
  const lanes = oneOf(list(opts.lanes ?? LANES.join(","), "--lanes"), LANES, "lane");
  const scenarioDir = join(HARNESS, "scenarios");
  const scenarios = opts.scenarios
    ? list(opts.scenarios, "--scenarios")
    : readdirSync(scenarioDir).filter((f) => f.endsWith(".json")).map((f) => basename(f, ".json")).sort();
  for (const s of scenarios) {
    if (!/^[A-Za-z0-9_-]+$/.test(s) || !existsSync(join(scenarioDir, `${s}.json`))) usage(`unknown scenario ${JSON.stringify(s)}: no ${join(scenarioDir, `${s}.json`)}`);
  }
  const jobs = opts.jobs === undefined ? 4 : Number(opts.jobs);
  if (!Number.isInteger(jobs) || jobs < 1) usage(`--jobs needs a positive integer, got ${JSON.stringify(opts.jobs)}`);
  const bothTs = names.every((n) => n.startsWith("ts-"));
  const rulingsPath = opts.rulings ? resolve(opts.rulings) : join(HARNESS, bothTs ? "rulings-ts.json" : "rulings.json");
  const rulings = loadRulings(rulingsPath, usage);
  const commands = names.map(serverCommand);
  const whole = !opts.scenarios && !opts.transports && !opts.lanes;

  mkdirSync(OUT, { recursive: true });
  const combos = [];
  for (const scenario of scenarios) for (const transport of transports) for (const lane of lanes) combos.push({ scenario, transport, lane });

  // Four drivers per combination: server index 0 or 1, untracked or tracked.
  const same = names[0] === names[1];
  const runs = [];
  for (const combo of combos) {
    combo.runs = [];
    names.forEach((name, s) => {
      for (const tracked of [false, true]) {
        const label = `${name}-${combo.lane}-${tracked ? "t" : "u"}${same && s === 1 ? "-b" : ""}`;
        const run = { combo, name, label, tracked, server: s };
        combo.runs.push(run);
        runs.push(run);
      }
    });
  }
  await pool(
    runs.map((run) => async () => {
      run.result = await runDriver({
        scenarioPath: join(scenarioDir, `${run.combo.scenario}.json`),
        label: run.label,
        transport: run.combo.transport,
        lane: run.combo.lane,
        expectPublishes: !run.tracked || run.name === "rust" ? 0 : undefined,
        tracked: run.tracked,
        command: commands[run.server],
      });
    }),
    jobs,
  );

  const summary = [];
  let failed = false;
  for (const combo of combos) {
    const exitCodes = Object.fromEntries(combo.runs.map((r) => [r.label, r.result.code]));
    const entry = { scenario: combo.scenario, transport: combo.transport, lane: combo.lane, servers: names, exitCodes };
    const driversOk = combo.runs.every((r) => r.result.code === 0);
    let line;
    if (!driversOk) {
      entry.driverOutput = Object.fromEntries(combo.runs.filter((r) => r.result.code !== 0).map((r) => [r.label, r.result.tail]));
      failed = true;
      line = `FAIL ${combo.scenario} ${combo.transport} ${combo.lane}: driver exit ${JSON.stringify(exitCodes)}`;
    } else {
      const doc = (r) => readJson(join(OUT, `${combo.scenario}.${r.label}.${combo.transport}.normalized.json`), usage);
      const [aU, aT, bU, bT] = combo.runs.map(doc);
      const entries = compareDeltas(documentDelta(aU, aT), documentDelta(bU, bT), rulings);
      const lines = [];
      const { ruled, unexpected } = printEntries(entries, (l) => lines.push(l));
      entry.delta = { total: entries.length, ruled, unexpected };
      if (unexpected > 0) {
        failed = true;
        entry.differences = entries.filter((e) => e.ruledBy.length === 0);
      }
      line = `${unexpected > 0 ? "FAIL" : "ok"} ${combo.scenario} ${combo.transport} ${combo.lane}: delta ${JSON.stringify(entry.delta)}`;
      if (unexpected > 0) line += `\n${lines.filter((l) => l.startsWith("DELTA")).map((l) => `  ${l}`).join("")}`.trimEnd();
    }
    summary.push(entry);
    process.stdout.write(`${line}\n`);
  }
  writeFileSync(join(OUT, "matrix.json"), `${JSON.stringify(summary, null, 2)}\n`);
  for (const l of rulingLines(rulings)) process.stdout.write(l);
  if (whole && rulings.some((r) => r.matched === 0)) failed = true;
  process.exit(failed ? 1 : 0);
}

main().catch((err) => {
  process.stderr.write(`matrix: ${err.stack ?? err}\n`);
  process.exit(1);
});

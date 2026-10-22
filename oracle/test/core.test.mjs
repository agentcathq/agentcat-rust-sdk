// loadCore bundles the pure TS modules into one file, so they share one logging instance; the
// oracle captures that instance's log lines and keeps its log file out of the developer's home.
import assert from "node:assert/strict";
import { existsSync, mkdtempSync, realpathSync, rmSync, symlinkSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { after, test } from "node:test";
import { pathToFileURL } from "node:url";

import { build } from "esbuild";

import { loadCore, sourceImportMeta } from "../src/bundle.mjs";
import { ensureTs } from "../src/fetch-ts.mjs";
import { captureLogs } from "../src/logs.mjs";
import { tsDir } from "../src/paths.mjs";

// Every log line of this process would land in $HOME/agentcat.log unless loadCore isolates it.
// HOME points at an empty directory before the first loadCore, so a missing isolation shows up
// there and never in the developer's real home. ensureTs runs first, with the real HOME, because a
// first build runs corepack, which keeps its cache under HOME.
await ensureTs();
const home = mkdtempSync(join(tmpdir(), "oracle-home-"));
const saved = { HOME: process.env.HOME, USERPROFILE: process.env.USERPROFILE };
process.env.HOME = home;
process.env.USERPROFILE = home;
after(() => {
  for (const [key, value] of Object.entries(saved)) {
    if (value === undefined) delete process.env[key];
    else process.env[key] = value;
  }
  rmSync(home, { recursive: true, force: true });
});

const NAMES = [
  "KSUID",
  "constants",
  "handles",
  "listWrap",
  "logging",
  "redaction",
  "registry",
  "sanitization",
  "tokenEstimate",
  "tools",
  "truncation",
  "validation",
];

test("loadCore returns the twelve core names, and KSUID is the TS default export", async () => {
  const core = await loadCore();
  assert.deepEqual(Object.keys(core).sort(), NAMES);
  assert.equal(core.KSUID.withPrefix("ses").fromParts(14e11, Buffer.alloc(16)), "ses_000000000000000000000000000");
});

test("captureLogs returns the value and the log lines without their timestamp and version head", async () => {
  const core = await loadCore();
  const captured = await captureLogs(core, () => core.validation.validateTags({ "a/b": "x" }));
  assert.deepEqual(captured, {
    value: null,
    logs: ['Dropping invalid tag: "a/b" — key contains invalid characters or is empty'],
  });
});

test("captureLogs rejects an entry without the head, and clears the sink also when fn throws", async () => {
  let sink = null;
  const fake = { logging: { setDiagnosticsSink: (fn) => (sink = fn) } };
  await assert.rejects(
    captureLogs(fake, () => sink("no head")),
    /without the "\[<timestamp>\] \[<version prefix>\] " head: "no head"/,
  );
  assert.equal(sink, null);
  await assert.rejects(
    captureLogs(fake, () => {
      throw new Error("boom");
    }),
    /boom/,
  );
  assert.equal(sink, null);
});

test("a call that logs leaves no agentcat.log in HOME", async () => {
  const core = await loadCore();
  assert.equal(core.validation.validateTags({ "a/b": "x" }), null); // logs one "Dropping invalid tag" line
  assert.equal(existsSync(join(home, "agentcat.log")), false);
});

test("the import.meta rewrite works when the clone is reached through a symlink", async (t) => {
  const dir = mkdtempSync(join(tmpdir(), "oracle-link-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const link = join(dir, "ts");
  symlinkSync(tsDir(), link, "dir");
  const result = await build({
    entryPoints: [join(link, "src", "modules", "logging.ts")],
    absWorkingDir: link,
    format: "esm",
    platform: "node",
    bundle: true,
    write: false,
    logLevel: "silent",
    plugins: [sourceImportMeta(join(link, "src"))],
  });
  const source = realpathSync(join(tsDir(), "src", "modules", "logging.ts"));
  assert.ok(result.outputFiles[0].text.includes(JSON.stringify(pathToFileURL(source).href)));
});

test("the import.meta rewrite leaves a module that is not a file on disk to its own plugin", async (t) => {
  const dir = mkdtempSync(join(tmpdir(), "oracle-virtual-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const virtual = {
    name: "virtual",
    setup(esbuild) {
      esbuild.onResolve({ filter: /^virtual:/ }, (args) => ({ path: args.path, namespace: "virtual" }));
      esbuild.onLoad({ filter: /.*/, namespace: "virtual" }, () => ({ contents: "export const x = 41 + 1;" }));
    },
  };
  const result = await build({
    stdin: { contents: 'export { x } from "virtual:x.js";', resolveDir: dir },
    format: "esm",
    bundle: true,
    write: false,
    logLevel: "silent",
    plugins: [sourceImportMeta(dir), virtual],
  });
  assert.match(result.outputFiles[0].text, /41 \+ 1/);
});

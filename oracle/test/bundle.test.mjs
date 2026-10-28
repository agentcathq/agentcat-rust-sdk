// Several processes build the same bundle at once (nextest runs one oracle server per test binary,
// and each one bundles what it is missing). Windows refuses a rename onto a file that another process
// holds open (the nightly failure of run 40495080931: EPERM on core.mjs), so the last steps of a build
// race there; every process must still end with the bundle.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { after, test } from "node:test";
import { pathToFileURL } from "node:url";

import { loadCore } from "../src/bundle.mjs";
import { ensureTs } from "../src/fetch-ts.mjs";

const BUNDLE_MODULE = new URL("../src/bundle.mjs", import.meta.url).href;
const scratch = mkdtempSync(join(tmpdir(), "oracle-bundle-"));
after(() => rmSync(scratch, { recursive: true, force: true }));

const CHILD = `
  import { setTimeout as sleep } from "node:timers/promises";
  const { loadModule } = await import(${JSON.stringify(BUNDLE_MODULE)});
  const [dir, goAt, retry] = process.argv.slice(1);
  await sleep(Math.max(0, Number(goAt) - Date.now()));
  const tokens = await loadModule("tokenEstimate", { dir, retry: JSON.parse(retry) });
  if (tokens.estimateInputTokens({ n: 1.0 }) !== 2) throw new Error("the bundle gives a wrong result");
`;

/**
 * Starts `children` processes that load the tokenEstimate bundle from `dir` at the same moment;
 * resolves with their exit codes and stderr. `nodeArgs` go before the script; `retry` is the retry
 * budget of each rename (fsretry.mjs), by default the oracle's own.
 */
async function buildInParallel(dir, children, { nodeArgs = [], retry = {} } = {}) {
  const goAt = Date.now() + 1500; // every child has started and imported bundle.mjs by then
  return Promise.all(
    Array.from({ length: children }, () => {
      const args = [...nodeArgs, "--input-type=module", "-e", CHILD, dir, String(goAt), JSON.stringify(retry)];
      const proc = spawn(process.execPath, args, { stdio: ["ignore", "ignore", "pipe"] });
      let stderr = "";
      proc.stderr.setEncoding("utf8").on("data", (text) => {
        stderr += text;
      });
      return new Promise((resolve) => proc.on("close", (code) => resolve({ code, stderr })));
    }),
  );
}

function assertAllExited(exits) {
  assert.deepEqual(
    exits.map(({ code }) => code),
    exits.map(() => 0),
    exits.map(({ stderr }) => stderr).join("\n"),
  );
}

test("parallel bundle builds converge", async () => {
  await ensureTs(); // the clone is built once here, so the children only bundle
  const dir = join(scratch, "bundles"); // empty: it does not exist yet
  assertAllExited(await buildInParallel(dir, 6));
  const tokens = await import(pathToFileURL(join(dir, "tokenEstimate.mjs")).href);
  assert.equal(tokens.estimateInputTokens({ n: 1.0 }), 2);
});

test("parallel bundle builds converge where a rename onto a bundle in place is refused", async () => {
  // Windows refuses a rename onto a file that another process holds open; here every rename onto
  // a bundle that is already in place is refused, so each child that comes second keeps the copy
  // in place and removes its own (renameWithRetry, after its retries). The retries wait 50 and
  // 100 ms here, so the waits of the test (1.5 s to start, then these) stay under 3 s.
  await ensureTs();
  const dir = join(scratch, "refused");
  const refuse = join(scratch, "refuse-rename.mjs");
  writeFileSync(
    refuse,
    `
    import fs from "node:fs";
    import { syncBuiltinESMExports } from "node:module";
    const rename = fs.renameSync;
    fs.renameSync = (from, to) => {
      if (String(to).endsWith(".mjs") && fs.existsSync(to)) {
        process.stderr.write("refused\\n");
        const message = \`EPERM: operation not permitted, rename '\${from}' -> '\${to}'\`;
        throw Object.assign(new Error(message), { code: "EPERM" });
      }
      rename(from, to);
    };
    syncBuiltinESMExports();
    `,
  );
  const exits = await buildInParallel(dir, 6, {
    nodeArgs: ["--import", pathToFileURL(refuse).href],
    retry: { attempts: 3, delayMs: 50 },
  });
  assertAllExited(exits);
  assert.ok(exits.some(({ stderr }) => stderr.includes("refused")), "no child came second: the race was not run");
  assert.deepEqual(readdirSync(dir), ["tokenEstimate.mjs"]);
  const tokens = await import(pathToFileURL(join(dir, "tokenEstimate.mjs")).href);
  assert.equal(tokens.estimateInputTokens({ n: 1.0 }), 2);
});

test("the header of bundle.mjs lists every module loadCore bundles", async () => {
  const text = readFileSync(new URL("../src/bundle.mjs", import.meta.url), "utf8");
  const header = text.slice(0, text.indexOf("\nimport "));
  const core = await loadCore();
  const names = Object.keys(core);
  assert.ok(names.includes("exceptions") && names.includes("session") && names.includes("sdk"), names.join(", "));
  for (const name of names) assert.match(header, new RegExp(`\\b${name}\\b`), `the header does not name ${name}`);
});

test("the doc of loadModule says that its bundles are not log-isolated", () => {
  const text = readFileSync(new URL("../src/bundle.mjs", import.meta.url), "utf8");
  const doc = text.slice(text.lastIndexOf("/**", text.indexOf("export async function loadModule")), text.indexOf("export async function loadModule"));
  assert.match(doc, /not log-isolated/);
});

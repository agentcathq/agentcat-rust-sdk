// Loads one module of the TS SDK source at the locked commit: esbuild bundles
// it (with everything it imports) into a standalone ESM file once, and later
// loads import that file.
import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

import { build, version as esbuildVersion } from "esbuild";

import { ensureTs, verifyTs } from "./fetch-ts.mjs";
import { bundleDir, tsDir } from "./paths.mjs";

// First line of every bundle. Bump BUNDLE_FORMAT when the build options change,
// so bundles cached under an unchanged SHA are rebuilt.
const BUNDLE_FORMAT = 1;
const STAMP = `// oracle bundle ${BUNDLE_FORMAT}, esbuild ${esbuildVersion}\n`;

const loaded = new Map();

/**
 * Imports `src/modules/<name>.ts` (or `src/thirdparty/ksuid/index.js` for
 * "ksuid") of the pinned TS SDK, bundled into `<bundleDir>/<name>.mjs`.
 */
export async function loadModule(name) {
  if (typeof name !== "string" || !/^[A-Za-z0-9_-]+$/.test(name)) {
    throw new Error(`oracle: invalid TS module name ${JSON.stringify(name)}`);
  }
  let pending = loaded.get(name);
  if (!pending) {
    pending = load(name);
    loaded.set(name, pending);
    pending.catch(() => loaded.delete(name));
  }
  return pending;
}

function source(name) {
  return name === "ksuid"
    ? join(tsDir(), "src", "thirdparty", "ksuid", "index.js")
    : join(tsDir(), "src", "modules", `${name}.ts`);
}

async function load(name) {
  await ensureTs();
  const out = join(bundleDir(), `${name}.mjs`);
  if (!isCurrent(out)) {
    const entry = source(name);
    if (!existsSync(entry)) {
      throw new Error(`oracle: the TS SDK has no module "${name}" (${entry} does not exist)`);
    }
    const result = await build({
      entryPoints: [entry],
      absWorkingDir: tsDir(),
      outfile: out,
      format: "esm",
      platform: "node",
      bundle: true,
      write: false,
      logLevel: "silent",
    });
    // Every later process reuses the bundle, so the clone must still be clean
    // now, after the build, not only when ensureTs() ran.
    await verifyTs();
    mkdirSync(bundleDir(), { recursive: true });
    // Write then rename, so a concurrent or interrupted run never leaves half a file.
    const tmp = `${out}.tmp-${process.pid}`;
    writeFileSync(tmp, STAMP + result.outputFiles[0].text);
    renameSync(tmp, out);
  }
  return import(pathToFileURL(out).href);
}

function isCurrent(file) {
  try {
    return readFileSync(file, "utf8").startsWith(STAMP);
  } catch {
    return false;
  }
}

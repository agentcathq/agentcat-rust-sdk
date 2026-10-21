// Loads the TS SDK source at the locked commit: esbuild bundles an entry (with
// everything it imports) into a standalone ESM file once, and later loads import
// that file. loadCore bundles the pure modules together, so they share one
// instance of every module they import (one logging state); loadModule bundles
// one module on its own.
import { existsSync, mkdirSync, readFileSync, realpathSync, renameSync, writeFileSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { dirname, isAbsolute, join, relative } from "node:path";
import { pathToFileURL } from "node:url";

import { build, transform, version as esbuildVersion } from "esbuild";

import { ensureTs, verifyTs } from "./fetch-ts.mjs";
import { isolateLogFile } from "./logs.mjs";
import { bundleDir, tsDir } from "./paths.mjs";

// First line of every bundle. Bump BUNDLE_FORMAT when the build options change,
// so bundles cached under an unchanged SHA are rebuilt. The clone's path is in
// it too: bundles embed source paths, so a cache restored elsewhere rebuilds.
// The last part names the entry, so a change to CORE_MODULES rebuilds core.mjs.
const BUNDLE_FORMAT = 3;
const stamp = (entry) => `// oracle bundle ${BUNDLE_FORMAT}, esbuild ${esbuildVersion}, ${tsDir()}, ${entry}\n`;

/** The namespaces of loadCore besides KSUID: `src/modules/<name>.ts` each. */
const CORE_MODULES = [
  "constants",
  "handles",
  "tools",
  "tokenEstimate",
  "validation",
  "truncation",
  "sanitization",
  "redaction",
  "logging",
];
const CORE_ENTRY = [
  ...CORE_MODULES.map((name) => `export * as ${name} from "./src/modules/${name}.ts";`),
  'export { default as KSUID } from "./src/thirdparty/ksuid/index.js";',
].join("\n");

const loaded = new Map();
let core;

/**
 * Imports the pure modules of the pinned TS SDK as one bundle, `<bundleDir>/core.mjs`: an object
 * with one namespace per name of CORE_MODULES and `KSUID`, the default export of
 * `src/thirdparty/ksuid/index.js`. Every namespace uses the same `logging` instance, whose log
 * file isolateLogFile (logs.mjs) has moved out of the developer's home.
 */
export async function loadCore() {
  if (!core) {
    core = buildCore();
    core.catch(() => {
      core = undefined;
    });
  }
  return core;
}

async function buildCore() {
  await ensureTs();
  const entry = { stdin: { contents: CORE_ENTRY, resolveDir: tsDir(), sourcefile: "oracle-core.js", loader: "js" } };
  const mod = await bundle("core", `core: ${CORE_MODULES.join(" ")} KSUID`, entry);
  isolateLogFile(mod);
  return mod;
}

/**
 * Imports `src/modules/<name>.ts` (or `src/thirdparty/ksuid/index.js` for
 * "ksuid") of the pinned TS SDK, bundled into `<bundleDir>/<name>.mjs`.
 *
 * In the SDK's own files (under `src/`), `import.meta.url`, `.dirname` and
 * `.filename` name the source file, as when Node runs it in place, so
 * `createRequire(import.meta.url)` resolves from the clone's node_modules.
 * Bundled dependencies keep their code unchanged: in them, `import.meta.url`
 * is the bundle's URL. loadCore's bundle is built the same way.
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
  const entry = source(name);
  if (!existsSync(entry)) {
    throw new Error(`oracle: the TS SDK has no module "${name}" (${entry} does not exist)`);
  }
  return bundle(name, relative(tsDir(), entry), { entryPoints: [entry] });
}

/** Builds `<bundleDir>/<out>.mjs` from `entryOptions` unless it is current, then imports it. */
async function bundle(out, entryName, entryOptions) {
  const file = join(bundleDir(), `${out}.mjs`);
  const head = stamp(entryName);
  if (!isCurrent(file, head)) {
    const result = await build({
      ...entryOptions,
      absWorkingDir: tsDir(),
      outfile: file,
      format: "esm",
      platform: "node",
      bundle: true,
      write: false,
      logLevel: "silent",
      plugins: [sourceImportMeta(join(tsDir(), "src"))],
    });
    // Every later process reuses the bundle, so the clone must still be clean
    // now, after the build, not only when ensureTs() ran.
    await verifyTs();
    mkdirSync(bundleDir(), { recursive: true });
    // Write then rename, so a concurrent or interrupted run never leaves half a file.
    const tmp = `${file}.tmp-${process.pid}`;
    writeFileSync(tmp, head + result.outputFiles[0].text);
    renameSync(tmp, file);
  }
  return import(pathToFileURL(file).href);
}

/**
 * esbuild plugin: in each file under `srcDir` that uses `import.meta`, replace
 * `import.meta.url`, `.dirname` and `.filename` with that file's own values.
 * esbuild's `transform` does it on the syntax tree, so strings and comments
 * stay as they are; the clone's tsconfig gives the same TS settings as the build.
 * `srcDir` and each file are compared as real paths: esbuild hands the plugin
 * real paths, and `srcDir` may be reached through a symlink. Exported for its test.
 */
export function sourceImportMeta(srcDir) {
  const realSrcDir = realpathSync(srcDir);
  const tsconfig = join(dirname(srcDir), "tsconfig.json");
  const tsconfigRaw = existsSync(tsconfig) ? readFileSync(tsconfig, "utf8") : undefined;
  return {
    name: "oracle-source-import-meta",
    setup(esbuild) {
      esbuild.onLoad({ filter: /\.[cm]?[jt]s$/ }, async (args) => {
        const path = realpathSync(args.path);
        const rel = relative(realSrcDir, path);
        if (rel === "" || rel.startsWith("..") || isAbsolute(rel)) return undefined;
        const text = await readFile(path, "utf8");
        if (!text.includes("import.meta")) return undefined;
        const { code } = await transform(text, {
          loader: /\.[cm]?ts$/.test(path) ? "ts" : "js",
          format: "esm",
          sourcefile: path,
          tsconfigRaw,
          define: {
            "import.meta.url": JSON.stringify(pathToFileURL(path).href),
            "import.meta.dirname": JSON.stringify(dirname(path)),
            "import.meta.filename": JSON.stringify(path),
          },
        });
        return { contents: code, loader: "js" };
      });
    },
  };
}

function isCurrent(file, head) {
  try {
    return readFileSync(file, "utf8").startsWith(head);
  } catch {
    return false;
  }
}

// Loads one module of the TS SDK source at the locked commit: esbuild bundles
// it (with everything it imports) into a standalone ESM file once, and later
// loads import that file.
import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { dirname, isAbsolute, join, relative } from "node:path";
import { pathToFileURL } from "node:url";

import { build, transform, version as esbuildVersion } from "esbuild";

import { ensureTs, verifyTs } from "./fetch-ts.mjs";
import { bundleDir, tsDir } from "./paths.mjs";

// First line of every bundle. Bump BUNDLE_FORMAT when the build options change,
// so bundles cached under an unchanged SHA are rebuilt. The clone's path is in
// it too: bundles embed source paths, so a cache restored elsewhere rebuilds.
const BUNDLE_FORMAT = 2;
const stamp = () => `// oracle bundle ${BUNDLE_FORMAT}, esbuild ${esbuildVersion}, ${tsDir()}\n`;

const loaded = new Map();

/**
 * Imports `src/modules/<name>.ts` (or `src/thirdparty/ksuid/index.js` for
 * "ksuid") of the pinned TS SDK, bundled into `<bundleDir>/<name>.mjs`.
 *
 * In the SDK's own files (under `src/`), `import.meta.url`, `.dirname` and
 * `.filename` name the source file, as when Node runs it in place, so
 * `createRequire(import.meta.url)` resolves from the clone's node_modules.
 * Bundled dependencies keep their code unchanged: in them, `import.meta.url`
 * is the bundle's URL.
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
      plugins: [sourceImportMeta(join(tsDir(), "src"))],
    });
    // Every later process reuses the bundle, so the clone must still be clean
    // now, after the build, not only when ensureTs() ran.
    await verifyTs();
    mkdirSync(bundleDir(), { recursive: true });
    // Write then rename, so a concurrent or interrupted run never leaves half a file.
    const tmp = `${out}.tmp-${process.pid}`;
    writeFileSync(tmp, stamp() + result.outputFiles[0].text);
    renameSync(tmp, out);
  }
  return import(pathToFileURL(out).href);
}

/**
 * esbuild plugin: in each file under `srcDir` that uses `import.meta`, replace
 * `import.meta.url`, `.dirname` and `.filename` with that file's own values.
 * esbuild's `transform` does it on the syntax tree, so strings and comments
 * stay as they are; the clone's tsconfig gives the same TS settings as the build.
 */
function sourceImportMeta(srcDir) {
  const tsconfig = join(dirname(srcDir), "tsconfig.json");
  const tsconfigRaw = existsSync(tsconfig) ? readFileSync(tsconfig, "utf8") : undefined;
  return {
    name: "oracle-source-import-meta",
    setup(esbuild) {
      esbuild.onLoad({ filter: /\.[cm]?[jt]s$/ }, async (args) => {
        const rel = relative(srcDir, args.path);
        if (rel === "" || rel.startsWith("..") || isAbsolute(rel)) return undefined;
        const text = await readFile(args.path, "utf8");
        if (!text.includes("import.meta")) return undefined;
        const { code } = await transform(text, {
          loader: /\.[cm]?ts$/.test(args.path) ? "ts" : "js",
          format: "esm",
          sourcefile: args.path,
          tsconfigRaw,
          define: {
            "import.meta.url": JSON.stringify(pathToFileURL(args.path).href),
            "import.meta.dirname": JSON.stringify(dirname(args.path)),
            "import.meta.filename": JSON.stringify(args.path),
          },
        });
        return { contents: code, loader: "js" };
      });
    },
  };
}

function isCurrent(file) {
  try {
    return readFileSync(file, "utf8").startsWith(stamp());
  } catch {
    return false;
  }
}

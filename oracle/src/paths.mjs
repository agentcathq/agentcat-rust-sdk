// Where the oracle keeps the pinned TS SDK clone and the modules bundled from it.
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ORACLE_DIR = join(dirname(fileURLToPath(import.meta.url)), "..");
const LOCK_FILE = join(ORACLE_DIR, "oracle.lock");
const CACHE_DIR = join(ORACLE_DIR, ".cache");

/** The TS repository and the commit the oracle is pinned to (`oracle/oracle.lock`). */
export function lock() {
  const { repo, sha } = JSON.parse(readFileSync(LOCK_FILE, "utf8"));
  if (typeof repo !== "string" || repo === "") {
    throw new Error(`${LOCK_FILE}: "repo" must be a non-empty string`);
  }
  if (typeof sha !== "string" || !/^[0-9a-f]{40}$/.test(sha)) {
    throw new Error(`${LOCK_FILE}: "sha" must be a full 40-character lowercase commit SHA`);
  }
  return { repo, sha };
}

/** The clone of the TS repository at the locked commit: `oracle/.cache/ts-<sha>`. */
export function tsDir() {
  return join(CACHE_DIR, `ts-${lock().sha}`);
}

/** The ESM build of the TS SDK inside the clone: `<tsDir>/dist/index.mjs`. */
export function tsDist() {
  return join(tsDir(), "dist", "index.mjs");
}

/** Where `loadModule` writes bundled TS modules: `oracle/.cache/bundle-<sha>`. */
export function bundleDir() {
  return join(CACHE_DIR, `bundle-${lock().sha}`);
}

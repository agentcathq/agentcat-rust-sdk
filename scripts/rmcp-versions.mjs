// Prints the JSON list of the rmcp 3.x legs CI tests: the highest patch of each minor, each with the
// rmcp-macros version to test it with. Run as a script, it reads the crates.io API; the tests import
// `latestPatches` and `withMacros` only.
import { pathToFileURL } from "node:url";

const API = "https://crates.io/api/v1/crates";
const USER_AGENT = "agentcat-rust-sdk rmcp-versions (https://github.com/agentcat-ai/agentcat-rust-sdk)";

/**
 * The highest patch of each 3.x minor: no pre-release, no yanked version.
 * @param {{num: string, yanked: boolean}[]} versions
 * @returns {string[]} sorted by minor, so 3.10.x follows 3.9.x
 */
export function latestPatches(versions) {
  const best = new Map();
  for (const { num, yanked } of versions) {
    const match = /^3\.(\d+)\.(\d+)$/.exec(num);
    if (yanked || match === null) continue;
    const minor = Number(match[1]);
    const patch = Number(match[2]);
    if (!best.has(minor) || best.get(minor).patch < patch) best.set(minor, { patch, num });
  }
  return [...best.entries()].sort((a, b) => a[0] - b[0]).map(([, { num }]) => num);
}

/**
 * Pairs each rmcp version with the highest rmcp-macros of the same minor (latestPatches rules).
 * rmcp and rmcp-macros release patches apart, so an rmcp patch may have no rmcp-macros of the same
 * number; rmcp asks for `^<its own version>` of rmcp-macros, which the highest patch of the minor meets.
 * @param {string[]} rmcpVersions 3.x versions, such as latestPatches of rmcp's
 * @param {{num: string, yanked: boolean}[]} macrosVersions the crates.io versions of rmcp-macros
 * @returns {{version: string, macros: string}[]}
 * @throws when a minor has no rmcp-macros release
 */
export function withMacros(rmcpVersions, macrosVersions) {
  const minor = (num) => num.split(".").slice(0, 2).join(".");
  const macrosByMinor = new Map(latestPatches(macrosVersions).map((num) => [minor(num), num]));
  return rmcpVersions.map((version) => {
    const macros = macrosByMinor.get(minor(version));
    if (macros === undefined) {
      throw new Error(`rmcp-versions: no rmcp-macros ${minor(version)}.x release for rmcp ${version}`);
    }
    return { version, macros };
  });
}

async function getJson(url) {
  let lastError;
  for (let attempt = 0; attempt < 3; attempt += 1) {
    try {
      const response = await fetch(url, { headers: { "User-Agent": USER_AGENT } });
      if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`);
      return await response.json();
    } catch (error) {
      lastError = error;
    }
  }
  throw lastError;
}

async function fetchVersions(crate) {
  const versions = [];
  let url = `${API}/${crate}/versions?per_page=100`;
  while (url !== null) {
    const page = await getJson(url);
    versions.push(...page.versions);
    const next = page.meta?.next_page;
    url = next ? new URL(next, `${API}/${crate}/versions`).href : null;
  }
  return versions;
}

if (process.argv[1] !== undefined && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const list = latestPatches(await fetchVersions("rmcp"));
  if (list.length === 0) {
    console.error("rmcp-versions: no 3.x version found");
    process.exit(1);
  }
  console.log(JSON.stringify(withMacros(list, await fetchVersions("rmcp-macros"))));
}

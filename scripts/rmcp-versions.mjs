// Prints the JSON list of the rmcp 3.x versions CI tests: the highest patch of each minor.
// Run as a script, it reads the crates.io API; the tests import `latestPatches` only.
import { pathToFileURL } from "node:url";

const URL_FIRST_PAGE = "https://crates.io/api/v1/crates/rmcp/versions?per_page=100";
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

async function fetchVersions() {
  const versions = [];
  let url = URL_FIRST_PAGE;
  while (url !== null) {
    const page = await getJson(url);
    versions.push(...page.versions);
    const next = page.meta?.next_page;
    url = next ? new URL(next, "https://crates.io/api/v1/crates/rmcp/versions").href : null;
  }
  return versions;
}

if (process.argv[1] !== undefined && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const list = latestPatches(await fetchVersions());
  if (list.length === 0) {
    console.error("rmcp-versions: no 3.x version found");
    process.exit(1);
  }
  console.log(JSON.stringify(list));
}

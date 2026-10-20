// Generates harness/contract/tool-contract.json: the tools/list result of the
// TS v1 reference server, served untracked. The Rust reference server loads
// this file, so both servers expose the same customer schemas.
//   node harness/src/dump-contract.mjs           write harness/contract/tool-contract.json
//   node harness/src/dump-contract.mjs --check   regenerate to a temp dir; exit 1 on any byte difference
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";
import { ensureTs } from "oracle/fetch-ts";

const HERE = dirname(fileURLToPath(import.meta.url));
const CONTRACT = join(HERE, "..", "contract", "tool-contract.json");
const GUARD = join(HERE, "egress-guard.mjs");
const TS_V1 = join(HERE, "servers", "ts-v1.mjs");

/** The contract file's text: the server's full tools/list result, every page. */
async function render() {
  await ensureTs();
  const home = mkdtempSync(join(tmpdir(), "harness-contract-home-"));
  const client = new Client({ name: "dump-contract", version: "1.0.0" }, { capabilities: {} });
  try {
    await client.connect(
      new StdioClientTransport({
        command: process.execPath,
        args: ["--import", pathToFileURL(GUARD).href, TS_V1, "stdio"],
        env: {
          ...process.env,
          HOME: home,
          USERPROFILE: home,
          // Untracked: nothing publishes, but the server still requires a local API URL.
          AGENTCAT_API_URL: "http://127.0.0.1:9",
          DISABLE_DIAGNOSTICS: "true",
          HARNESS_NO_TRACK: "1",
          HARNESS_OPTIONS: "{}",
        },
        stderr: "inherit",
      }),
    );
    const tools = [];
    let cursor;
    do {
      const page = await client.listTools(cursor === undefined ? undefined : { cursor });
      tools.push(...page.tools);
      cursor = page.nextCursor;
    } while (cursor !== undefined);
    return `${JSON.stringify(tools, null, 2)}\n`;
  } finally {
    await client.close().catch(() => {});
    rmSync(home, { recursive: true, force: true });
  }
}

async function check() {
  const tmp = mkdtempSync(join(tmpdir(), "harness-contract-"));
  try {
    const fresh = join(tmp, "tool-contract.json");
    writeFileSync(fresh, await render());
    let have = null;
    try {
      have = readFileSync(CONTRACT);
    } catch {
      have = null;
    }
    if (have === null || !readFileSync(fresh).equals(have)) {
      process.stderr.write("harness/contract/tool-contract.json out of date: run node harness/src/dump-contract.mjs\n");
      return 1;
    }
    process.stdout.write("harness/contract/tool-contract.json up to date\n");
    return 0;
  } finally {
    rmSync(tmp, { recursive: true, force: true });
  }
}

const args = process.argv.slice(2);
if (args.length > 1 || (args.length === 1 && args[0] !== "--check")) {
  process.stderr.write("usage: node harness/src/dump-contract.mjs [--check]\n");
  process.exit(2);
}
try {
  if (args[0] === "--check") {
    process.exitCode = await check();
  } else {
    const text = await render();
    mkdirSync(dirname(CONTRACT), { recursive: true });
    writeFileSync(CONTRACT, text);
    process.stdout.write(`${JSON.parse(text).length} tools -> ${CONTRACT}\n`);
  }
} catch (err) {
  process.stderr.write(`${err?.stack ?? err}\n`);
  process.exitCode = 1;
}

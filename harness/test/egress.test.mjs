// The harness never reaches a non-local address: the TS server refuses a
// non-local API URL, and the preloaded guard blocks non-local fetches.
import assert from "node:assert/strict";
import http from "node:http";
import { test } from "node:test";
import { pathToFileURL } from "node:url";

import { GUARD, TS_SERVER, run, tempDir } from "./support.mjs";

function serverEnv(t, apiUrl) {
  return {
    ...process.env,
    AGENTCAT_API_URL: apiUrl,
    DISABLE_DIAGNOSTICS: "true",
    HOME: tempDir(t),
    HARNESS_PROJECT_ID: "proj_HARNESS00000000000000000000",
  };
}

test("(a) ts-v1 stdio refuses a non-local AGENTCAT_API_URL with exit 2", async (t) => {
  const [cmd, ...args] = TS_SERVER;
  const res = await run(cmd, [...args, "stdio"], { env: serverEnv(t, "https://api.agentcat.com") });
  assert.equal(res.code, 2, res.stderr);
  assert.match(res.stderr, /refusing to start/);
  assert.equal(res.stdout, "");
});

test("(a) ts-v1 refuses a host that only starts like a local one", async (t) => {
  const [cmd, ...args] = TS_SERVER;
  for (const url of ["http://localhost.example.com", "http://127.0.0.1.example.com:80", "https://127.0.0.1:1"]) {
    const res = await run(cmd, [...args, "stdio"], { env: serverEnv(t, url) });
    assert.equal(res.code, 2, `${url}: ${res.stderr}`);
    assert.match(res.stderr, /refusing to start/);
  }
});

test("(b) with the guard preloaded, a non-local fetch rejects and a local one works", async (t) => {
  const local = http.createServer((req, res) => res.end("ok"));
  await new Promise((resolve) => local.listen(0, "127.0.0.1", resolve));
  t.after(() => local.close());
  const localUrl = `http://127.0.0.1:${local.address().port}/`;
  const script = `
    const out = {};
    try { await fetch("https://api.agentcat.com/"); out.remote = "resolved"; }
    catch (e) { out.remote = String(e.message); }
    out.local = await (await fetch(${JSON.stringify(localUrl)})).text();
    process.stdout.write(JSON.stringify(out));
  `;
  // A tripwire loaded before the guard stands in for the real fetch on
  // non-local URLs, so a broken guard fails this test without network access.
  const tripwire = `
    const realFetch = globalThis.fetch;
    globalThis.fetch = (input, init) => {
      const url = new URL(input instanceof Request ? input.url : String(input));
      if (url.hostname === "127.0.0.1") return realFetch(input, init);
      return Promise.reject(new Error("tripwire: the guard let " + url.origin + " through"));
    };
  `;
  const res = await run(process.execPath, [
    "--import",
    `data:text/javascript,${encodeURIComponent(tripwire)}`,
    "--import",
    pathToFileURL(GUARD).href,
    "--input-type=module",
    "-e",
    script,
  ]);
  assert.equal(res.code, 0, res.stderr);
  const out = JSON.parse(res.stdout);
  assert.match(out.remote, /egress-guard: blocked/);
  assert.equal(out.local, "ok");
  assert.match(res.stderr, /\[egress-guard\] BLOCKED https:\/\/api\.agentcat\.com/);
});

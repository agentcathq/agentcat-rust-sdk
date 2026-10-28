// The http-sse transport (C59): stateful streamable HTTP whose POST answers are SSE streams
// (TS enableJsonResponse false, rmcp with_json_response(false)), and the driver and matrix lanes on it.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { before, test } from "node:test";

import { ensureTs } from "oracle/fetch-ts";

import { BASIC, RUST_SERVER, TS_SERVER, TS_V2_SERVER, driver, expectedFor, outFile, readJson, run, tempDir } from "./support.mjs";

before(async () => {
  await ensureTs();
});

const basic = readJson(BASIC);
const SERVERS = [
  ["ts-v1", TS_SERVER],
  ["ts-v2", TS_V2_SERVER],
  ["rust", RUST_SERVER],
];

/** Starts `server <mode> 0` untracked; resolves {url, stop()}. */
async function serve(t, server, mode = "http-sse") {
  const [cmd, ...args] = server;
  const env = {
    ...process.env,
    AGENTCAT_API_URL: "http://127.0.0.1:9",
    DISABLE_DIAGNOSTICS: "true",
    HOME: tempDir(t),
    HARNESS_PROJECT_ID: basic.projectId,
    HARNESS_NO_TRACK: "1",
  };
  const child = spawn(cmd, [...args, mode, "0"], { env, stdio: ["ignore", "pipe", "pipe"] });
  t.after(() => child.kill("SIGKILL"));
  const line = await new Promise((resolve, reject) => {
    child.once("exit", (code) => reject(new Error(`server exited ${code} before its ready line`)));
    createInterface({ input: child.stdout }).once("line", resolve);
  });
  return JSON.parse(line).url;
}

const HEADERS = { "content-type": "application/json", accept: "application/json, text/event-stream" };
const post = (url, body, extra = {}) => fetch(url, { method: "POST", headers: { ...HEADERS, ...extra }, body: JSON.stringify(body) });

for (const [name, server] of SERVERS) {
  test(`${name} answers a tools/call over http-sse with text/event-stream`, { timeout: 120_000 }, async (t) => {
    const url = await serve(t, server);
    const init = await post(url, {
      jsonrpc: "2.0",
      id: 1,
      method: "initialize",
      params: { protocolVersion: "2025-11-25", capabilities: {}, clientInfo: { name: "sse-test", version: "1.0.0" } },
    });
    const sid = init.headers.get("mcp-session-id");
    assert.ok(sid, "a session ID");
    await init.text();
    const session = { "mcp-session-id": sid, "mcp-protocol-version": "2025-11-25" };
    const ok = await post(url, { jsonrpc: "2.0", method: "notifications/initialized" }, session);
    await ok.text();
    const res = await post(url, { jsonrpc: "2.0", id: 2, method: "tools/call", params: { name: "echo", arguments: { msg: "hi" } } }, session);
    assert.equal(res.status, 200);
    assert.match(res.headers.get("content-type") ?? "", /^text\/event-stream/);
    assert.match(await res.text(), /^data: .*"id":2/m);
  });
}

// The 2026-07-28 envelope: no initialize, no session; the request carries its protocol version, client
// info and capabilities in params._meta and the method and tool name in headers.
async function call2026(url) {
  const res = await post(
    url,
    {
      jsonrpc: "2.0",
      id: 1,
      method: "tools/call",
      params: {
        name: "echo",
        arguments: { msg: "hi" },
        _meta: {
          "io.modelcontextprotocol/protocolVersion": "2026-07-28",
          "io.modelcontextprotocol/clientInfo": { name: "sse-test", version: "1.0.0" },
          "io.modelcontextprotocol/clientCapabilities": {},
        },
      },
    },
    { "mcp-protocol-version": "2026-07-28", "mcp-method": "tools/call", "mcp-name": "echo" },
  );
  return { status: res.status, type: res.headers.get("content-type") ?? "", text: await res.text() };
}

// The 2026-07-28 lane: http answers JSON, http-sse answers SSE, on both servers (README, HTTP responses).
for (const [name, server] of SERVERS.filter(([n]) => n !== "ts-v1")) {
  for (const [mode, type] of [["http-sse", /^text\/event-stream/], ["http", /^application\/json/]]) {
    test(`${name} answers a 2026-07-28 tools/call over ${mode} as ${type.source.slice(1)}`, { timeout: 120_000 }, async (t) => {
      const res = await call2026(await serve(t, server, mode));
      assert.equal(res.status, 200, res.text);
      assert.match(res.type, type, res.text);
      assert.match(res.text, /"id":1/);
      assert.match(res.text, /hi/);
    });
  }
}

test("a bad transport name is still refused by the usage line of each server", { timeout: 60_000 }, async (t) => {
  for (const [name, server] of SERVERS) {
    const [cmd, ...args] = server;
    const res = await run(cmd, [...args, "http-sse-x", "0"], {
      env: { ...process.env, AGENTCAT_API_URL: "http://127.0.0.1:9", HOME: tempDir(t), HARNESS_PROJECT_ID: basic.projectId },
    });
    assert.equal(res.code, 2, name);
    assert.match(res.stderr, /http-sse <port>/, name);
  }
});

test("the driver runs basic.json over http-sse on a Rust and a TS v2 tracked server, on both lanes", { timeout: 300_000 }, async () => {
  for (const [label, server] of [["ssets2", TS_V2_SERVER], ["ssers", RUST_SERVER]]) {
    for (const lane of ["2025-11-25", "2026-07-28"]) {
      const res = await driver(BASIC, `${label}-${lane}`, "http-sse", ["--lane", lane], { server });
      assert.equal(res.code, 0, `${label} ${lane}: ${res.stderr}`);
      const report = readJson(outFile("basic", `${label}-${lane}`, "http-sse", "report"));
      assert.equal(report.publishes, expectedFor(basic, "http-sse", lane));
    }
  }
});

test("matrix.mjs --scenarios basic --transports http-sse runs both lanes and exits 0", { timeout: 600_000 }, async () => {
  const res = await run(process.execPath, [new URL("../src/matrix.mjs", import.meta.url).pathname, "--scenarios", "basic", "--transports", "http-sse"]);
  assert.equal(res.code, 0, res.stdout + res.stderr);
  assert.match(res.stdout, /^ok basic http-sse 2025-11-25:/m);
  assert.match(res.stdout, /^ok basic http-sse 2026-07-28:/m);
});

// The harness never reaches a non-local address: the TS server refuses a missing or non-local API
// URL and hands it to the TS SDK as `apiBaseUrl`, and the preloaded guard blocks non-local
// fetches, TCP and TLS connections, DNS lookups and UDP sends.
//
// Every guard test loads a tripwire before the guard. The tripwire stands in for the real fetch,
// socket connect, TLS connect, DNS lookup and UDP send on non-local targets, so a broken guard
// fails the test with a "tripwire:" message and never reaches the network.
import assert from "node:assert/strict";
import dgram from "node:dgram";
import http from "node:http";
import net from "node:net";
import { join } from "node:path";
import { test } from "node:test";
import { pathToFileURL } from "node:url";

import { ensureTs } from "oracle/fetch-ts";

import { BASIC, GUARD, HARNESS, TS_SERVER, driver, outFile, readJson, run, tempDir, writeJson } from "./support.mjs";

function serverEnv(t, apiUrl) {
  const env = {
    ...process.env,
    AGENTCAT_API_URL: apiUrl,
    DISABLE_DIAGNOSTICS: "true",
    HOME: tempDir(t),
    HARNESS_PROJECT_ID: "proj_HARNESS00000000000000000000",
  };
  if (apiUrl === undefined) delete env.AGENTCAT_API_URL;
  return env;
}

const TRIPWIRE = `
  import dgram from "node:dgram";
  import dns from "node:dns";
  import { syncBuiltinESMExports } from "node:module";
  import net from "node:net";
  import tls from "node:tls";
  const LOCAL = ["127.0.0.1", "localhost", "::1", "[::1]"];
  const trip = (what) => new Error("tripwire: the guard let " + what + " through");
  const realFetch = globalThis.fetch;
  globalThis.fetch = (input, init) => {
    const url = new URL(input instanceof Request ? input.url : String(input));
    if (url.hostname === "127.0.0.1") return realFetch(input, init);
    return Promise.reject(trip(url.origin));
  };
  const realConnect = net.Socket.prototype.connect;
  net.Socket.prototype.connect = function (...args) {
    const first = Array.isArray(args[0]) ? args[0][0] : args[0];
    const options = first !== null && typeof first === "object" ? first : {};
    const host = options.host ?? args[1];
    if (!options.path && !LOCAL.includes(host)) throw trip("a connection to " + host);
    return realConnect.apply(this, args);
  };
  const realTlsConnect = tls.connect;
  tls.connect = function (...args) {
    const options = Object.assign({}, ...args.filter((a) => a !== null && typeof a === "object"));
    const host = options.host ?? (typeof args[1] === "string" ? args[1] : undefined);
    if (!options.socket && !LOCAL.includes(host)) throw trip("a TLS connection to " + host);
    return realTlsConnect.apply(this, args);
  };
  // A lookup of an address literal sends nothing (dgram looks up 0.0.0.0 to bind).
  const lookupSends = (host) => !LOCAL.includes(host) && net.isIP(String(host)) === 0;
  const realLookup = dns.lookup;
  dns.lookup = function (host, ...rest) {
    if (lookupSends(host)) throw trip("a lookup of " + host);
    return realLookup.call(this, host, ...rest);
  };
  const realPromisesLookup = dns.promises.lookup;
  dns.promises.lookup = function (host, ...rest) {
    if (lookupSends(host)) return Promise.reject(trip("a lookup of " + host));
    return realPromisesLookup.call(this, host, ...rest);
  };
  const realSend = dgram.Socket.prototype.send;
  dgram.Socket.prototype.send = function (...args) {
    const address = args.slice(1).find((a) => typeof a === "string");
    if (!LOCAL.includes(address)) throw trip("a UDP send to " + address);
    return realSend.apply(this, args);
  };
  syncBuiltinESMExports();
`;

// Settles with what the callback passes to ok, or with the error message (a synchronous throw too).
const ATTEMPT = `
  const attempt = (start) => new Promise((resolve) => {
    const fail = (err) => resolve(String(err?.message ?? err));
    try { start(resolve, fail); } catch (err) { fail(err); }
  });
`;

/** Runs an ES module script in a Node process with the tripwire, then the guard, preloaded. */
function guarded(script, env = {}) {
  return run(
    process.execPath,
    [
      "--import",
      `data:text/javascript,${encodeURIComponent(TRIPWIRE)}`,
      "--import",
      pathToFileURL(GUARD).href,
      "--input-type=module",
      "-e",
      script,
    ],
    { env: { ...process.env, ...env } },
  );
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

test("(a) ts-v1 refuses to start without AGENTCAT_API_URL", async (t) => {
  const [cmd, ...args] = TS_SERVER;
  for (const url of [undefined, ""]) {
    const res = await run(cmd, [...args, "stdio"], { env: serverEnv(t, url) });
    assert.equal(res.code, 2, `${url}: ${res.stderr}`);
    assert.match(res.stderr, /refusing to start/);
    assert.equal(res.stdout, "");
  }
});

test("(a) the TS SDK options carry AGENTCAT_API_URL as apiBaseUrl and disable diagnostics", async (t) => {
  const common = pathToFileURL(join(HARNESS, "src", "servers", "common.mjs")).href;
  const script = `
    const { startup } = await import(${JSON.stringify(common)});
    process.stdout.write(JSON.stringify(startup("probe").options));
  `;
  const env = { ...serverEnv(t, "http://127.0.0.1:9/"), HARNESS_OPTIONS: JSON.stringify({ enableTracing: true }) };
  const res = await run(process.execPath, ["--input-type=module", "-e", script], { env });
  assert.equal(res.code, 0, res.stderr);
  assert.deepEqual(JSON.parse(res.stdout), {
    apiBaseUrl: "http://127.0.0.1:9/",
    disableDiagnostics: true,
    enableTracing: true,
  });
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
  const res = await guarded(script);
  assert.equal(res.code, 0, res.stderr);
  const out = JSON.parse(res.stdout);
  assert.match(out.remote, /egress-guard: blocked/);
  assert.equal(out.local, "ok");
  assert.match(res.stderr, /\[egress-guard\] BLOCKED https:\/\/api\.agentcat\.com/);
});

test("(c) the guard refuses https, http, TCP, TLS, DNS and UDP to a non-local host before any packet", async () => {
  const script = `
    import dgram from "node:dgram";
    import dns from "node:dns";
    import { lookup } from "node:dns/promises";
    import http from "node:http";
    import https from "node:https";
    import net from "node:net";
    import { connect as tlsConnect } from "node:tls";
    ${ATTEMPT}
    const host = "api.agentcat.com";
    const udp = dgram.createSocket("udp4");
    const out = {
      httpsGet: await attempt((ok, fail) => https.get("https://" + host + "/", () => ok("connected")).on("error", fail)),
      httpGet: await attempt((ok, fail) => http.get("http://" + host + "/", () => ok("connected")).on("error", fail)),
      netConnect: await attempt((ok, fail) => net.connect(443, host, () => ok("connected")).on("error", fail)),
      netConnectOptions: await attempt((ok, fail) => net.connect({ host, port: 443 }, () => ok("connected")).on("error", fail)),
      tlsConnect: await attempt((ok, fail) => tlsConnect({ host, port: 443, servername: host }, () => ok("connected")).on("error", fail)),
      dnsLookup: await attempt((ok, fail) => dns.lookup(host, (err, address) => (err ? fail(err) : ok(address)))),
      dnsPromisesLookup: await attempt((ok, fail) => lookup(host).then((r) => ok(r.address), fail)),
      udpSend: await attempt((ok, fail) => udp.send("x", 53, host, (err) => (err ? fail(err) : ok("sent")))),
      // An address literal (TEST-NET-3, never routed) is refused like a name.
      netConnectAddress: await attempt((ok, fail) => net.connect(443, "203.0.113.1", () => ok("connected")).on("error", fail)),
    };
    udp.close();
    process.stdout.write(JSON.stringify(out), () => process.exit(0));
  `;
  const res = await guarded(script);
  assert.equal(res.code, 0, res.stderr);
  const out = JSON.parse(res.stdout);
  assert.equal(Object.keys(out).length, 9);
  const notBlocked = Object.entries(out).filter(([check, result]) => {
    const target = check === "netConnectAddress" ? "203.0.113.1" : "api.agentcat.com";
    return !result.startsWith(`egress-guard: blocked ${target}`);
  });
  assert.deepEqual(notBlocked, []);
  assert.match(res.stderr, /\[egress-guard\] BLOCKED api\.agentcat\.com:443/);
  assert.doesNotMatch(res.stderr, /tripwire/);
});

test("(c) with the guard preloaded, loopback, localhost lookups, local sockets and UDP to loopback still work", async (t) => {
  const web = http.createServer((req, res) => res.end("ok"));
  await new Promise((resolve) => web.listen(0, "127.0.0.1", resolve));
  t.after(() => web.close());
  let tcpConnections = 0;
  const tcp = net.createServer((socket) => {
    tcpConnections += 1;
    socket.destroy();
  });
  await new Promise((resolve) => tcp.listen(0, "127.0.0.1", resolve));
  t.after(() => tcp.close());
  const pipePath =
    process.platform === "win32" ? `\\\\.\\pipe\\agentcat-guard-${process.pid}` : join(tempDir(t), "guard.sock");
  const pipe = net.createServer((socket) => socket.end());
  await new Promise((resolve) => pipe.listen(pipePath, resolve));
  t.after(() => pipe.close());
  const udp = dgram.createSocket("udp4");
  const received = new Promise((resolve) => udp.once("message", (msg) => resolve(msg.toString())));
  await new Promise((resolve) => udp.bind(0, "127.0.0.1", resolve));
  t.after(() => udp.close());

  const script = `
    import dgram from "node:dgram";
    import dns from "node:dns";
    import { lookup } from "node:dns/promises";
    import http from "node:http";
    import net from "node:net";
    import tls from "node:tls";
    ${ATTEMPT}
    const env = process.env;
    const udp = dgram.createSocket("udp4");
    const out = {
      httpGet: await attempt((ok, fail) =>
        http.get(env.WEB_URL, (res) => {
          let body = "";
          res.setEncoding("utf8").on("data", (d) => (body += d)).on("end", () => ok(body));
        }).on("error", fail)),
      netConnect: await attempt((ok, fail) => {
        const s = net.connect(Number(env.TCP_PORT), "127.0.0.1", () => { s.destroy(); ok("connected"); });
        s.on("error", fail);
      }),
      // The TCP server closes at once, so the TLS handshake fails, but only after the connection.
      tlsConnect: await attempt((ok, fail) => {
        const s = tls.connect({ host: "127.0.0.1", port: Number(env.TCP_PORT), rejectUnauthorized: false }, () => ok("handshake"));
        s.on("error", fail);
      }),
      pipe: await attempt((ok, fail) => {
        const s = net.connect(env.PIPE_PATH, () => { s.destroy(); ok("connected"); });
        s.on("error", fail);
      }),
      lookupLocalhost: await attempt((ok, fail) => dns.lookup("localhost", (err) => (err ? fail(err) : ok("resolved")))),
      promisesLookupLocalhost: await attempt((ok, fail) => lookup("localhost").then(() => ok("resolved"), fail)),
      udpSend: await attempt((ok, fail) => udp.send("ping", Number(env.UDP_PORT), "127.0.0.1", (err) => (err ? fail(err) : ok("sent")))),
      fetch: await attempt((ok, fail) => fetch(env.WEB_URL).then((r) => r.text()).then(ok, fail)),
    };
    udp.close();
    process.stdout.write(JSON.stringify(out), () => process.exit(0));
  `;
  const res = await guarded(script, {
    WEB_URL: `http://127.0.0.1:${web.address().port}/`,
    TCP_PORT: String(tcp.address().port),
    PIPE_PATH: pipePath,
    UDP_PORT: String(udp.address().port),
  });
  assert.equal(res.code, 0, res.stderr);
  const out = JSON.parse(res.stdout);
  assert.equal(out.httpGet, "ok");
  assert.equal(out.netConnect, "connected");
  assert.doesNotMatch(out.tlsConnect, /egress-guard|tripwire/);
  assert.equal(out.pipe, "connected");
  assert.equal(out.lookupLocalhost, "resolved");
  assert.equal(out.promisesLookupLocalhost, "resolved");
  assert.equal(out.udpSend, "sent");
  assert.equal(out.fetch, "ok");
  assert.equal(await received, "ping");
  assert.equal(tcpConnections, 2);
  assert.doesNotMatch(res.stderr, /egress-guard|tripwire/);
});

test("an untracked run of basic.json makes no publish attempt and does not trip the guard", { timeout: 120_000 }, async (t) => {
  await ensureTs();
  const scenario = writeJson(join(tempDir(t), "basic.json"), { ...readJson(BASIC), expectPublishes: 0 });
  const res = await driver(scenario, "untracked", "stdio", [], { env: { HARNESS_NO_TRACK: "1" } });
  assert.equal(res.code, 0, res.stderr);
  assert.doesNotMatch(res.stderr, /egress-guard/);
  assert.deepEqual(readJson(outFile("basic", "untracked", "stdio", "raw")), []);
  const transcript = readJson(outFile("basic", "untracked", "stdio", "transcript"));
  const emit = transcript.find((e) => e.request?.name === "emit_custom");
  assert.deepEqual(emit.result.content, [{ type: "text", text: "published" }]);
});

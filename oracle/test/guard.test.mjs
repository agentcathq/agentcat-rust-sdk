// list-tests.mjs runs `vitest list` (which runs the top level of every TS test file) under the
// egress guard. The guard's own tests are in harness/test/egress.test.mjs.
//
// The vitest test loads a tripwire before the guard: it stands in for the real socket connect and
// DNS lookup on non-local targets, so a missing guard fails the test with a "tripwire:" message
// and never reaches the network.
import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { pathToFileURL } from "node:url";

import { ensureTs, guardedNodeOptions, run, vitestCommand } from "../src/fetch-ts.mjs";
import { tsDir } from "../src/paths.mjs";

const GUARD_IMPORT = `--import=${pathToFileURL(join(import.meta.dirname, "..", "src", "egress-guard.mjs")).href}`;

const TRIPWIRE = `
  import dns from "node:dns";
  import { syncBuiltinESMExports } from "node:module";
  import net from "node:net";
  const LOCAL = ["127.0.0.1", "localhost", "::1", "[::1]"];
  const trip = (what) => new Error("tripwire: the guard let " + what + " through");
  const realConnect = net.Socket.prototype.connect;
  net.Socket.prototype.connect = function (...args) {
    const first = Array.isArray(args[0]) ? args[0][0] : args[0];
    const options = first !== null && typeof first === "object" ? first : {};
    const host = options.host ?? args[1];
    if (!options.path && !LOCAL.includes(host)) throw trip("a connection to " + host);
    return realConnect.apply(this, args);
  };
  const realLookup = dns.lookup;
  dns.lookup = function (host, ...rest) {
    if (!LOCAL.includes(host) && net.isIP(String(host)) === 0) throw trip("a lookup of " + host);
    return realLookup.call(this, host, ...rest);
  };
  syncBuiltinESMExports();
`;

// A test file whose top level tries to reach the network, as unreviewed TS code could.
const PROBE = `
  import dns from "node:dns";
  import net from "node:net";
  import { writeFileSync } from "node:fs";
  const attempt = (f) => { try { f(); return "not blocked"; } catch (err) { return err.message; } };
  writeFileSync(process.env.PROBE_OUT, JSON.stringify({
    guard: globalThis[Symbol.for("agentcat.egressGuard")] === true,
    netConnect: attempt(() => net.connect(443, "api.agentcat.com").on("error", () => {})),
    dnsLookup: attempt(() => dns.lookup("api.agentcat.com", () => {})),
  }));
`;

test("guardedNodeOptions adds the guard after the given NODE_OPTIONS", () => {
  assert.equal(guardedNodeOptions(""), GUARD_IMPORT);
  assert.equal(guardedNodeOptions("  "), GUARD_IMPORT);
  assert.equal(guardedNodeOptions("--max-old-space-size=512"), `--max-old-space-size=512 ${GUARD_IMPORT}`);
});

test("the vitest that list-tests runs loads the guard into the processes that collect test files", { timeout: 300_000 }, async (t) => {
  await ensureTs();
  const project = mkdtempSync(join(tmpdir(), "oracle-guard-"));
  t.after(() => rmSync(project, { recursive: true, force: true }));
  writeFileSync(join(project, "probe.test.mjs"), PROBE);
  const out = join(project, "probe.json");
  const tripwire = `--import=data:text/javascript,${encodeURIComponent(TRIPWIRE)}`;
  const vitest = vitestCommand(tsDir(), ["list", "--root", project], tripwire);
  assert.equal(vitest.cmd, process.execPath);
  const stdout = await run(vitest.cmd, vitest.args, project, { ...vitest.env, PROBE_OUT: out });
  const probe = JSON.parse(readFileSync(out, "utf8"));
  assert.deepEqual(
    probe,
    {
      guard: true,
      netConnect: "egress-guard: blocked api.agentcat.com:443",
      dnsLookup: "egress-guard: blocked api.agentcat.com",
    },
    stdout,
  );
});

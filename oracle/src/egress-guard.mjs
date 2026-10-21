// The egress guard. Preloaded with `node --import <this file>` into every TS reference server
// (harness/src/egress-guard.mjs re-exports it) and, through NODE_OPTIONS, into the `vitest list`
// run of oracle/src/parity/list-tests.mjs and every Node process it starts. It refuses outbound
// connections to anything but loopback, so these runs cannot reach AgentCat production (events,
// OTLP diagnostics, exporters) even when the TS code ignores the local API URL.
//
// It patches, in the process that loads it:
// - globalThis.fetch: a URL whose host is not 127.0.0.1, localhost or [::1] rejects.
// - net.Socket.prototype.connect: every TCP connection (net.connect, http, https, tls, undici and so
//   fetch too) to a host other than localhost, 127.0.0.0/8 or ::1 throws. A Unix socket or a named
//   pipe (a `path`) passes.
// - tls.connect: the same host check, before the socket exists.
// - dns.lookup and dns.promises.lookup: a host name other than localhost throws (rejects). An
//   address literal passes: its lookup sends nothing (dgram looks up 0.0.0.0 to bind), and the
//   connect and send checks above see the address itself.
// - dgram.Socket send and connect: an address other than those throws.
// A refusal writes `[egress-guard] BLOCKED <target>` to stderr and throws (or rejects with)
// `egress-guard: blocked <target>` before any packet leaves.
//
// Not covered: worker threads (Node runs no preload in them), child processes started without
// the guard (git, corepack and pnpm, which need the network), other runtimes (workerd), native
// addons, dns.resolve* and dns.Resolver. Rust servers get no guard: they only ever receive a local
// base URL.
import dgram from "node:dgram";
import dns from "node:dns";
import { syncBuiltinESMExports } from "node:module";
import net from "node:net";
import tls from "node:tls";

const FETCH_HOSTS = new Set(["127.0.0.1", "localhost", "[::1]"]);
const INSTALLED = Symbol.for("agentcat.egressGuard");

/** localhost, an IPv4 address in 127.0.0.0/8 or the IPv6 address ::1 (brackets allowed). */
function isLoopback(host) {
  if (typeof host !== "string") return false;
  let name = host.toLowerCase();
  if (name.startsWith("[") && name.endsWith("]")) name = name.slice(1, -1);
  if (name === "localhost") return true;
  if (net.isIPv4(name)) return name.startsWith("127.");
  if (net.isIPv6(name)) {
    try {
      return new URL(`http://[${name}]/`).hostname === "[::1]";
    } catch {
      return false;
    }
  }
  return false;
}

function blocked(target) {
  process.stderr.write(`[egress-guard] BLOCKED ${target}\n`);
  return new Error(`egress-guard: blocked ${target}`);
}

/** A pipe name as net reads it: a string that is not a non-negative number. */
function isPipeName(value) {
  return typeof value === "string" && !(Number(value) >= 0);
}

/**
 * The target of `net.Socket.prototype.connect(...args)`, or null for a Unix socket or a named
 * pipe. Mirrors net's argument rules: `(options[, cb])`, `(path[, cb])`, `(port[, host][, cb])`;
 * `net.connect` passes its normalized arguments as one array. The default host is localhost.
 */
function socketTarget(args) {
  const first = Array.isArray(args[0]) ? args[0][0] : args[0];
  if (first !== null && typeof first === "object") {
    return first.path ? null : { host: first.host || "localhost", port: first.port };
  }
  if (isPipeName(first)) return null;
  return { host: typeof args[1] === "string" && args[1] !== "" ? args[1] : "localhost", port: first };
}

/**
 * The target of `tls.connect(...args)`, or null when it wraps an existing socket or uses a pipe.
 * As tls reads them: net's rules, then an options object in the second or third place wins.
 */
function tlsTarget(args) {
  const options = {};
  const [first, second, third] = args;
  if (first !== null && typeof first === "object") Object.assign(options, first);
  else if (isPipeName(first)) options.path = first;
  else {
    options.port = first;
    if (typeof second === "string") options.host = second;
  }
  if (second !== null && typeof second === "object") Object.assign(options, second);
  else if (third !== null && typeof third === "object") Object.assign(options, third);
  if (options.socket || options.path) return null;
  return { host: options.host || "localhost", port: options.port };
}

/**
 * The address of `dgram.Socket.prototype.send(msg, offset, length, port, address, cb)`, or null
 * when none is given (dgram then uses loopback, or the address the socket is connected to).
 */
function dgramSendAddress(args) {
  const [, , length, port, address] = args;
  const withOffset = Boolean(address) || (Boolean(port) && typeof port !== "function");
  const given = withOffset ? address : length;
  return typeof given === "string" && given !== "" ? given : null;
}

/** Whether `dns.lookup(hostname)` would query DNS for a name other than localhost. */
function lookupLeaves(hostname) {
  return Boolean(hostname) && net.isIP(String(hostname)) === 0 && !isLoopback(String(hostname));
}

const describe = ({ host, port }) => (port === undefined || port === null ? String(host) : `${host}:${port}`);

/** Copies the symbol-keyed properties (util.promisify's argument names on dns.lookup). */
function keepSymbols(from, to) {
  for (const key of Object.getOwnPropertySymbols(from)) {
    Object.defineProperty(to, key, Object.getOwnPropertyDescriptor(from, key));
  }
  return to;
}

if (!globalThis[INSTALLED]) {
  globalThis[INSTALLED] = true;

  const realFetch = globalThis.fetch;
  globalThis.fetch = async function guardedFetch(input, init) {
    const url = new URL(input instanceof Request ? input.url : String(input));
    if (!FETCH_HOSTS.has(url.hostname)) {
      process.stderr.write(`[egress-guard] BLOCKED ${url.origin}${url.pathname}\n`);
      throw new Error(`egress-guard: blocked ${url.origin}`);
    }
    return realFetch(input, init);
  };

  const realConnect = net.Socket.prototype.connect;
  net.Socket.prototype.connect = function guardedConnect(...args) {
    const target = socketTarget(args);
    if (target !== null && !isLoopback(target.host)) throw blocked(describe(target));
    return realConnect.apply(this, args);
  };

  const realTlsConnect = tls.connect;
  tls.connect = function guardedTlsConnect(...args) {
    const target = tlsTarget(args);
    if (target !== null && !isLoopback(target.host)) throw blocked(describe(target));
    return realTlsConnect.apply(this, args);
  };

  const realLookup = dns.lookup;
  dns.lookup = keepSymbols(realLookup, function guardedLookup(hostname, ...rest) {
    if (lookupLeaves(hostname)) throw blocked(String(hostname));
    return realLookup.call(this, hostname, ...rest);
  });

  const realPromisesLookup = dns.promises.lookup;
  dns.promises.lookup = function guardedPromisesLookup(hostname, ...rest) {
    if (lookupLeaves(hostname)) return Promise.reject(blocked(String(hostname)));
    return realPromisesLookup.call(this, hostname, ...rest);
  };

  const realSend = dgram.Socket.prototype.send;
  dgram.Socket.prototype.send = function guardedSend(...args) {
    const address = dgramSendAddress(args);
    if (address !== null && !isLoopback(address)) throw blocked(address);
    return realSend.apply(this, args);
  };

  const realDgramConnect = dgram.Socket.prototype.connect;
  dgram.Socket.prototype.connect = function guardedDgramConnect(port, address, ...rest) {
    if (typeof address === "string" && address !== "" && !isLoopback(address)) {
      throw blocked(describe({ host: address, port }));
    }
    return realDgramConnect.call(this, port, address, ...rest);
  };

  // Named imports of builtins (`import { connect } from "node:tls"`) see the patched functions.
  syncBuiltinESMExports();
}

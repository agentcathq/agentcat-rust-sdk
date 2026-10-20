// Preloaded into every TS reference server: `node --import <this file> ...`.
// Any fetch to a host other than 127.0.0.1, localhost or [::1] is refused and
// logged to stderr, so the harness can never reach production AgentCat
// (events, OTLP diagnostics, exporters). The TS SDK and its generated API
// client send every request through the global fetch. Rust servers get the
// same guarantee by only ever receiving a local base URL.
const LOCAL_HOSTS = new Set(["127.0.0.1", "localhost", "[::1]"]);
const INSTALLED = Symbol.for("agentcat.harness.egressGuard");

if (!globalThis[INSTALLED]) {
  globalThis[INSTALLED] = true;
  const realFetch = globalThis.fetch;
  globalThis.fetch = async function guardedFetch(input, init) {
    const url = new URL(input instanceof Request ? input.url : String(input));
    if (!LOCAL_HOSTS.has(url.hostname)) {
      process.stderr.write(`[egress-guard] BLOCKED ${url.origin}${url.pathname}\n`);
      throw new Error(`egress-guard: blocked ${url.origin}`);
    }
    return realFetch(input, init);
  };
}

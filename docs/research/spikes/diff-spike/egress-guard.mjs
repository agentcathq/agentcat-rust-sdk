// Preloaded with `node --import ./egress-guard.mjs` into the TS reference server.
// Any fetch to a host other than 127.0.0.1/localhost is refused and logged to
// stderr, so the harness can never reach production AgentCat (events, OTLP
// diagnostics, exporters). The Rust side gets the same guarantee by only ever
// receiving a 127.0.0.1 base URL and running with diagnostics disabled.
const realFetch = globalThis.fetch;
globalThis.fetch = async (input, init) => {
  const url = new URL(input instanceof Request ? input.url : String(input));
  if (url.hostname !== "127.0.0.1" && url.hostname !== "localhost") {
    process.stderr.write(`[egress-guard] BLOCKED ${url.origin}${url.pathname}\n`);
    throw new Error(`egress-guard: blocked ${url.origin}`);
  }
  return realFetch(input, init);
};

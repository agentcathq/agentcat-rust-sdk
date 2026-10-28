// Normalizer: turns one run (raw mock-API requests + MCP transcript) into a
// language-neutral document that a TS run and a Rust run must match exactly.
// Rules are explicit and each one is reported, so a rule can never silently
// hide a real difference.

const ID_RE = /\b(evt|ses|agt)_[0-9A-Za-z]{27}\b/g;
const ISO_MS_Z = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/;
const snake = (k) => k.replace(/[A-Z]/g, (c) => "_" + c.toLowerCase());

// Fields that legitimately differ per language; replaced by a class token.
const LANGUAGE_FIELDS = {
  sdk_language: (v) => `<sdk_language:${typeof v}>`,
  agentcat_version: (v) => (typeof v === "string" && /^\d+\.\d+\.\d+/.test(v) ? "<semver>" : `<bad-version:${v}>`),
};

// Header names worth comparing (backend reads user-agent and allowlisted ids
// out of parameters by nested search, so only name/value matter, not path).
const HEADER_KEEP = new Set(["user-agent", "x-anthropic-client", "x-test-header", "mcp-protocol-version"]);

function collectHeaders(node, out) {
  if (!node || typeof node !== "object") return;
  if (Array.isArray(node)) return node.forEach((n) => collectHeaders(n, out));
  for (const [k, v] of Object.entries(node)) {
    if (k === "headers" && v && typeof v === "object" && !Array.isArray(v)) {
      for (const [hk, hv] of Object.entries(v)) {
        if (HEADER_KEEP.has(hk.toLowerCase())) out[hk.toLowerCase()] = hv;
      }
    } else collectHeaders(v, out);
  }
}

export function normalizeRun({ requests, transcript }) {
  const report = { camelCaseAliasKeys: new Set(), nonPublishRequests: [], rules: [] };
  const publishes = [];
  for (const r of requests) {
    if (r.path === "/v1/public/events/" && r.method === "POST") publishes.push(r);
    else report.nonPublishRequests.push({ method: r.method, path: r.path });
  }

  // Order: by original event timestamp, then resource name, then canonical JSON
  // (the queue publishes up to 5 events concurrently, so arrival order is noise).
  publishes.sort((a, b) =>
    String(a.body?.timestamp).localeCompare(String(b.body?.timestamp)) ||
    String(a.body?.resource_name).localeCompare(String(b.body?.resource_name)) ||
    JSON.stringify(a.body).localeCompare(JSON.stringify(b.body)),
  );

  const events = publishes.map((r) => {
    const body = structuredClone(r.body);
    // Rule 1: camelCase duplicates emitted by the TS generated client
    // (PublishEventRequestToJSON spreads the camelCase model first).
    for (const k of Object.keys(body)) {
      if (k !== snake(k) && snake(k) in body) {
        report.camelCaseAliasKeys.add(k);
        delete body[k];
      }
    }
    // Rule 2: timestamp -> format class.
    if ("timestamp" in body) body.timestamp = ISO_MS_Z.test(body.timestamp) ? "<iso8601-ms-Z>" : `<bad-timestamp:${body.timestamp}>`;
    // Rule 3: duration -> type class (non-negative integer ms).
    if ("duration" in body) body.duration = Number.isInteger(body.duration) && body.duration >= 0 ? "<duration-ms>" : `<bad-duration:${body.duration}>`;
    // Rule 4: language-specific fields -> class tokens.
    for (const [k, f] of Object.entries(LANGUAGE_FIELDS)) if (k in body) body[k] = f(body[k]);
    // Rule 5: parameters.extra is the host MCP SDK's request context; keep only
    // backend-relevant headers by name.
    if (body.parameters && typeof body.parameters === "object" && "extra" in body.parameters) {
      const headers = {};
      collectHeaders(body.parameters.extra, headers);
      body.parameters.extra = { "<headers>": headers };
    }
    // Rule 6: error stack/frames are language-specific; keep message/type presence.
    if (body.error && typeof body.error === "object") {
      for (const k of ["stack", "frames", "platform"]) if (k in body.error) body.error[k] = `<${k}>`;
    }
    return { mock_status: r.status, body };
  });

  // Rule 7: random IDs -> first-appearance placeholders, consistent across
  // transcript and events (so "the ses_ the client was issued is the ses_ on
  // the event" is still checked).
  const doc = { transcript, events };
  const ids = new Map();
  const counters = {};
  const json = JSON.stringify(doc).replace(ID_RE, (m, prefix) => {
    if (!ids.has(m)) {
      counters[prefix] = (counters[prefix] ?? 0) + 1;
      ids.set(m, `<${prefix}#${counters[prefix]}>`);
    }
    return ids.get(m);
  });
  report.camelCaseAliasKeys = [...report.camelCaseAliasKeys].sort();
  report.rules = ["camelCase-alias-drop", "timestamp-class", "duration-class", "language-fields", "extra-headers-only", "error-stack-elided", "id-placeholders"];
  return { normalized: JSON.parse(json), report };
}

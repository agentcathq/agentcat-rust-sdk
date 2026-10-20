// Normalizer: turns one run (raw mock-API requests + MCP transcript) into a
// language-neutral document that a TS run and a Rust run must match exactly.
// Rules are explicit and each one is listed in report.rules, so a rule can
// never silently hide a real difference.
//
// The document: {transcript, events, violations}. `violations` holds failed
// ordered checks ({rule, path, ...}); diff.mjs reports each one as a diff entry.

const ID_RE = /\b(evt|ses|agt)_[0-9A-Za-z]{27}\b/g;
const ISO_MS_Z = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/;
const PUBLISH_PATH = "/v1/public/events/";
const snake = (k) => k.replace(/[A-Z]/g, (c) => `_${c.toLowerCase()}`);

export const RULES = [
  "camelCase-alias-drop",
  "timestamp-class",
  "duration-class",
  "language-fields",
  "extra-headers-only",
  "error-stack-elided",
  "id-placeholders",
  "mcp-session-first",
  "property-order",
  "literal-derived-ids",
];

/** The key under which property-order records the key order of `inputSchema.properties`. */
export const PROPERTIES_ORDER_KEY = "<properties-order>";

// Fields that legitimately differ per language; replaced by a class token.
const LANGUAGE_FIELDS = {
  sdk_language: (v) => `<sdk_language:${typeof v}>`,
  agentcat_version: (v) => (typeof v === "string" && /^\d+\.\d+\.\d+/.test(v) ? "<semver>" : `<bad-version:${v}>`),
};

// Header names worth comparing (the backend reads user-agent and allowlisted
// IDs out of parameters by nested search, so only name and value matter).
const HEADER_KEEP = new Set(["user-agent", "x-anthropic-client", "x-test-header", "mcp-protocol-version"]);

function collectHeaders(node, out) {
  if (!node || typeof node !== "object") return;
  if (Array.isArray(node)) {
    for (const n of node) collectHeaders(n, out);
    return;
  }
  for (const [k, v] of Object.entries(node)) {
    if (k === "headers" && v && typeof v === "object" && !Array.isArray(v)) {
      for (const [hk, hv] of Object.entries(v)) {
        if (HEADER_KEEP.has(hk.toLowerCase())) out[hk.toLowerCase()] = hv;
      }
    } else collectHeaders(v, out);
  }
}

const isObject = (v) => v !== null && typeof v === "object" && !Array.isArray(v);

/** Calls `visit(node, path)` for every object and array in `node`, depth first. */
function walk(node, path, visit) {
  if (node === null || typeof node !== "object") return;
  visit(node, path);
  if (Array.isArray(node)) node.forEach((x, i) => walk(x, `${path}[${i}]`, visit));
  else for (const [k, v] of Object.entries(node)) walk(v, `${path}.${k}`, visit);
}

/** The session ID that resolveSessionId's canned string derives, as the TS SDK does. */
async function literalIds(scenario) {
  const spec = scenario?.options?.resolveSessionId;
  const value = isObject(spec) ? spec.returns : undefined;
  if (typeof value !== "string" || value.trim() === "" || typeof scenario.projectId !== "string") return [];
  const { loadModule } = await import("oracle/bundle");
  const { deriveSessionId } = await loadModule("handles");
  return [deriveSessionId(value.trim(), scenario.projectId)];
}

/**
 * Normalizes one run. `scenario` (optional) supplies the options profile and
 * project ID for literal-derived-ids. Resolves with {normalized, report}.
 */
export async function normalizeRun({ requests, transcript, scenario }) {
  const report = { camelCaseAliasKeys: new Set(), nonPublishRequests: [], rules: RULES, literalIds: [], violations: [] };
  const publishes = [];
  for (const r of requests) {
    if (r.path === PUBLISH_PATH && r.method === "POST") publishes.push(r);
    else report.nonPublishRequests.push({ method: r.method, path: r.path });
  }

  // Order: event timestamp, then resource name, then the body with its random
  // IDs masked (the queue publishes up to 5 events concurrently, so arrival
  // order is noise; masking keeps the order the same from run to run).
  const masked = (r) => JSON.stringify(r.body).replace(ID_RE, "<id>");
  publishes.sort(
    (a, b) =>
      String(a.body?.timestamp).localeCompare(String(b.body?.timestamp)) ||
      String(a.body?.resource_name).localeCompare(String(b.body?.resource_name)) ||
      masked(a).localeCompare(masked(b)),
  );

  const events = publishes.map((r) => {
    const body = structuredClone(r.body);
    if (!isObject(body)) return { mock_status: r.status, body };
    // camelCase-alias-drop: camelCase duplicates emitted by the TS generated
    // client (PublishEventRequestToJSON spreads the camelCase model first).
    for (const k of Object.keys(body)) {
      if (k !== snake(k) && snake(k) in body) {
        report.camelCaseAliasKeys.add(k);
        delete body[k];
      }
    }
    // timestamp-class: ISO 8601 with milliseconds and Z.
    if ("timestamp" in body) body.timestamp = ISO_MS_Z.test(body.timestamp) ? "<iso8601-ms-Z>" : `<bad-timestamp:${body.timestamp}>`;
    // duration-class: a non-negative integer of milliseconds.
    if ("duration" in body) body.duration = Number.isInteger(body.duration) && body.duration >= 0 ? "<duration-ms>" : `<bad-duration:${body.duration}>`;
    // language-fields: class tokens.
    for (const [k, f] of Object.entries(LANGUAGE_FIELDS)) if (k in body) body[k] = f(body[k]);
    // extra-headers-only: parameters.extra is the host MCP SDK's request
    // context; keep only backend-relevant headers by name.
    if (isObject(body.parameters) && "extra" in body.parameters) {
      const headers = {};
      collectHeaders(body.parameters.extra, headers);
      body.parameters.extra = { "<headers>": headers };
    }
    // error-stack-elided: stack and frames are language-specific; keep presence.
    if (isObject(body.error)) {
      for (const k of ["stack", "frames", "platform"]) if (k in body.error) body.error[k] = `<${k}>`;
    }
    return { mock_status: r.status, body };
  });

  const doc = { transcript: structuredClone(transcript), events, violations: [] };

  walk({ transcript: doc.transcript, events: doc.events }, "$", (node, path) => {
    if (Array.isArray(node)) return;
    // property-order: object comparison ignores key order, so the key order of
    // each listed tool's inputSchema.properties becomes an explicit array.
    if (Array.isArray(node.tools)) {
      for (const tool of node.tools) {
        if (isObject(tool) && isObject(tool.inputSchema) && isObject(tool.inputSchema.properties)) {
          tool.inputSchema[PROPERTIES_ORDER_KEY] = Object.keys(tool.inputSchema.properties);
        }
      }
    }
    // mcp-session-first: a structuredContent with mcp_session has it first.
    const sc = node.structuredContent;
    if (isObject(sc) && Object.hasOwn(sc, "mcp_session")) {
      const firstKey = Object.keys(sc)[0];
      if (firstKey !== "mcp_session") {
        doc.violations.push({ rule: "mcp-session-first", path: `${path}.structuredContent`, firstKey });
      }
    }
  });

  // id-placeholders: random IDs -> first-appearance placeholders, consistent
  // across transcript and events (so "the ses_ the client was issued is the
  // ses_ on the event" is still checked). literal-derived-ids: a session ID
  // derived from resolveSessionId's canned string is deterministic and stays.
  report.literalIds = await literalIds(scenario);
  const keep = new Set(report.literalIds);
  const ids = new Map();
  const counters = {};
  const json = JSON.stringify(doc).replace(ID_RE, (m, prefix) => {
    if (keep.has(m)) return m;
    if (!ids.has(m)) {
      counters[prefix] = (counters[prefix] ?? 0) + 1;
      ids.set(m, `<${prefix}#${counters[prefix]}>`);
    }
    return ids.get(m);
  });
  const normalized = JSON.parse(json);
  report.camelCaseAliasKeys = [...report.camelCaseAliasKeys].sort();
  report.violations = normalized.violations;
  return { normalized, report };
}

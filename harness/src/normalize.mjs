// Normalizer: turns one run (raw mock-API requests + MCP transcript) into a
// language-neutral document that a TS run and a Rust run must match exactly.
// Rules are explicit and each one is listed in report.rules, so a rule can
// never silently hide a real difference.
//
// The document: {transcript, events, nonPublishRequests, violations}.
// `violations` holds failed ordered checks ({rule, path, ...}); diff.mjs
// reports each one as a diff entry.

const ID_RE = /\b(evt|ses|agt)_[0-9A-Za-z]{27}\b/g;
const ISO_Z = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{3})?Z$/;
const PUBLISH_PATH = "/v1/public/events/";
const snake = (k) => k.replace(/[A-Z]/g, (c) => `_${c.toLowerCase()}`);

export const RULES = [
  "request-fields",
  "number-source",
  "camelCase-alias-drop",
  "timestamp-class",
  "duration-class",
  "language-fields",
  "extra-projection",
  "error-shape-class",
  "id-placeholders",
  "mcp-session-first",
  "property-order",
  "server-info-meta-drop",
  "literal-derived-ids",
];

/** The key under which property-order records the key order of `inputSchema.properties`. */
export const PROPERTIES_ORDER_KEY = "<properties-order>";

/**
 * The `_meta` key that the TS v2 host library (`@modelcontextprotocol/server`) adds to every
 * 2026-07-28 result; rmcp adds none. server-info-meta-drop drops it from transcript results (RD-13).
 */
export const SERVER_INFO_META_KEY = "io.modelcontextprotocol/serverInfo";

/** The sdk_language values the backend knows. */
export const SDK_LANGUAGES = ["TypeScript", "Rust"];

// Fields that legitimately differ per language; replaced by a class token.
const LANGUAGE_FIELDS = {
  sdk_language: (v) => (SDK_LANGUAGES.includes(v) ? "<sdk_language>" : `<bad-sdk_language:${v}>`),
  agentcat_version: (v) => (typeof v === "string" && /^\d+\.\d+\.\d+/.test(v) ? "<semver>" : `<bad-version:${v}>`),
};

const kind = (v) => (Array.isArray(v) ? "array" : v === null ? "null" : typeof v);

/** frames -> `<frames:array:<sorted union of frame keys>>`, or `<frames:<type>>`. */
function framesToken(frames) {
  if (!Array.isArray(frames)) return `<frames:${kind(frames)}>`;
  const keys = new Set();
  for (const frame of frames) {
    if (isObject(frame)) for (const k of Object.keys(frame)) keys.add(k);
    else keys.add(`<${kind(frame)}>`);
  }
  return `<frames:array:${[...keys].sort().join(",")}>`;
}

/** error-shape-class: language-specific error fields -> tokens that keep their shape. */
function classifyError(error) {
  if ("stack" in error) error.stack = error.stack === "" ? "<stack:empty>" : `<stack:${kind(error.stack)}>`;
  if ("frames" in error) error.frames = framesToken(error.frames);
  if ("platform" in error) error.platform = `<platform:${kind(error.platform)}>`;
}

// extra-projection: header values that differ from run to run, and their tokens.
const VOLATILE_HEADERS = { host: "<host>", "content-length": "<content-length>", "mcp-session-id": "<mcp-session-id>" };

/** Copies every header of every "headers" object under `node` into `out`, by lower-case name. */
function collectHeaders(node, out) {
  if (!node || typeof node !== "object") return;
  if (Array.isArray(node)) {
    for (const n of node) collectHeaders(n, out);
    return;
  }
  for (const [k, v] of Object.entries(node)) {
    if (k === "headers" && v && typeof v === "object" && !Array.isArray(v)) {
      for (const [hk, hv] of Object.entries(v)) {
        const name = hk.toLowerCase();
        out[name] = Object.hasOwn(VOLATILE_HEADERS, name) ? VOLATILE_HEADERS[name] : hv;
      }
    } else collectHeaders(v, out);
  }
}

/** A URL with its port (the server's ephemeral port) as `<port>`. */
const maskPort = (url) => url.replace(/^([A-Za-z][A-Za-z0-9+.-]*:\/\/[^/?#]*):\d+(?=[/?#]|$)/, "$1:<port>");

/**
 * extra-projection: the host MCP SDK's request context as `{"<keys>": <its sorted top-level keys,
 * comma-joined>, "<headers>": <every header of a "headers" object anywhere in it>, "<url>": <the
 * request URL>}`; a part the context does not have is left out (stdio has no headers and no URL).
 */
function projectExtra(extra) {
  if (!isObject(extra)) return extra;
  const out = { "<keys>": Object.keys(extra).sort().join(",") };
  const headers = {};
  collectHeaders(extra, headers);
  if (Object.keys(headers).length > 0) out["<headers>"] = headers;
  const url = extra.requestInfo?.url ?? extra.http?.req?.url;
  if (typeof url === "string") out["<url>"] = maskPort(url);
  return out;
}

const isObject = (v) => v !== null && typeof v === "object" && !Array.isArray(v);

/** Calls `visit(node, path)` for every object and array in `node`, depth first. */
function walk(node, path, visit) {
  if (node === null || typeof node !== "object") return;
  visit(node, path);
  if (Array.isArray(node)) node.forEach((x, i) => walk(x, `${path}[${i}]`, visit));
  else for (const [k, v] of Object.entries(node)) walk(v, `${path}.${k}`, visit);
}

// number-source needs the source text of each number (JSON.parse source text access, Node 22).
const SOURCE_TEXT = (() => {
  let source;
  JSON.parse("1.0", (key, value, context) => {
    source = context?.source;
    return value;
  });
  return source === "1.0";
})();

/**
 * number-source: the publish body, read from the raw text the mock recorded. An integer number below
 * 1e21 in magnitude whose source text is not the text JavaScript writes for it (`1.0`, `-0`, `1e2`, an
 * exact integer above 2^53) becomes `<non-js-number:<source>>`; every other number compares by value,
 * so a float written as serde_json writes it equals TS's (RD-32). A request without raw text, or whose
 * text is not JSON, keeps its parsed body.
 */
function publishBody(r) {
  if (typeof r.rawBody !== "string") return structuredClone(r.body);
  if (!SOURCE_TEXT) throw new Error("normalize: number-source needs JSON.parse source text access (Node 22 or later)");
  try {
    return JSON.parse(r.rawBody, (key, value, context) =>
      typeof value === "number" && Number.isInteger(value) && Math.abs(value) < 1e21 && context.source !== String(value)
        ? `<non-js-number:${context.source}>`
        : value,
    );
  } catch {
    return structuredClone(r.body);
  }
}

// id-placeholders: a derived ses_ ID (B-065) has a KSUID time between 2024-01-01 and about
// 2024-02-19; a minted one has the time it was minted.
const BASE62 = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const KSUID_EPOCH_S = 1_400_000_000;
const DERIVED_BEFORE_MS = Date.UTC(2024, 2, 1);

/** The time (epoch ms) in the KSUID of an ID `<prefix>_<27 base62 characters>`. */
function ksuidTimeMs(id) {
  let n = 0n;
  for (const c of id.slice(id.indexOf("_") + 1)) n = n * 62n + BigInt(BASE62.indexOf(c));
  return (Number(n >> 128n) + KSUID_EPOCH_S) * 1000;
}

const derived = (id) => id.startsWith("ses_") && ksuidTimeMs(id) < DERIVED_BEFORE_MS;

/** A string that is equal for deeply equal JSON values, whatever the key order. */
function canon(v) {
  if (Array.isArray(v)) return `[${v.map(canon).join(",")}]`;
  if (isObject(v)) return `{${Object.keys(v).sort().map((k) => `${JSON.stringify(k)}:${canon(v[k])}`).join(",")}}`;
  return JSON.stringify(v) ?? "undefined";
}

/** The time of a raw event timestamp (epoch ms), or Infinity when it is not one. */
function timeOf(timestamp) {
  const t = typeof timestamp === "string" ? Date.parse(timestamp) : Number.NaN;
  return Number.isNaN(t) ? Number.POSITIVE_INFINITY : t;
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
 * project ID for literal-derived-ids, and `eventOrder`. Resolves with {normalized, report}.
 */
export async function normalizeRun({ requests, transcript, scenario }) {
  const report = { camelCaseAliasKeys: new Set(), nonPublishRequests: [], rules: RULES, literalIds: [], violations: [] };
  const publishes = [];
  // request-fields: every non-publish request stays in the document as
  // {method, path, query, status}, sorted (arrival order is noise).
  const nonPublishRequests = [];
  for (const r of requests) {
    if (r.path === PUBLISH_PATH && r.method === "POST") publishes.push(r);
    else nonPublishRequests.push({ method: r.method, path: r.path, query: r.query ?? "", status: r.status });
  }
  nonPublishRequests.sort((a, b) => JSON.stringify(a).localeCompare(JSON.stringify(b)));
  report.nonPublishRequests = nonPublishRequests;

  const normalizedEvents = publishes.map((r) => {
    // number-source: the body as the server wrote it.
    const raw = publishBody(r);
    const body = structuredClone(raw);
    // request-fields: a publish keeps its status, content-type and query;
    // its other headers (user-agent, accept) are language-specific.
    const entry = { mock_status: r.status, content_type: r.headers?.["content-type"] ?? null, query: r.query ?? "", body };
    if (!isObject(body)) return { raw, entry };
    // camelCase-alias-drop: camelCase duplicates emitted by the TS generated
    // client (PublishEventRequestToJSON spreads the camelCase model first).
    for (const k of Object.keys(body)) {
      if (k !== snake(k) && snake(k) in body) {
        report.camelCaseAliasKeys.add(k);
        delete body[k];
      }
    }
    // timestamp-class: ISO 8601 with milliseconds, or whole seconds (RD-04), and Z.
    if ("timestamp" in body) body.timestamp = ISO_Z.test(body.timestamp) ? "<iso8601-Z>" : `<bad-timestamp:${body.timestamp}>`;
    // duration-class: a non-negative integer of milliseconds.
    if ("duration" in body) body.duration = Number.isInteger(body.duration) && body.duration >= 0 ? "<duration-ms>" : `<bad-duration:${body.duration}>`;
    // language-fields: class tokens.
    for (const [k, f] of Object.entries(LANGUAGE_FIELDS)) if (k in body) body[k] = f(body[k]);
    // extra-projection: parameters.extra is the host MCP SDK's request context.
    if (isObject(body.parameters) && "extra" in body.parameters) body.parameters.extra = projectExtra(body.parameters.extra);
    // error-shape-class: stack, frames and platform differ per language;
    // their tokens keep the type and, for frames, the frame keys.
    if (isObject(body.error)) {
      classifyError(body.error);
      if (Array.isArray(body.error.chained_errors)) {
        for (const chained of body.error.chained_errors) if (isObject(chained)) classifyError(chained);
      }
    }
    return { raw, entry };
  });

  // Order: the event's time (its raw timestamp); in the same millisecond, an
  // event with a duration (a call, which can enclose events published during
  // it) before one without; then the resource name, then the normalized body
  // with its random IDs masked, key order aside (the queue publishes up to 5
  // events concurrently, so arrival order is noise; masking keeps the order
  // the same from run to run, and the normalized body has no raw duration or
  // timestamp to decide a tie). A scenario with "eventOrder": "multiset"
  // sorts on the masked normalized body alone.
  const masked = (e) => canon(e.entry.body).replace(ID_RE, "<id>");
  for (const e of normalizedEvents) e.key = masked(e);
  const byBody = (a, b) => (a.key < b.key ? -1 : a.key > b.key ? 1 : 0);
  const untimed = (e) => (typeof e.raw?.duration === "number" ? 0 : 1);
  const byTime = (a, b) =>
    timeOf(a.raw?.timestamp) - timeOf(b.raw?.timestamp) ||
    String(a.raw?.timestamp).localeCompare(String(b.raw?.timestamp)) ||
    untimed(a) - untimed(b) ||
    String(a.raw?.resource_name).localeCompare(String(b.raw?.resource_name)) ||
    byBody(a, b);
  normalizedEvents.sort(scenario?.eventOrder === "multiset" ? byBody : byTime);
  const events = normalizedEvents.map((e) => e.entry);

  const doc = { transcript: structuredClone(transcript), events, nonPublishRequests, violations: [] };

  // server-info-meta-drop: host behavior, not AgentCat's (RD-13). The TS v2 host library puts
  // _meta["io.modelcontextprotocol/serverInfo"] on every 2026-07-28 result and rmcp puts none, so
  // a result that only the tracked run has (an intercepted get_more_tools) differs by that key
  // alone. Only that key goes; a _meta left empty goes too. Other _meta keys stay.
  for (const entry of doc.transcript) {
    const meta = isObject(entry) && isObject(entry.result) ? entry.result._meta : undefined;
    if (isObject(meta) && Object.hasOwn(meta, SERVER_INFO_META_KEY)) {
      delete meta[SERVER_INFO_META_KEY];
      if (Object.keys(meta).length === 0) delete entry.result._meta;
    }
  }

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
  // ses_ on the event" is still checked); a derived ses_ ID (B-065) gets
  // <ses-derived#n>, so a derived ID never stands in for a minted one.
  // literal-derived-ids: a session ID derived from resolveSessionId's canned
  // string is deterministic and stays.
  report.literalIds = await literalIds(scenario);
  const keep = new Set(report.literalIds);
  const ids = new Map();
  const counters = {};
  const json = JSON.stringify(doc).replace(ID_RE, (m, prefix) => {
    if (keep.has(m)) return m;
    if (!ids.has(m)) {
      const tag = derived(m) ? "ses-derived" : prefix;
      counters[tag] = (counters[tag] ?? 0) + 1;
      ids.set(m, `<${tag}#${counters[tag]}>`);
    }
    return ids.get(m);
  });
  const normalized = JSON.parse(json);
  report.camelCaseAliasKeys = [...report.camelCaseAliasKeys].sort();
  report.violations = normalized.violations;
  return { normalized, report };
}

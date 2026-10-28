// The comparison of the two demo servers' publishes and tool lists (plan 5, Task 17). Pure functions, so that the
// tests feed them small lists. RD-39 is the one expected difference: see EXPECTED_RD39.

/** Sorts the keys of every object, so that two JSON values with the same content have the same text. */
export function canonical(value) {
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
  if (value !== null && typeof value === "object") {
    return `{${Object.keys(value).sort().map((k) => `${JSON.stringify(k)}:${canonical(value[k])}`).join(",")}}`;
  }
  return JSON.stringify(value);
}

const field = (body, snake, camel) => body?.[snake] ?? body?.[camel];

/**
 * The fields of one publish that the two servers must share. `sessions` are the `ses_` IDs the client saw for the
 * server, in order: a publish's session is its index there (`null`: no session, or one the client never saw).
 * The error keeps its type and message; its platform, stack and frames belong to the SDK of each language.
 */
export function normalizeEvent(request, sessions) {
  const body = request.body ?? {};
  const sessionId = field(body, "session_id", "sessionId");
  const index = sessions.indexOf(sessionId);
  const error = body.error;
  return {
    type: field(body, "event_type", "eventType") ?? null,
    name: field(body, "resource_name", "resourceName") ?? null,
    userId: field(body, "identify_actor_given_id", "identifyActorGivenId") ?? null,
    userName: field(body, "identify_actor_name", "identifyActorName") ?? null,
    tags: body.tags ?? null,
    source: body.properties?.source ?? null,
    redactedBy: body.properties?.redacted_by ?? null,
    isError: field(body, "is_error", "isError") ?? null,
    error: error ? { type: error.type ?? null, message: error.message ?? null } : null,
    session: index >= 0 ? index : null,
    redacted: String(request.rawBody ?? "").includes("[REDACTED]"),
    leaked: String(request.rawBody ?? "").includes("secret-token"),
  };
}

/** RD-39: the TS custom event of the first call of the second session has no session; the Rust one has it. */
export const EXPECTED_RD39 = { type: "agentcat:custom", name: "echo-called", rustSession: 1, tsSession: null };

/**
 * Compares the publishes of the Rust server with those of the TS server. Each side is `{ requests, sessions }`.
 * `secretSteps` is the number of scenario calls that send secret-token. Returns a list of failure texts and the
 * number of events compared. Events are paired after sorting by their normalized text, so delivery order does
 * not matter.
 */
export function compareEvents({ rust, ts, secretSteps }) {
  const failures = [];
  const side = (data) => data.requests.map((request) => normalizeEvent(request, data.sessions));
  const rustEvents = side(rust);
  let tsEvents = side(ts);

  for (const [label, events] of [["rust", rustEvents], ["ts", tsEvents]]) {
    if (events.some((e) => e.leaked)) failures.push(`${label}: a publish still holds secret-token`);
    const redacted = events.filter((e) => e.redacted).length;
    if (redacted !== secretSteps) failures.push(`${label}: ${redacted} publish(es) hold [REDACTED], expected ${secretSteps}`);
  }

  // RD-39: the TS custom event without a session must be there exactly once, and the Rust one must not lack it.
  const isRd39 = (e) => e.type === EXPECTED_RD39.type && e.name === EXPECTED_RD39.name;
  const tsMissing = tsEvents.filter((e) => isRd39(e) && e.session === EXPECTED_RD39.tsSession);
  const rustMissing = rustEvents.filter((e) => isRd39(e) && e.session === null);
  if (rustMissing.length > 0) failures.push(`rust: ${rustMissing.length} custom event(s) have no session; RD-39 expects the Rust ones to have it`);
  if (tsMissing.length !== 1) {
    failures.push(`ts: ${tsMissing.length} custom event(s) have no session, expected exactly 1 (RD-39); if the TS SDK now gives it, update RD-39`);
  } else {
    // Replace the expected difference by the Rust value; any other difference then shows up below.
    const target = tsMissing[0];
    tsEvents = tsEvents.map((e) => (e === target ? { ...e, session: EXPECTED_RD39.rustSession } : e));
  }

  const key = (e) => canonical(e);
  const a = rustEvents.map(key).sort();
  const b = tsEvents.map(key).sort();
  if (a.length !== b.length) failures.push(`event counts differ: rust ${a.length}, ts ${b.length}`);
  const seen = new Map();
  for (const text of a) seen.set(text, (seen.get(text) ?? 0) + 1);
  for (const text of b) seen.set(text, (seen.get(text) ?? 0) - 1);
  for (const [text, n] of seen) {
    if (n > 0) failures.push(`${n} event(s) only in rust: ${text}`);
    if (n < 0) failures.push(`${-n} event(s) only in ts: ${text}`);
  }
  return { failures, compared: Math.min(a.length, b.length) };
}

/** Compares two tools/list results by name, description and input schema (key order does not matter). */
export function compareTools(rustTools, tsTools) {
  const failures = [];
  const byName = (tools) => new Map(tools.map((tool) => [tool.name, tool]));
  const r = byName(rustTools);
  const t = byName(tsTools);
  for (const name of new Set([...r.keys(), ...t.keys()])) {
    if (!r.has(name) || !t.has(name)) {
      failures.push(`tools/list: ${name} is only in ${r.has(name) ? "rust" : "ts"}`);
      continue;
    }
    for (const part of ["description", "inputSchema", "annotations", "outputSchema"]) {
      if (canonical(r.get(name)[part] ?? null) !== canonical(t.get(name)[part] ?? null)) {
        failures.push(`tools/list: ${name} ${part} differs: rust ${canonical(r.get(name)[part] ?? null)}, ts ${canonical(t.get(name)[part] ?? null)}`);
      }
    }
  }
  return failures;
}

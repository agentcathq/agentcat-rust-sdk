// Vectors for TS's event queue (src/modules/eventQueue.ts): each case builds one event the way TS
// does and runs it through TS's own EventQueue: `add`, then `processEvent` with the API client
// replaced by a recorder and the backoff delay replaced by a no-op that records the milliseconds.
// A tool-call event goes through publishEvent (eventInput built as callWrap builds it, its hooks fired
// through TS's own resolvers; an error result gets TS's captureException error, and the protocol
// version of `call.protocol_version` becomes the request's `_meta` and the SDK's protocol tag); a
// custom event through publishCustomEvent. A fresh queue serves each
// case; the module's queue only hands its events over. node:test mock timers freeze the clock at
// NOW and drive the 30 s hook budget until the queue settles (a watchdog of 5 s real time names a case
// that never does). Nothing reaches the network.
//
// Case name prefixes: call:, hooks:, redact-event:, redact-strings:, sanitize:, truncate:, retry:,
// custom:. Fields of a case (inputs, then outputs):
//   ts_test         the TS tests the case reproduces, when one does
//   hook_mode       true: a resolveSessionId hook is configured (call cases only)
//   custom          custom cases only: {form: "string", session, project_id, data} or
//                   {form: "server", project_id, tracked_project_id, server_info, client_info, data}:
//                   the publishCustomEvent arguments (`session` is the session-string argument, `data`
//                   the CustomEventData) and, for the server form, the tracked server's project and the
//                   identities its events carry (`_serverInfo` and getClientVersion())
//   event           the in-memory event as publishEvent or publishCustomEvent hands it to the queue, in
//                   TS key order: camelCase keys, `timestamp` an ISO string, no functions, no `pending`,
//                   no undefined values (event_order.json lists the undefined keys)
//   hooks           raw outcomes of the call hooks, by TS option name (resolveSessionId, identify,
//                   eventTags, eventProperties): {"returns": value}, {"throws": message} (an Error) or
//                   {"hangs": true} (never settles)
//   redact_event    the redactEvent hook: {"returns": null}, {"throws": message} or {"merge": fields}
//                   (returns {...event, ...fields}); tool calls and server-form custom events only
//   redact_strings  the redactSensitiveInformation hook: {"replace": secret} (each occurrence becomes
//                   "[REDACTED]") or {"throw_at": n} (call n throws Error("redactor failed"), the others
//                   return their text); TS runs it on tool calls only (K-03), as the server-form case
//                   that configures one shows
//   fail_attempts   how many publish attempts reject with Error("mock failure")
//   requests        each PublishEventRequest the API client received: TS's camelCase model before the
//                   generated serializer, without its undefined fields, `timestamp` an ISO string
//   delays          the backoff milliseconds the queue asked for
//   logs            TS's log lines, without the timestamp and version head
// Every `evt_` ID is "<evt>" and every minted session ID "<minted>", in requests and logs. A rejected
// publish is an Error made with Error.stackTraceLimit = 0, so TS's error text, which holds the stack,
// is the same on every machine.
import "../../egress-guard.mjs"; // first import: nothing below may reach the network

import { mock } from "node:test";

import { loadCore } from "../../bundle.mjs";
import { captureLogs } from "../../logs.mjs";
import { tsKey, withWatchdog } from "../support.mjs";

/** The frozen clock of every case. */
const NOW = Date.parse("2026-10-23T12:00:00.000Z");
/** When a tool call started; its duration is set, so the clock never computes one. */
const CALL_START = "2026-10-23T11:59:59.250Z";
const CALL_DURATION = 750;
const PROJECT = "proj_queue";
const SERVER_INFO = { name: "queue-server", version: "1.0.0" };
const CLIENT_INFO = { name: "queue-client", version: "2.0.0" };
const SECRET = "SECRET-VALUE-42";
/** Real milliseconds the queue may take to settle before the case fails. */
const WATCHDOG_MS = 5_000;
const EVT_ID = /evt_[0-9A-Za-z]{27}/g;

/** A valid session ID that reads as its label (TS's test helper `sid`). */
function sid(label) {
  return `ses_${`${label.replace(/[^0-9A-Za-z]/g, "")}${"0".repeat(27)}`.slice(0, 27)}`;
}

const SESSION = sid("queue");

/** A JSON copy of `value`. */
const copyJson = (value) => (value === undefined ? undefined : JSON.parse(JSON.stringify(value)));

/** `object` without undefined values and functions, `timestamp` (a Date) as its ISO string. */
function plain(object, skip = []) {
  const out = {};
  for (const [key, value] of Object.entries(object)) {
    if (value === undefined || typeof value === "function" || skip.includes(key)) continue;
    out[key] = key === "timestamp" && value instanceof Date ? value.toISOString() : copyJson(value);
  }
  return out;
}

// ------------------------------------------------------------------------------------------ hooks

/** A hook function with the raw outcome `outcome`. */
function hookFrom(outcome) {
  if ("hangs" in outcome) return () => new Promise(() => {});
  if ("throws" in outcome) {
    return async () => {
      throw new Error(outcome.throws);
    };
  }
  return async () => copyJson(outcome.returns);
}

/** The redactEvent hook of `spec` (redact_event), or undefined. */
function eventRedactor(spec) {
  if (spec === undefined) return undefined;
  if ("throws" in spec) {
    return () => {
      throw new Error(spec.throws);
    };
  }
  if ("merge" in spec) return (event) => ({ ...event, ...copyJson(spec.merge) });
  return () => null;
}

/** The redactSensitiveInformation hook of `spec` (redact_strings), or undefined. */
function stringRedactor(spec) {
  if (spec === undefined) return undefined;
  if ("replace" in spec) return async (text) => text.replaceAll(spec.replace, "[REDACTED]");
  let calls = 0;
  return async (text) => {
    calls += 1;
    if (calls === spec.throw_at) throw new Error("redactor failed");
    return text;
  };
}

/** The tracking data of a server with project `projectId` and the hooks of `spec`. */
function trackingData(spec, projectId) {
  const hooks = spec.hooks ?? {};
  const options = {
    enableTracing: true,
    enableToolCallContext: true,
    enableAgentTracking: false,
    redactEvent: eventRedactor(spec.redact_event),
    redactSensitiveInformation: stringRedactor(spec.redact_strings),
  };
  for (const name of ["resolveSessionId", "identify", "eventTags", "eventProperties"]) {
    if (hooks[name] !== undefined) options[name] = hookFrom(hooks[name]);
  }
  return { projectId, options };
}

/** A low-level server as publishEvent reads it, tracked with `data`. */
function trackedServer(core, data, serverInfo = SERVER_INFO, clientInfo = CLIENT_INFO) {
  const server = { _serverInfo: copyJson(serverInfo), getClientVersion: () => copyJson(clientInfo) };
  core.internal.setServerTrackingData(server, data);
  return server;
}

// ------------------------------------------------------------------------------------- publishing

/**
 * Publishes the tool call of `spec` as src/engine/callWrap.ts does: resolves the handles, fires the
 * configured hooks through TS's resolvers, builds the eventInput in callWrap's order and hands it to
 * publishEvent. `spec.input` replaces the whole eventInput (the TS tests that call publishEvent
 * directly); `spec.call` sets the call's parts.
 */
function publishCall(core, spec) {
  const projectId = spec.project_id ?? PROJECT;
  const data = trackingData(spec, projectId);
  const server = trackedServer(core, data);
  if (spec.input !== undefined) {
    const input = { ...copyJson(spec.input), timestamp: new Date(CALL_START) };
    core.eventQueue.publishEvent(server, input, { clientInfo: CLIENT_INFO });
    return;
  }
  const call = spec.call ?? {};
  const name = call.name ?? "echo";
  const args = { msg: "hello", ...(call.args ?? {}) };
  // A supplied session keeps the event deterministic; hook mode injects no session_id parameter.
  if (!spec.hook_mode) args.session_id = SESSION;
  const request = { method: "tools/call", params: { name, arguments: args }, jsonrpc: "2.0", id: 1 };
  if (call.protocol_version !== undefined) {
    request.params._meta = { [core.constants.META_PROTOCOL_VERSION_KEY]: call.protocol_version };
  }
  const extra = { requestId: 1 };
  const resolution = core.handles.resolveHandles(data.options, projectId || undefined, request, extra, true);

  const pending = {};
  if (resolution.hookMode) pending.sessionHookValue = core.handles.invokeSessionHook(data.options, request, extra);
  if (data.options.identify) pending.identity = core.internal.resolveIdentity(data, request, extra);
  if (data.options.eventTags) pending.tags = core.internal.resolveEventTags(data, request, extra);
  if (data.options.eventProperties) pending.properties = core.internal.resolveEventProperties(data, request, extra);

  const event = {
    sessionId: resolution.sessionId,
    resourceName: name,
    parameters: { request, extra },
    inputTokens: core.tokenEstimate.estimateInputTokens(args),
    eventType: "mcp:tools/call",
    timestamp: new Date(CALL_START),
    redactionFn: data.options.redactSensitiveInformation,
  };
  if (Object.keys(pending).length > 0) event.pending = pending;
  event.tags = { ...core.handles.buildHandleTags(resolution, core.session.getProtocolVersion(request, extra)) };
  if ("context" in args) event.userIntent = args.context;
  const response = copyJson(call.response ?? { content: [{ type: "text", text: "hello" }] });
  // As callWrap does for an error result: the event records TS's own captureException of the result.
  if (response.isError === true) {
    event.isError = true;
    event.error = core.exceptions.captureException(response);
  }
  event.response = response;
  event.outputTokens = core.tokenEstimate.estimateOutputTokens(response);
  event.duration = CALL_DURATION;
  core.eventQueue.publishEvent(server, event, { clientInfo: CLIENT_INFO });
}

/** Publishes the custom event of `spec.custom` through TS's publishCustomEvent. */
async function publishCustom(core, spec) {
  const { form, session, project_id: projectId, data } = spec.custom;
  if (form === "string") {
    await core.sdk.publishCustomEvent(session, projectId, copyJson(data));
    return;
  }
  const { tracked_project_id: tracked, server_info: serverInfo, client_info: clientInfo } = spec.custom;
  const server = trackedServer(core, trackingData(spec, tracked), serverInfo, clientInfo);
  await core.sdk.publishCustomEvent(server, projectId, copyJson(data));
}

// ------------------------------------------------------------------------------------------- queue

/** A fresh TS EventQueue whose API client records each request and fails the first `failAttempts`. */
function recordingQueue(core, failAttempts) {
  const queue = new core.eventQueue.eventQueue.constructor();
  const requests = [];
  const delays = [];
  const runs = [];
  let added = 0;
  let attempts = 0;
  queue.apiClient = {
    publishEvent: async ({ publishEventRequest }) => {
      requests.push(plain(publishEventRequest));
      attempts += 1;
      if (attempts <= failAttempts) throw stacklessError("mock failure");
      return {};
    },
  };
  queue.delay = async (ms) => {
    delays.push(ms);
  };
  const add = queue.add.bind(queue);
  queue.add = (event) => {
    added += 1;
    return add(event);
  };
  const processEvent = queue.processEvent.bind(queue);
  queue.processEvent = (queued) => {
    const run = processEvent(queued);
    runs.push(run);
    return run;
  };
  /**
   * Resolves once the queue has taken an event and every run of processEvent has finished (a run
   * that another run starts is awaited too). Rejects with a named error after `WATCHDOG_MS` of real
   * time, for a run that never finishes (a hook that hangs while the mock clock stands still).
   */
  const settled = (name) => {
    const finished = (async () => {
      for (;;) {
        await drain();
        if (added === 0) continue;
        const seen = runs.length;
        await Promise.all(runs);
        if (runs.length === seen) return;
      }
    })();
    return withWatchdog(finished, WATCHDOG_MS, `oracle: case ${name}: the queue did not settle within ${WATCHDOG_MS} ms`);
  };
  return { queue, requests, delays, settled };
}

/** An Error whose stack is its first line only, the same on every machine. */
function stacklessError(message) {
  const limit = Error.stackTraceLimit;
  Error.stackTraceLimit = 0;
  try {
    return new Error(message);
  } finally {
    Error.stackTraceLimit = limit;
  }
}

/** Freezes Date at NOW and mocks setTimeout, without Node's experimental-API warning for it. */
function freezeClock() {
  const emit = process.emitWarning;
  process.emitWarning = function quiet(warning, ...rest) {
    if (String(warning).startsWith("The MockTimers API")) return undefined;
    return emit.call(this, warning, ...rest);
  };
  try {
    mock.timers.enable({ apis: ["setTimeout", "Date"], now: NOW });
  } finally {
    process.emitWarning = emit;
  }
}

/** Resolves after the pending promise callbacks have run; real `setImmediate` is not mocked. */
const drain = () => new Promise((resolve) => setImmediate(resolve));

/** Runs `spec` through TS; resolves with its event, requests, delays and logs, masked. */
async function run(core, spec) {
  const recorder = recordingQueue(core, spec.fail_attempts ?? 0);
  const hangs = Object.values(spec.hooks ?? {}).some((outcome) => outcome.hangs);
  const shared = core.eventQueue.eventQueue;
  const ownAdd = Object.hasOwn(shared, "add") ? shared.add : undefined;
  const events = [];
  shared.add = (event) => {
    events.push(plain(event, ["pending"]));
    recorder.queue.add(event);
  };
  let logs;
  try {
    freezeClock();
    ({ logs } = await captureLogs(core, async () => {
      await (spec.custom === undefined ? publishCall(core, spec) : publishCustom(core, spec));
      // A hook that hangs ends at the 30 s budget: advance the mock clock until the queue is done.
      let done = false;
      const settled = recorder.settled(spec.name).finally(() => {
        done = true;
      });
      settled.catch(() => {}); // the await below reports it
      while (hangs && !done) {
        await drain(); // every hook that settles has settled
        if (!done) mock.timers.tick(core.pendingEvent.PENDING_HOOK_TIMEOUT_MS);
      }
      await settled;
    }));
  } finally {
    mock.timers.reset();
    if (ownAdd === undefined) delete shared.add;
    else shared.add = ownAdd;
  }
  if (events.length !== 1) throw new Error(`oracle: case ${spec.name}: ${events.length} events reached the queue`);
  return mask(core, { event: events[0], requests: recorder.requests, delays: recorder.delays, logs });
}

/** `out` with each `evt_` ID as "<evt>" and each minted session ID (TS's tag says so) as "<minted>". */
function mask(core, out) {
  const minted = new Set();
  for (const request of out.requests) {
    if (!/^evt_[0-9A-Za-z]{27}$/.test(request.id)) throw new Error(`oracle: unexpected event ID ${request.id}`);
    if (request.tags?.agentcat_session_id_source === "minted") {
      if (!core.handles.isValidSessionId(request.sessionId)) throw new Error(`oracle: bad minted ID ${request.sessionId}`);
      minted.add(request.sessionId);
    }
  }
  const text = (s) => [...minted].reduce((t, id) => t.replaceAll(id, "<minted>"), s.replace(EVT_ID, "<evt>"));
  return {
    event: out.event,
    requests: out.requests.map((request) => JSON.parse(text(JSON.stringify(request)))),
    delays: out.delays,
    logs: out.logs.map(text),
  };
}

// ------------------------------------------------------------------------------------------- cases

const specs = [];

/** Adds a case; `tsTests` are [file, title] pairs of the TS tests it reproduces. */
function add(name, spec, tsTests = []) {
  const fail = (message) => {
    throw new Error(`oracle: case ${name}: ${message}`);
  };
  if (Boolean(spec.hook_mode) !== (spec.hooks?.resolveSessionId !== undefined)) fail("hook_mode goes with a resolveSessionId hook");
  if (spec.custom !== undefined && (spec.hooks !== undefined || spec.hook_mode)) fail("custom events run no call hooks");
  if (spec.custom?.form === "string" && (spec.redact_event ?? spec.redact_strings) !== undefined) {
    fail("a session-string custom event gets no redaction hook");
  }
  specs.push({ name, ...spec, ...(tsTests.length > 0 && { ts_test: tsTests.map(([file, title]) => tsKey(file, title)) }) });
}

const QUEUE_TEST = "eventQueue";
const TAGS_TEST = "event-tags-properties";
const CUSTOM_TEST = "publishCustomEvent";

function callCases() {
  add("call:plain", { call: { args: { context: "say hello" } } });
  add("call:tool-error", { call: { response: { content: [{ type: "text", text: "the lookup failed" }], isError: true } } });
  add("call:protocol-version", { call: { protocol_version: "2025-11-25" } });
  add("call:telemetry-only", { project_id: "" });
  add(
    "call:no-session",
    { input: { eventType: "mcp:tools/call", resourceName: "do_thing" } },
    [[QUEUE_TEST, "EventQueue > publishEvent > should publish an absent sessionId as empty string and send it as null"]],
  );
  add(
    "call:tokens",
    { input: { eventType: "mcp:tools/call", resourceName: "do_thing", inputTokens: 6, outputTokens: 4 } },
    [[QUEUE_TEST, "EventQueue > publishEvent > should send inputTokens and outputTokens on the wire request"]],
  );
  add(
    "call:no-tokens",
    { input: { eventType: "mcp:tools/call", resourceName: "do_thing" } },
    [[QUEUE_TEST, "EventQueue > publishEvent > should leave the token fields undefined when the event has none"]],
  );
  const leak = "TOP_SECRET_PAYLOAD_abc123";
  add(
    "call:log-metadata-only",
    {
      input: {
        sessionId: "test-session",
        eventType: "mcp:tools/call",
        resourceName: "do_thing",
        parameters: { request: { params: { arguments: { apiKey: leak } } } },
        response: { content: [{ type: "text", text: leak }] },
        userIntent: leak,
      },
    },
    [[QUEUE_TEST, "EventQueue > Integration tests > logs only event metadata on successful send, never the payload"]],
  );
}

function hookCases() {
  const user = { userId: "user-42", userName: "Ada", userData: { plan: "pro", seats: 3 } };
  add("hooks:identify", { hooks: { identify: { returns: user } } });
  add("hooks:identify-no-data", { hooks: { identify: { returns: { userId: "user-7" } } } });
  add("hooks:identify-null", { hooks: { identify: { returns: null } } });
  add("hooks:identify-throws", { hooks: { identify: { throws: "Failed to identify user" } } });
  add("hooks:identify-hangs", { hooks: { identify: { hangs: true } } });

  const valid51 = Object.fromEntries(Array.from({ length: 51 }, (_, i) => [`k${String(i).padStart(2, "0")}`, `v${i}`]));
  add("hooks:tags-invalid:mixed", {
    hooks: {
      eventTags: {
        returns: {
          "bad/key": "x",
          ["k".repeat(33)]: "x",
          count: 5,
          multi: "a\nb",
          long: "v".repeat(201),
          env: "prod",
        },
      },
    },
  });
  add("hooks:tags-invalid:51-valid", { hooks: { eventTags: { returns: valid51 } } });
  add("hooks:tags-invalid:none-valid", { hooks: { eventTags: { returns: { "a b!": "x", empty: 1 } } } });
  add(
    "hooks:tags-invalid:ts-bang-key",
    { hooks: { eventTags: { returns: { valid: "value", "invalid!": "value" } } } },
    [[TAGS_TEST, "Event Tags & Properties > eventTags callback > should validate tags from callback"]],
  );
  add(
    "hooks:tags-collide",
    { hooks: { eventTags: { returns: { agentcat_session_id_source: "customer-value", env: "test" } } } },
    [[TAGS_TEST, "Event Tags & Properties > eventTags callback > should overwrite a customer tag that collides with an SDK tag name"]],
  );
  add("hooks:tags-throws", { hooks: { eventTags: { throws: "Tags callback failed" } } });
  add("hooks:properties", { hooks: { eventProperties: { returns: { device: "desktop", flags: ["dark_mode"], nested: { key: "value" } } } } });
  add("hooks:properties-empty", { hooks: { eventProperties: { returns: {} } } });
  add("hooks:properties-throws", { hooks: { eventProperties: { throws: "Properties callback failed" } } });
  add("hooks:session-hook", { hook_mode: true, hooks: { resolveSessionId: { returns: "customer-abc" } } });
  add("hooks:session-hook-telemetry-only", {
    hook_mode: true,
    project_id: "",
    hooks: { resolveSessionId: { returns: "customer-abc" } },
  });
  add("hooks:session-hook-null", { hook_mode: true, hooks: { resolveSessionId: { returns: null } } });
  add("hooks:session-hook-throws", { hook_mode: true, hooks: { resolveSessionId: { throws: "no session here" } } });
  add("hooks:all-hang", {
    hook_mode: true,
    hooks: {
      resolveSessionId: { hangs: true },
      identify: { hangs: true },
      eventTags: { hangs: true },
      eventProperties: { hangs: true },
    },
  });
  add("hooks:all-settle", {
    hook_mode: true,
    hooks: {
      resolveSessionId: { returns: "customer-abc" },
      identify: { returns: user },
      eventTags: { returns: { env: "test", agentcat_session_id_source: "customer-value" } },
      eventProperties: { returns: { device: "mobile" } },
    },
  });
}

function redactionCases() {
  add("redact-event:drop", { redact_event: { returns: null } }, [
    [QUEUE_TEST, "EventQueue > event-level redaction hook (redactEvent) > should drop the event when the hook returns null"],
  ]);
  add("redact-event:throws", { redact_event: { throws: "hook exploded" } }, [
    [QUEUE_TEST, "EventQueue > event-level redaction hook (redactEvent) > should drop the event when the hook throws"],
  ]);
  add("redact-event:merge-restored", {
    redact_event: {
      merge: {
        id: "evt_fromhook",
        sessionId: sid("hook"),
        projectId: "proj_other",
        eventType: "agentcat:custom",
        timestamp: "2000-01-01T00:00:00.000Z",
        resourceName: "renamed_by_hook",
      },
    },
  });
  add(
    "redact-event:merge-parameters",
    {
      input: { sessionId: "test-session", eventType: "mcp:tools/call", resourceName: "do_thing", parameters: { text: "raw sensitive value" } },
      redact_event: { merge: { parameters: { text: "[EVENT-REDACTED]" } } },
    },
    [[QUEUE_TEST, "EventQueue > event-level redaction hook (redactEvent) > should apply the redactEvent option from tracking data to published events"]],
  );
  add("redact-event:then-strings", {
    call: { args: { context: "raw secret" } },
    redact_event: { merge: { userIntent: "raw secret (reviewed)" } },
    redact_strings: { replace: "secret" },
  });

  add("redact-strings:replace", {
    hooks: {
      identify: { returns: { userId: `user-${SECRET}`, userName: "Ada", userData: { token: SECRET } } },
      eventTags: { returns: { note: `has ${SECRET}` } },
    },
    redact_strings: { replace: SECRET },
    call: {
      name: `lookup_${SECRET}`,
      args: { query: `find ${SECRET}`, context: `why ${SECRET}` },
      response: { content: [{ type: "text", text: `found ${SECRET}` }], isError: true },
    },
  });
  add("redact-strings:throws", { redact_strings: { throw_at: 2 } });
}

function stageCases() {
  add("sanitize:image-block", {
    call: {
      response: {
        content: [
          { type: "text", text: "here is the chart" },
          { type: "image", data: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==", mimeType: "image/png" },
        ],
      },
    },
  });
  add("truncate:big-response", { call: { response: { content: [{ type: "text", text: "0123456789".repeat(12000) }] } } });
  add("truncate:non-string-context", { call: { args: { context: 42 } } });
  add("truncate:array-context", { call: { args: { context: ["plan", "act"] } } });
}

function retryCases() {
  add("retry:2-failures", { fail_attempts: 2 });
  add("retry:4-failures", { fail_attempts: 4 });
}

/** The server-form `custom` input for project argument `projectId` and event data `data`. */
const serverForm = (projectId, data) => ({
  form: "server",
  project_id: projectId,
  tracked_project_id: PROJECT,
  server_info: SERVER_INFO,
  client_info: CLIENT_INFO,
  data,
});

function customCases() {
  const string = (session, projectId, data) => ({ form: "string", session, project_id: projectId, data });
  add(
    "custom:string-form",
    { custom: string("user-session-12345", "proj_test123", { resourceName: "custom-action", parameters: { action: "test" } }) },
    [[CUSTOM_TEST, "publishCustomEvent > with session ID string > should use the session ID string verbatim as the session ID"]],
  );
  add(
    "custom:string-form-session-precedence",
    { custom: string("ignored-string", "proj", { sessionId: "ses_wins00000000000000000000000" }) },
    [[CUSTOM_TEST, "publishCustomEvent > with session ID string > should let eventData.sessionId take precedence over the string"]],
  );
  add(
    "custom:string-form-all-fields",
    {
      custom: string("user-session-12345", "proj_test123", {
        resourceName: "full-test",
        parameters: { key: "value" },
        response: { result: "success" },
        message: "Complete test",
        duration: 1500,
        isError: false,
        error: null,
      }),
    },
    [[CUSTOM_TEST, "publishCustomEvent > with session ID string > should handle all event data fields"]],
  );
  add("custom:string-form-empty", { custom: string("test-session", "proj_test", {}) }, [
    [CUSTOM_TEST, "publishCustomEvent > event structure > should handle empty event data object"],
  ]);
  add("custom:string-form-empty-session", { custom: string("", "proj_test", { resourceName: "custom-action" }) });
  add("custom:string-form-invalid-tags", {
    custom: string("user-session-12345", "proj_test123", {
      resourceName: "custom-action",
      tags: { valid: "value", "bad!key": "value", multi: "a\nb" },
    }),
  });
  add("custom:string-form-tags-none-valid", {
    custom: string("user-session-12345", "proj_test123", { resourceName: "custom-action", tags: { "bad!key": "value" } }),
  });
  add("custom:string-form-tags-properties", {
    custom: string("user-session-12345", "proj_test123", {
      resourceName: "custom-action",
      tags: { env: "production" },
      properties: { device: "mobile", feature_flags: ["beta"] },
    }),
  });

  add(
    "custom:server-form",
    {
      custom: serverForm("proj_test123", {
        sessionId: "ses_srv000000000000000000000000",
        resourceName: "custom-action",
        parameters: { action: "test" },
        message: "Testing custom event",
      }),
    },
    [[CUSTOM_TEST, "publishCustomEvent > with tracked server > should publish custom event with tracked server"]],
  );
  add("custom:server-form-no-session", { custom: serverForm("proj_test123", { resourceName: "custom-action" }) }, [
    [CUSTOM_TEST, "publishCustomEvent > with tracked server > should publish with an empty session id and warn when no sessionId is provided"],
  ]);
  add(
    "custom:server-form-error",
    { custom: serverForm("proj_test123", { isError: true, error: { message: "Test error", code: "ERR_001" } }) },
    [[CUSTOM_TEST, "publishCustomEvent > with tracked server > should handle error data correctly"]],
  );
  add("custom:server-form-duration", {
    custom: serverForm("proj_test123", { sessionId: sid("timed"), resourceName: "timed-action", duration: 1500 }),
  });
  add(
    "custom:server-form-tags-properties",
    {
      custom: serverForm("test-project", {
        resourceName: "custom-action",
        tags: { env: "production", trace_id: "xyz" },
        properties: { device: "mobile", feature_flags: ["beta"] },
      }),
    },
    [[TAGS_TEST, "Event Tags & Properties > publishCustomEvent > should include tags and properties passed directly"]],
  );
  add(
    "custom:server-form-invalid-tags",
    { custom: serverForm("test-project", { resourceName: "custom-action", tags: { valid: "value", "bad!key": "value" } }) },
    [[TAGS_TEST, "Event Tags & Properties > publishCustomEvent > should validate tags passed directly"]],
  );
  add(
    "custom:server-form-properties-empty",
    { custom: serverForm("test-project", { resourceName: "custom-action", properties: {} }) },
    [[TAGS_TEST, "Event Tags & Properties > publishCustomEvent > should normalize empty properties to omission"]],
  );
  add("custom:server-form-redact-drop", {
    custom: serverForm("proj_test123", { sessionId: sid("drop"), resourceName: "custom-action" }),
    redact_event: { returns: null },
  });
  add(
    "custom:server-form-redact-merge",
    {
      custom: serverForm("test-project", { resourceName: "custom_action", response: { secret: "raw value" } }),
      redact_event: { merge: { response: { scrubbed: true } } },
    },
    [["redaction", "redactEvent hook integration tests > should apply the hook to custom events published on a tracked server"]],
  );
  add("custom:server-form-no-string-redaction", {
    custom: serverForm("proj_test123", { sessionId: sid("plain"), resourceName: "custom-action", parameters: { key: SECRET } }),
    redact_strings: { replace: SECRET },
  });
}

callCases();
hookCases();
redactionCases();
stageCases();
retryCases();
customCases();

/** The case of `spec`: its name, ts_test and inputs (a call's parts are in its event), then TS's outputs. */
async function caseOf(core, spec) {
  const { event, requests, delays, logs } = await run(core, spec);
  const pick = (names) => Object.fromEntries(names.filter((n) => spec[n] !== undefined).map((n) => [n, spec[n]]));
  const behavior = pick(["hooks", "redact_event", "redact_strings", "fail_attempts"]);
  return { name: spec.name, ...pick(["ts_test", "hook_mode", "custom"]), event, ...behavior, requests, delays, logs };
}

export async function generate() {
  const core = await loadCore();
  const cases = [];
  for (const spec of specs) cases.push(await caseOf(core, spec));
  return { cases };
}

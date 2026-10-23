// The functions the oracle server can call: name -> function. Property tests name
// them in requests. Add an entry here when a test needs another TS function.
// A TS function that logs is wrapped in captureLogs: its result is {value, logs}.
import { loadCore, loadModule } from "./bundle.mjs";
import { captureLogs } from "./logs.mjs";
import { lock } from "./paths.mjs";

const core = await loadCore();
// Two modules the shared core bundle does not hold: they log nothing, so a bundle of their own is fine.
const exceptions = await loadModule("exceptions");
const handleInjection = await loadModule("handle-injection");

/** What the oracle.* handle wrappers write in place of a minted (random) session ID. */
const MINTED = "<minted>";

/** A copy of the JSON value `value`; a `__proto__` key stays an own key. */
const copyJson = (value) => JSON.parse(JSON.stringify(value));

/**
 * `event` with a top-level string `timestamp` turned into a `Date`, the type it has in TS when the
 * queue runs (the Rust event holds its ISO string). `dates` maps each such Date to its string.
 * Nested `timestamp` keys stay strings, as in TS.
 */
function withDate(event, dates) {
  if (event !== null && typeof event === "object" && typeof event.timestamp === "string") {
    const date = new Date(event.timestamp);
    dates.set(date, event.timestamp);
    event.timestamp = date;
  }
  return event;
}

/**
 * `event` as strict JSON: a top-level `timestamp` that is one of the Dates of `dates` back to its
 * string (any other Date as its ISO string), and the top-level `undefined` values left out, as
 * JSON.stringify leaves them out. A plain object, so a prototype that an assigned `__proto__` key
 * set is gone (RD-24).
 */
function withoutDate(event, dates) {
  const plain = (key, value) =>
    key === "timestamp" && value instanceof Date ? (dates.get(value) ?? value.toISOString()) : value;
  return Object.fromEntries(
    Object.entries(event)
      .filter(([, value]) => value !== undefined)
      .map(([key, value]) => [key, plain(key, value)]),
  );
}

/**
 * `result` ({sessionId, sessionSource, ...}) with a minted session ID replaced by MINTED, after
 * checking that it is a valid session ID; any other result as it is.
 */
function maskMinted(result) {
  if (result.sessionSource !== "minted") return result;
  if (!core.handles.isValidSessionId(result.sessionId)) {
    throw new Error(`minted session ID ${JSON.stringify(result.sessionId)} is not valid`);
  }
  return { ...result, sessionId: MINTED };
}

export const functions = {
  "JSON.stringify": (...args) => JSON.stringify(...args),
  // The oracle itself: the TS commit it runs, and fixed replies for testing a client.
  "oracle.sha": () => lock().sha,
  "oracle.echo": (value) => value,
  "oracle.throw": (message) => {
    throw new TypeError(message);
  },
  // JavaScript built-ins the TS SDK leans on (the Rust js module). String and typeof take
  // {value?}, so that an absent value is undefined, which a JSON request cannot carry.
  "String.prototype.trim": (s) => s.trim(),
  "String.prototype.slice": (s, n) => s.slice(0, n),
  "String.length": (s) => s.length,
  "oracle.String": ({ value }) => String(value),
  "oracle.typeof": ({ value }) => typeof value,
  "oracle.reformat": (text) => JSON.stringify(JSON.parse(text)),
  "ksuid.fromParts": (prefix, timeMs, payloadHex) =>
    core.KSUID.withPrefix(prefix).fromParts(timeMs, Buffer.from(payloadHex, "hex")),
  "tokenEstimate.estimateInputTokens": (...args) => core.tokenEstimate.estimateInputTokens(...args),
  "tokenEstimate.estimateOutputTokens": (...args) => core.tokenEstimate.estimateOutputTokens(...args),
  "tokenEstimate.estimateTokens": (...args) => core.tokenEstimate.estimateTokens(...args),
  "truncation.normalize": (...args) => core.truncation.normalize(...args),
  // truncateEvent as {value} or, when it throws, {error: String(error)}. TS assigns undefined to
  // each absent field it limits (userIntent, resourceName, the server and client fields); the
  // value leaves those keys out, as JSON.stringify does, since strict JSON has no undefined.
  "oracle.truncateEvent": (event) => {
    let value;
    try {
      value = core.truncation.truncateEvent(event);
    } catch (e) {
      return { error: String(e) };
    }
    return { value: Object.fromEntries(Object.entries(value).filter(([, v]) => v !== undefined)) };
  },
  "sanitization.sanitizeEvent": (event) => core.sanitization.sanitizeEvent(event),
  // redactEvent with a recording redactor: s -> "[R:" + s + "]", throwing Error("redactor failed")
  // on call number `throwAt` (1-based; absent: never). Gives {value, calls} or, when redactEvent
  // throws, {error: String(error), calls}; `calls` lists every string the redactor got, the one
  // it threw on included. A top-level string `timestamp` is a Date for TS (see withDate).
  "oracle.redactEvent": async ({ event, throwAt }) => {
    const calls = [];
    const redactor = async (text) => {
      calls.push(text);
      if (calls.length === throwAt) throw new Error("redactor failed");
      return `[R:${text}]`;
    };
    const dates = new Map();
    let value;
    try {
      value = await core.redaction.redactEvent(withDate(copyJson(event), dates), redactor);
    } catch (e) {
      return { error: String(e), calls };
    }
    return { value: withoutDate(value, dates), calls };
  },
  // applyEventRedaction on a copy of `original` with a hook that returns a copy of `replacement`
  // (null or absent: the hook returns null or undefined, and the event is dropped). Gives the
  // event the hook left, or null for a drop. A top-level string `timestamp` of either is a Date
  // for TS (see withDate). TS gives the event an own `redactionFn` (the original's: undefined
  // here); the reply leaves undefined values out, as JSON.stringify does.
  "oracle.applyEventRedaction": async ({ original, replacement }) => {
    const dates = new Map();
    const event = withDate(copyJson(original), dates);
    const result = replacement === undefined ? undefined : withDate(copyJson(replacement), dates);
    const kept = await core.redaction.applyEventRedaction(event, () => result);
    return kept ? withoutDate(event, dates) : null;
  },
  "validation.validateTags": (tags) => captureLogs(core, () => core.validation.validateTags(tags)),
  // Session handles. A resolution is TS's HandleResolution, in its camelCase form.
  "handles.deriveSessionId": (id, projectId) => core.handles.deriveSessionId(id, projectId),
  "handles.isValidSessionId": (value) => core.handles.isValidSessionId(value),
  "handles.extractHandle": (args, name) => core.handles.extractHandle(args, name),
  "handles.buildMintBackText": (res) => core.handles.buildMintBackText(res),
  "handles.buildStructuredMintBack": (res) => core.handles.buildStructuredMintBack(res),
  "handles.buildHandleTags": (res, protocolVersion) => core.handles.buildHandleTags(res, protocolVersion),
  // resolveHandles for a tool call with `args` (absent: undefined). Hook mode is a configured
  // resolveSessionId; resolveHandles never calls it (the caller does, then sessionFromHookValue).
  "oracle.resolveHandles": ({ args, hookMode, agentTracking, sessionParamIsOurs }) =>
    maskMinted(
      core.handles.resolveHandles(
        { enableAgentTracking: agentTracking, ...(hookMode && { resolveSessionId: () => null }) },
        "proj_1",
        { params: { name: "tool", arguments: args } },
        undefined,
        sessionParamIsOurs,
      ),
    ),
  // sessionFromHookValue for a hook value (absent: null) and a project ID (absent: undefined).
  "oracle.sessionFromHookValue": ({ value, projectId }) =>
    maskMinted(core.handles.sessionFromHookValue(value ?? null, projectId)),
  // appendMintBack and mirrorStructuredMintBack (src/modules/handles.ts): the result with the block
  // prepended, or the mint mirrored into structuredContent; the input as it was when nothing applies.
  "handles.appendMintBack": (result, text) => core.handles.appendMintBack(result, text),
  "handles.mirrorStructuredMintBack": (result, mint) => core.handles.mirrorStructuredMintBack(result, mint),
  // captureException (src/modules/exceptions.ts) on one JSON value: a CallToolResult-like value
  // ({isError, content: [...]}) gives {message, platform} (its `type` is undefined, so absent).
  "exceptions.captureException": (error) => exceptions.captureException(error),
  // The arguments cloneRequestWithStrippedArguments (src/modules/handle-injection.ts) hands the
  // customer's handler for a call of tool `name`. `registry` is [[name, [params]]] (the injected
  // parameters per tool) or null for no registry.
  "oracle.strip": ({ name, arguments: args, registry }) =>
    handleInjection.cloneRequestWithStrippedArguments(
      { method: "tools/call", params: { name, arguments: args } },
      registry === null ? undefined : new Map(registry.map(([tool, params]) => [tool, new Set(params)])),
    ).params.arguments,
  // handleReportMissing (src/modules/tools.ts) for a call whose `context` argument is `context`
  // (absent: undefined): {value: the result, logs}.
  "oracle.reportMissing": ({ context }) => captureLogs(core, () => core.tools.handleReportMissing({ context })),
  // buildInjectedList (src/engine/listWrap.ts) on each listing page in turn, with one tracking-data
  // object for all pages, as one tracked server holds it: its reported-conflict and declared-session
  // sets and its get_more_tools shadow warning carry over from page to page. `options` are the TS
  // option names; hook mode is a resolveSessionId hook (never called here). Gives one entry per page:
  // {tools, registry: [[name, [params]]], outputRegistry: [names], declaredSessionParams: [names],
  // logs}, the registry and sets in their insertion order and the declared set as it stands after the
  // page. Each page is a copy, so the input stays as it was.
  "oracle.buildInjectedList": async ({ options, hookMode, pages }) => {
    const data = {
      projectId: "proj_1",
      options: { ...options, ...(hookMode && { resolveSessionId: () => null }) },
    };
    const out = [];
    for (const page of pages) {
      const { value, logs } = await captureLogs(core, () => core.listWrap.buildInjectedList(data, copyJson(page)));
      out.push({
        tools: value.tools,
        registry: [...value.registry].map(([name, params]) => [name, [...params]]),
        outputRegistry: [...value.outputRegistry],
        declaredSessionParams: [...core.registry.getDeclaredSessionParams(data)],
        logs,
      });
    }
    return out;
  },
};

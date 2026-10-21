// The functions the oracle server can call: name -> function. Property tests name
// them in requests. Add an entry here when a test needs another TS function.
// A TS function that logs is wrapped in captureLogs: its result is {value, logs}.
import { loadCore } from "./bundle.mjs";
import { captureLogs } from "./logs.mjs";
import { lock } from "./paths.mjs";

const core = await loadCore();

/** What the oracle.* handle wrappers write in place of a minted (random) session ID. */
const MINTED = "<minted>";

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
};

// The functions the oracle server can call: name -> function. Property tests name
// them in requests. Add an entry here when a test needs another TS function.
// A TS function that logs is wrapped in captureLogs: its result is {value, logs}.
import { loadCore } from "./bundle.mjs";
import { captureLogs } from "./logs.mjs";
import { lock } from "./paths.mjs";

const core = await loadCore();

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
  "tokenEstimate.estimateInputTokens": (...args) => core.tokenEstimate.estimateInputTokens(...args),
  "validation.validateTags": (tags) => captureLogs(core, () => core.validation.validateTags(tags)),
};

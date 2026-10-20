// The functions the oracle server can call: name -> function. Property tests name
// them in requests. Add an entry here when a test needs another TS function.
import { loadModule } from "./bundle.mjs";

const tokenEstimate = await loadModule("tokenEstimate");

export const functions = {
  "JSON.stringify": (...args) => JSON.stringify(...args),
  "tokenEstimate.estimateInputTokens": (...args) => tokenEstimate.estimateInputTokens(...args),
};

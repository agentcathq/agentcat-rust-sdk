// Vectors for the pure parts of a tool call in the TS SDK: stripping the injected arguments
// (src/modules/handle-injection.ts), the mint-back text and its mirror (src/modules/handles.ts),
// the error of an error result (src/modules/exceptions.ts) and the get_more_tools call
// (src/modules/tools.ts).
//
// Case name prefixes, each with its fields:
//   strip:<label>          input: {name, arguments, registry};  output: the arguments the customer's
//                          handler receives. `registry` is [[tool, [injected params]]], or null for no
//                          registry (oracle.strip, functions.mjs)
//   mint-back:<label>      input: {result, text};  output: appendMintBack(result, text). `text` is
//                          buildMintBackText(resolution) of the source in the label; where TS has no
//                          text it does not call appendMintBack (callWrap.ts), `text` is null and the
//                          output is the result as it was
//   structured:<label>     input: {structuredContent?, mint};  output: the result
//                          {content: [], structuredContent?} that mirrorStructuredMintBack returns;
//                          output_text: JSON.stringify of its structuredContent, which has the wire
//                          key order (the other comparisons ignore key order), absent with it
//   result-error:<label>   input: a CallToolResult with isError;  output: captureException(input)
//                          (its absent `type` is left out)
//   report-missing:<label> input: {context?};  output: {value, logs} of oracle.reportMissing
//   A case may carry "deviation": the ruled-divergence ID of a Rust value that is not TS's.
import { functions } from "../../functions.mjs";
import { tsKey } from "../support.mjs";

const cases = [];

/** A JSON copy of `value`: no undefined, no prototype tricks. */
const plain = (value) => (value === undefined ? undefined : JSON.parse(JSON.stringify(value)));

/** Adds a case; `fields` follow the output (ts_test, deviation, output_text). */
function add(name, input, output, fields = {}) {
  cases.push({ name, input: plain(input), output: plain(output), ...fields });
}

const T = "ses_2xF9kQm3rTvB8nLpYw7ZcHd4Ke1";
const text = (t) => ({ type: "text", text: t });

// ---------------------------------------------------------------------------------------- strip

const STRIP = "stripInjectedArguments > ";
const registry = [
  ["add_todo", ["session_id", "agent_id", "context"]],
  ["deploy", ["agent_id", "context"]], // tool owns session_id
];

async function strip(name, tool, args, reg, tsTest) {
  const input = { name: tool, arguments: args, registry: reg };
  add(`strip:${name}`, input, await functions["oracle.strip"](input), tsTest && { ts_test: [tsKey("handle-injection", `${STRIP}${tsTest}`)] });
}

async function stripCases() {
  await strip("ts:exact", "add_todo", { text: "x", session_id: "ses_1", agent_id: "agt_1", context: "why" }, registry, "strips exactly what was injected");
  await strip("ts:owns-session", "deploy", { session_id: "prod-42", agent_id: "agt_1", context: "why" }, registry, "preserves the customer's own session_id param");
  const fallback = "falls back to stripping all three when no registry entry exists";
  await strip("ts:fallback-absent-tool", "unlisted_tool", { text: "x", session_id: "a", agent_id: "b", context: "c" }, registry, fallback);
  await strip("ts:fallback-no-registry", "any", { text: "x", session_id: "a" }, null, fallback);
  await strip("ts:get-more-tools", "get_more_tools", { context: "need a tool", session_id: "a", agent_id: "b" }, null, "fallback never strips get_more_tools' own context");

  // With a registry entry.
  await strip("entry:none-present", "add_todo", { text: "x" }, registry);
  await strip("entry:only-session", "add_todo", { session_id: "s" }, registry);
  await strip("entry:only-agent", "add_todo", { agent_id: "a", text: "x" }, registry);
  await strip("entry:only-context", "add_todo", { context: "c", text: "x" }, registry);
  await strip("entry:empty-arguments", "add_todo", {}, registry);
  await strip("entry:empty-list", "plain", { session_id: "mine", agent_id: "mine", context: "mine" }, [["plain", []]]);
  await strip("entry:only-context-recorded", "ctx", { session_id: "mine", agent_id: "mine", context: "c" }, [["ctx", ["context"]]]);
  await strip("entry:agent-only-recorded", "agent", { session_id: "mine", agent_id: "a", context: "mine" }, [["agent", ["agent_id"]]]);
  await strip("entry:get-more-tools-recorded", "get_more_tools", { session_id: "s", agent_id: "a", context: "c" }, [["get_more_tools", ["session_id", "agent_id"]]]);
  await strip("entry:get-more-tools-empty", "get_more_tools", { session_id: "s", agent_id: "a", context: "c" }, [["get_more_tools", []]]);
  await strip("entry:key-order", "add_todo", { z: 1, session_id: "s", a: 2, context: "c", m: 3 }, registry);
  await strip("entry:nested-values-kept", "add_todo", { session_id: "s", payload: { session_id: "inner", context: [1] } }, registry);
  await strip("entry:null-values", "add_todo", { session_id: null, agent_id: null, context: null, k: null }, registry);
  // Customer keys named like ours.
  await strip("entry:lookalike-keys", "add_todo", { Session_Id: 1, "session_id ": 2, sessionId: 3, "agent-id": 4, context2: 5, contexts: 6, " context": 7 }, registry);
  await strip("entry:index-keys", "add_todo", { b: 1, 10: 2, 2: 3, session_id: "s" }, registry);
  await strip("entry:other-tool-in-registry", "other", { session_id: "mine", context: "mine" }, registry);

  // Without an entry.
  await strip("fallback:none-present", "unlisted", { text: "x" }, registry);
  await strip("fallback:empty-arguments", "unlisted", {}, null);
  await strip("fallback:each-alone-session", "t", { session_id: "s" }, null);
  await strip("fallback:each-alone-agent", "t", { agent_id: "a" }, null);
  await strip("fallback:each-alone-context", "t", { context: "c" }, null);
  await strip("fallback:get-more-tools-with-registry", "get_more_tools", { context: "c", session_id: "s", agent_id: "a", extra: 1 }, registry);
  await strip("fallback:get-more-tools-only-context", "get_more_tools", { context: "c" }, null);
  await strip("fallback:key-order", "t", { z: 1, context: "c", a: 2, agent_id: "a", session_id: "s" }, null);
  await strip("fallback:lookalike-keys", "t", { Context: 1, "context ": 2, sessionid: 3, agent_ids: 4 }, null);
  await strip("fallback:empty-name", "", { session_id: "s", text: "x" }, registry);
  await strip("fallback:name-like-prototype", "toString", { session_id: "s", text: "x" }, null);
}

// ------------------------------------------------------------------------------------ mint-back

const APPEND = "appendMintBack > ";

const SOURCES = {
  minted: { sessionId: T, sessionSource: "minted", hookMode: false },
  invalid: { sessionId: "", sessionSource: "invalid", hookMode: false },
  supplied: { sessionId: T, sessionSource: "supplied", hookMode: false },
  foreign: { sessionId: "", sessionSource: "foreign", hookMode: false },
  "hook-mode": { sessionId: "", sessionSource: "supplied", hookMode: true },
};

/** The result of appendMintBack(result, text) as callWrap.ts calls it: not at all without text. */
function mintBack(result, mintText) {
  return mintText ? functions["handles.appendMintBack"](plain(result), mintText) : result;
}

function mintBackCase(name, result, mintText, tsTest) {
  add(`mint-back:${name}`, { result, text: mintText }, mintBack(result, mintText), tsTest && { ts_test: tsTest });
}

function mintBackCases() {
  const k = (title) => [tsKey("handles", `${APPEND}${title}`)];
  const minted = functions["handles.buildMintBackText"](SOURCES.minted);
  const invalid = functions["handles.buildMintBackText"](SOURCES.invalid);
  mintBackCase("ts:prepends-first", { content: [text("hi")] }, "BLOCK", k("prepends a text block as the first content element"));
  mintBackCase("ts:is-error", { isError: true, content: [text("boom")] }, "BLOCK", k("prepends on isError results too"));
  mintBackCase("text:plain-block", { content: [text("hi")] }, "BLOCK");

  const results = {
    single: { content: [text("hello")] },
    "several-blocks": {
      content: [text("one"), { type: "image", data: "AAAA", mimeType: "image/png" }, text("three")],
    },
    "is-error": { isError: true, content: [text("it broke")] },
    "is-error-false": { isError: false, content: [text("fine")] },
    empty: { content: [] },
    structured: { content: [text("42 things")], structuredContent: { count: 42 } },
    "structured-is-error": { isError: true, content: [text("bad")], structuredContent: { msg: "boom" } },
    resources: {
      content: [
        { type: "resource", resource: { uri: "file:///a.txt", text: "body" } },
        { type: "resource_link", uri: "file:///b.txt", name: "b" },
      ],
    },
    meta: { content: [text("m")], _meta: { k: "v" } },
  };
  for (const [source, resolution] of Object.entries(SOURCES)) {
    const mintText = functions["handles.buildMintBackText"](resolution);
    for (const [label, result] of Object.entries(results)) {
      if (mintText === null && label !== "single" && label !== "is-error") continue;
      mintBackCase(`${source}:${label}`, result, mintText);
    }
  }
  // The text itself: both mint-back texts have a newline, a dash and a session ID.
  mintBackCase("text:minted-text", { content: [text("x")] }, minted);
  mintBackCase("text:invalid-text", { content: [text("x")] }, invalid);
  mintBackCase("text:multiline-and-unicode", { content: [text("x")] }, "line 1\nline 2 \u{1F600} — \u0000", null);
}

// ------------------------------------------------------------------------------------ structured

const MIRROR = "mirrorStructuredMintBack > ";
const mint = (...keys) => {
  const all = { session_id: T, agent_id: "opus|cc|k3n9x", status: "issued" };
  return Object.fromEntries(keys.map((key) => [key, all[key]]));
};

function structuredCase(name, structuredContent, mintValue, tsTest, fields = {}) {
  const result = { content: [] };
  if (structuredContent !== undefined) result.structuredContent = plain(structuredContent);
  const out = functions["handles.mirrorStructuredMintBack"](plain(result), plain(mintValue));
  const input = { mint: mintValue };
  if (structuredContent !== undefined) input.structuredContent = structuredContent;
  add(`structured:${name}`, input, out, {
    ...(tsTest && { ts_test: [tsKey("handles", `${MIRROR}${tsTest}`)] }),
    ...(out.structuredContent !== undefined && { output_text: JSON.stringify(out.structuredContent) }),
    ...fields,
  });
}

function structuredCases() {
  const m = mint("session_id", "status");
  structuredCase("ts:first-key", { a: 1 }, m, "adds the field as the FIRST key of structuredContent without mutating");
  const untouched = "returns the input untouched when structuredContent is absent or not a plain object";
  structuredCase("ts:absent", undefined, m, untouched);
  structuredCase("ts:array", [1, 2], m, untouched);
  structuredCase("ts:string", "nope", m, untouched);
  structuredCase("ts:own-mcp-session", { mcp_session: "customer-owned" }, m, "never clobbers an existing key (customer data wins)");
  structuredCase("ts:is-error", { msg: "boom" }, m, "applies to isError results that carry structuredContent");

  // Plain objects, and where the mirror goes among index keys.
  structuredCase("object:empty", {}, m);
  structuredCase("object:one-key", { count: 42 }, m);
  structuredCase("object:index-keys-first", { 0: "a", 10: "b", x: "c" }, m);
  structuredCase("object:index-keys-mixed", { x: "c", 10: "b", 0: "a", y: "d" }, m);
  structuredCase("object:index-keys-descending", { 10: 1, 9: 2, 2: 3, 1: 4 }, m);
  structuredCase("object:index-keys-only", { 5: "a", 3: "b" }, m);
  structuredCase("object:index-limit", { 4294967294: "a", 4294967295: "b", 4294967296: "c", z: 1 }, m);
  structuredCase("object:not-index-spellings", { "01": 1, "-1": 2, "1.5": 3, " 1": 4, 1: 5 }, m);
  structuredCase("object:nested", { a: { mcp_session: "inner" }, b: [{ 0: 1 }] }, m);
  structuredCase("object:own-mcp-session-with-others", { 0: 1, mcp_session: { x: 1 }, z: 2 }, m);
  structuredCase("object:own-mcp-session-null", { mcp_session: null }, m);
  structuredCase("object:similar-key", { mcp_sessions: 1, "mcp_session ": 2, Mcp_Session: 3 }, m);
  structuredCase("object:unicode-keys", { "\u{1F600}": 1, "é": 2 }, m);
  structuredCase("object:null-values", { a: null, 3: null }, m);

  // Values that are not plain objects.
  structuredCase("other:array-empty", [], m);
  structuredCase("other:array-of-objects", [{ a: 1 }], m);
  structuredCase("other:string", "text", m);
  structuredCase("other:string-empty", "", m);
  structuredCase("other:null", null, m);
  structuredCase("other:number", 7, m);
  structuredCase("other:zero", 0, m);
  structuredCase("other:true", true, m);
  structuredCase("other:false", false, m);

  // Mints.
  const body = { 2: "b", a: 1, 0: "z" };
  structuredCase("mint:one-key-session", body, mint("session_id"));
  structuredCase("mint:one-key-agent", body, mint("agent_id"));
  structuredCase("mint:one-key-status", body, mint("status"));
  structuredCase("mint:two-keys", body, mint("session_id", "status"));
  structuredCase("mint:two-keys-agent", body, mint("agent_id", "status"));
  structuredCase("mint:three-keys", body, mint("session_id", "agent_id", "status"));
  structuredCase("mint:three-keys-array", [1], mint("session_id", "agent_id", "status"));
  structuredCase("mint:unrecognized", { x: 1 }, { agent_id: "a|b|c", status: "unrecognized" });
}

// -------------------------------------------------------------------------------- result-error

function resultErrorCase(name, result, tsTest) {
  add(`result-error:${name}`, result, functions["exceptions.captureException"](plain(result)), tsTest && { ts_test: tsTest });
}

function resultErrorCases() {
  const err = (content, more = {}) => ({ isError: true, content, ...more });
  resultErrorCase("text-one", err([text("tool failed")]));
  resultErrorCase("text-two", err([text("first"), text("second")]));
  resultErrorCase("text-three-joined-with-space", err([text("a"), text("b"), text("c")]));
  resultErrorCase("text-trimmed", err([text("  padded \n")]));
  resultErrorCase("text-inner-whitespace-kept", err([text(" a\n"), text("\tb ")]));
  resultErrorCase("text-empty-first", err([text(""), text("later")]));
  resultErrorCase("text-empty-last", err([text("early"), text("")]));
  resultErrorCase("text-whitespace-only", err([text("   \n\t ")]));
  resultErrorCase("text-empty", err([text("")]));
  resultErrorCase("text-unicode", err([text("fail \u{1F600} — é")]));
  resultErrorCase("text-js-whitespace", err([text(" ﻿ x ")]));
  resultErrorCase("mixed-image-between", err([text("a"), { type: "image", data: "AAAA", mimeType: "image/png" }, text("b")]));
  resultErrorCase("mixed-resource-first", err([{ type: "resource", resource: { uri: "file:///a", text: "ignored" } }, text("shown")]));
  resultErrorCase("mixed-link-and-audio", err([{ type: "resource_link", uri: "file:///b", name: "b" }, { type: "audio", data: "AAAA", mimeType: "audio/wav" }, text("z")]));
  resultErrorCase("no-text-image", err([{ type: "image", data: "AAAA", mimeType: "image/png" }]));
  resultErrorCase("no-text-resource", err([{ type: "resource", resource: { uri: "file:///a", text: "ignored" } }]));
  resultErrorCase("empty-content", err([]));
  resultErrorCase("structured-ignored", err([text("with data")], { structuredContent: { code: 7 } }));
  resultErrorCase("meta-ignored", err([text("with meta")], { _meta: { k: 1 } }));
  resultErrorCase("is-error-false", { isError: false, content: [text("not an error")] });
}

// ------------------------------------------------------------------------------ report-missing

async function reportMissingCase(name, input, fields = {}) {
  add(`report-missing:${name}`, input, await functions["oracle.reportMissing"](input), fields);
}

async function reportMissingCases() {
  const k = (title) => ({ ts_test: [tsKey("report-missing", `Report Missing Tool > 2. Tool Execution Tests > ${title}`)] });
  // The first two tests also assert the published event (plan 4); only the third is the answer alone.
  await reportMissingCase("ts:only-context", { context: "Need a database query tool for SQL operations" });
  await reportMissingCase("ts:description-and-context", { context: "User wants to fetch data from external REST APIs" });
  await reportMissingCase("ts:missing-description", { context: "Some context without description" }, k("should handle missing description parameter gracefully"));

  await reportMissingCase("string", { context: "abc" });
  await reportMissingCase("string-empty", { context: "" });
  await reportMissingCase("string-emoji", { context: "\u{1F600}x" });
  await reportMissingCase("string-astral-only", { context: "\u{1F600}\u{1D11E}" });
  await reportMissingCase("string-combining", { context: "éé" });
  await reportMissingCase("string-long", { context: "x".repeat(300) });
  await reportMissingCase("array", { context: [1, 2, 3] });
  await reportMissingCase("array-empty", { context: [] });
  await reportMissingCase("array-of-strings", { context: ["\u{1F600}", "ab"] });
  await reportMissingCase("number", { context: 5 });
  await reportMissingCase("true", { context: true });
  await reportMissingCase("false", { context: false });
  await reportMissingCase("null", { context: null });
  await reportMissingCase("absent", {});
  await reportMissingCase("object-empty", { context: {} });
  await reportMissingCase("object-keys", { context: { a: 1, b: 2 } });
  // R23: TS reads `.length` off any value, an object's own `length` key too; Rust counts only
  // strings and arrays (RD-26 lists the case).
  await reportMissingCase("object-length-key", { context: { length: 7 } }, { deviation: "RD-26" });
  await reportMissingCase("object-length-string", { context: { length: "ab" } }, { deviation: "RD-26" });
  await reportMissingCase("object-length-null", { context: { length: null } });
  await reportMissingCase("object-length-zero", { context: { length: 0 } });
}

export async function generate() {
  cases.length = 0;
  await stripCases();
  mintBackCases();
  structuredCases();
  resultErrorCases();
  await reportMissingCases();
  return { cases };
}

// Vectors for the session handles of the TS SDK (src/modules/handles.ts): the derived session ID,
// the validity check, reading a handle off tool arguments, the resolution of one call, the
// mint-back text and object, the handle tags, and the session of a hook value.
//
// Case name prefixes, each with its fields:
//   derive:<label>      input: {id, projectId?};  output: deriveSessionId(id, projectId)
//   valid:<label>       input: a string;          output: isValidSessionId(input);
//                                                 pattern: new RegExp(SESSION_ID_PARAM_PATTERN).test(input)
//   extract:<label>     input: {args?, name};     output: extractHandle(args, name), null for undefined
//   resolve:<label>     input: {args?, hookMode, agentTracking, sessionParamIsOurs};
//                       output: oracle.resolveHandles(input), the resolution
//   mint-back:<label>   input: a resolution;      output: buildMintBackText(input)
//   structured:<label>  input: a resolution;      output: buildStructuredMintBack(input)
//   tags:<label>        input: {res, protocolVersion?};  output: buildHandleTags(res, protocolVersion)
//   hook-value:<label>  input: {value?, projectId?};     output: oracle.sessionFromHookValue(input),
//                                                        {sessionId, sessionSource}
// An absent field is undefined. A resolution is TS's HandleResolution: {sessionId, sessionSource,
// hookMode, agentId?, agentSource?}. A minted session ID is random, so the oracle.* functions
// (functions.mjs) check it with isValidSessionId and write "<minted>" in its place.
import { loadCore } from "oracle/bundle";

import { functions } from "../../functions.mjs";
import { pick, seeded, text, tsKey } from "../support.mjs";

const k = (title) => tsKey("handles", title);
const PRIMITIVES = "handle primitives > ";
const PROMPTED = "resolveHandles — prompted mode > ";
const HOOK = "resolveHandles — hook mode > ";
const MINT_TEXT = "buildMintBackText > ";
const STRUCTURED = "buildStructuredMintBack > ";
const TAGS = "buildHandleTags > ";
const VALID = "isValidSessionId > ";
const HOOK_VALUE = "sessionFromHookValue > ";
const INVALID_FOREIGN = "invalid and foreign mint-back > ";
// The value-contract test of constants.test.ts runs SESSION_ID_PARAM_PATTERN against five inputs.
const VALUE_CONTRACT = tsKey("constants", "v4 copy pins (byte-exact) > pins the session_id value-contract constants");

// The fixtures of handles.test.ts.
const T = "ses_2xF9kQm3rTvB8nLpYw7ZcHd4Ke1";
const A = "opus-4.80-1m|claude-code|k3n9x";
const A_SHORT = "opus|cc|k3n9x";
// The session ID of the mint-back cases that came from constants.json.
const SESSION_ID = "ses_0123456789abcdefghijklmnopq";
const BODY = T.slice(4);

/** The sid() helper of the TS tests (src/tests/test-utils.ts): a valid session ID that reads as its label. */
function sid(label) {
  return `ses_${(label.replace(/[^0-9A-Za-z]/g, "") + "0".repeat(27)).slice(0, 27)}`;
}

/** `fields` with each key whose value is undefined left out. */
function defined(fields) {
  return Object.fromEntries(Object.entries(fields).filter(([, v]) => v !== undefined));
}

/** A case; `ts_test` only when there are keys. */
function caseOf(name, fields, tsTest) {
  return { name, ...fields, ...(tsTest && tsTest.length > 0 && { ts_test: tsTest }) };
}

// Characters of the seeded derive IDs: ASCII, ":", whitespace, BMP letters and astral characters.
const DERIVE_ALPHABET = "abcXYZ019-_:| \t\n\u0085 ﻿é世ſ😀𝄞";

function deriveCases(core) {
  const cases = [];
  const add = (name, id, projectId, tsTest) => {
    const input = defined({ id, projectId });
    cases.push(caseOf(`derive:${name}`, { input, output: core.handles.deriveSessionId(id, projectId) }, tsTest));
  };
  // Appendix C of docs/research/01-ts-sdk-behavior.md, with the tests that pin the same calls.
  add("customer-abc+proj_1", "customer-abc", "proj_1", [
    k(`${PRIMITIVES}derives deterministic session ids from id+project`),
    k(`${PRIMITIVES}different project yields a different derived id`),
    k(`${PRIMITIVES}derives without a project id`),
    k(`${PRIMITIVES}pins derived session ids to their golden values`),
  ]);
  add("customer-abc", "customer-abc", undefined, [
    k(`${PRIMITIVES}derives without a project id`),
    k(`${PRIMITIVES}pins derived session ids to their golden values`),
  ]);
  add("space-x-space+p", " x ", "p", [k(`${PRIMITIVES}does not trim its input`)]);
  add("x+p", "x", "p", [k(`${PRIMITIVES}does not trim its input`)]);
  add("task-123+proj_abc", "task-123", "proj_abc");
  add("task-123", "task-123", undefined);
  add("task-123+empty-project", "task-123", "");
  // Other calls of the TS tests.
  add("customer-abc+proj_2", "customer-abc", "proj_2", [k(`${PRIMITIVES}different project yields a different derived id`)]);
  add("customer-42+proj_1", "customer-42", "proj_1", [k(`${HOOK_VALUE}derives deterministically from the hook value + project`)]);
  add("corr-1+proj_1", "corr-1", "proj_1", [k(`${HOOK_VALUE}trims the value before deriving`)]);
  add("anything+proj_1", "anything", "proj_1");
  // The hashed text is `${id}:${projectId}`, so a colon in the ID can give the same text.
  add("a:b", "a:b", undefined);
  add("a+b", "a", "b");
  // Whitespace (hashed as it is), Unicode and astral characters, and project IDs.
  add("empty", "", undefined);
  add("empty+p", "", "p");
  add("tab+p", "\t", "p");
  add("bom-x", "﻿x", undefined);
  add("x-nel+proj", "x\u0085", "proj_2bCg3fv09poapZdDQUDJsT2UCvG");
  add("unicode", "é世ſ", "p");
  add("astral", "😀𝄞", undefined);
  add("astral+astral-project", "x😀", "proj_😀");
  add("customer-abc+unicode-project", "customer-abc", "projé世");
  const rand = seeded(4);
  for (let i = 0; i < 40; i++) {
    const id = text(rand, DERIVE_ALPHABET, 0, 40);
    const projectId = pick(rand, [undefined, "", "proj_1", text(rand, DERIVE_ALPHABET, 1, 10)]);
    add(`seeded-${i}`, id, projectId);
  }
  return cases;
}

function validCases(core) {
  const pattern = new RegExp(core.constants.SESSION_ID_PARAM_PATTERN);
  const cases = [];
  const add = (name, value, tsTest) => {
    const fields = { input: value, output: core.handles.isValidSessionId(value), pattern: pattern.test(value) };
    cases.push(caseOf(`valid:${name}`, fields, tsTest));
  };
  add("body-26", `ses_${BODY.slice(0, 26)}`);
  add("body-27", T);
  add("body-28", `${T}Z`);
  add("body-27-zeros", `ses_${"0".repeat(27)}`);
  add("body-27-z", `ses_${"z".repeat(27)}`);
  add("derived", core.handles.deriveSessionId("anything", "proj_1"));
  add("non-base62-dash", `ses_${BODY.slice(0, 26)}-`);
  add("non-base62-underscore", `ses__${BODY.slice(0, 26)}`);
  add("arabic-indic-digit", `ses_${BODY.slice(0, 26)}٣`);
  add("fullwidth-digit", `ses_${BODY.slice(0, 26)}３`);
  add("kelvin-sign", `ses_${BODY.slice(0, 26)}K`);
  add("long-s", `ses_${BODY.slice(0, 26)}ſ`);
  // 25 characters and an astral one: 27 UTF-16 units, but not 27 base62 characters.
  add("astral-body-27-units", `ses_${BODY.slice(0, 25)}😀`);
  add("trailing-newline", `${T}\n`);
  add("leading-newline", `\n${T}`);
  add("trailing-cr", `${T}\r`);
  add("trailing-space", `${T} `);
  add("leading-space", ` ${T}`);
  add("trailing-nel", `${T}\u0085`);
  add("trailing-line-separator", `${T} `);
  add("trailing-bom", `${T}﻿`);
  add("inner-tab", `ses_${BODY.slice(0, 13)}\t${BODY.slice(13, 26)}`);
  add("prefix-upper", `SES_${BODY}`);
  add("prefix-capital", `Ses_${BODY}`);
  add("prefix-dash", `ses-${BODY}`);
  add("prefix-no-underscore", `ses${BODY}`);
  add("prefix-evt", `evt_${BODY}`);
  add("prefix-sess", `sess_${BODY}`);
  add("start-newline", "start\n");
  add("start-spaced", " start ");
  // The tests of handles.test.ts.
  add("ts:sid-parent", sid("parent"), [k(`${VALID}accepts the test helper's IDs`)]);
  add("ts:wrong-prefix", "task_2xF9kQm3rTvB8nLpYw7ZcHd4Ke", [k(`${VALID}rejects wrong prefix`)]);
  add("ts:no-prefix", "2xF9kQm3rTvB8nLpYw7ZcHd4Ke1", [k(`${VALID}rejects no prefix`)]);
  add("ts:too-short", "ses_abc", [k(`${VALID}rejects too short`)]);
  add("ts:too-long", `ses_${"a".repeat(28)}`, [k(`${VALID}rejects too long`)]);
  add("ts:empty", "", [k(`${VALID}rejects empty`)]);
  add("ts:prefix-only", "ses_", [k(`${VALID}rejects prefix only`)]);
  add("ts:customer-value", "my-app-session-42", [k(`${VALID}rejects customer value`)]);
  add("ts:non-base62-body", `ses_${"-".repeat(27)}`, [k(`${VALID}rejects non-base62 body`)]);
  add("ts:inner-whitespace", `ses_ ${"a".repeat(26)}`, [k(`${VALID}rejects inner whitespace`)]);
  add("ses_short", "ses_short");
  // The inputs the value-contract test of constants.test.ts runs the pattern against.
  add("contract:start", "start", [VALUE_CONTRACT]);
  add("contract:ses-27-a", `ses_${"a".repeat(27)}`, [VALUE_CONTRACT]);
  add("contract:ses-25-a", `ses_${"a".repeat(25)}`, [VALUE_CONTRACT]);
  add("contract:restart", "restart", [VALUE_CONTRACT]);
  add("contract:upper-start", "START", [VALUE_CONTRACT]);
  return cases;
}

function extractCases(core) {
  const cases = [];
  const add = (label, args, name, tsTest) => {
    const output = core.handles.extractHandle(args, name) ?? null;
    cases.push(caseOf(`extract:${label}`, { input: defined({ args, name }), output }, tsTest));
  };
  const trimmed = k(`${PRIMITIVES}extractHandle returns trimmed non-empty strings only`);
  add("ts:spaced-ses_x", { session_id: " ses_x " }, "session_id", [trimmed]);
  add("ts:empty", { session_id: "" }, "session_id", [trimmed]);
  add("ts:spaces", { session_id: "   " }, "session_id", [trimmed]);
  add("ts:number", { session_id: 42 }, "session_id", [trimmed]);
  add("ts:args-undefined", undefined, "session_id", [trimmed]);
  add("ts:args-null", null, "agent_id", [trimmed]);
  add("ts:args-string", "not-an-object", "session_id", [trimmed]);
  add("ts:any-string", { session_id: "my-own-correlation-id" }, "session_id", [
    k(`${PRIMITIVES}extractHandle returns any non-empty string; validation happens in resolveHandles`),
  ]);
  add("string", { session_id: "x" }, "session_id");
  add("padded", { session_id: "  x  " }, "session_id");
  add("bom", { session_id: "﻿x" }, "session_id");
  add("nel", { session_id: "x\u0085" }, "session_id");
  add("mongolian-vowel-separator", { session_id: "᠎x" }, "session_id");
  add("line-separator-ideographic-space", { session_id: " x　" }, "session_id");
  add("inner-space", { session_id: " a b " }, "session_id");
  add("null", { session_id: null }, "session_id");
  add("true", { session_id: true }, "session_id");
  add("array", { session_id: ["x"] }, "session_id");
  add("object", { session_id: { a: "x" } }, "session_id");
  add("missing-key", { other: "x" }, "session_id");
  add("agent-padded", { agent_id: "  a  ", session_id: "s" }, "agent_id");
  add("agent-from-session-only", { session_id: "s" }, "agent_id");
  add("args-empty", {}, "session_id");
  add("args-array", ["x"], "session_id");
  add("args-number", 7, "session_id");
  return cases;
}

// The session_id and agent_id values of the resolve grid (undefined: the key is absent).
const SESSION_VALUES = {
  omitted: undefined,
  empty: "",
  spaces: "   ",
  number: 42,
  start: "start",
  "space-START-tab": " START\t",
  sTaRt: "sTaRt",
  "bom-start": "﻿start",
  "start-nel": "start\u0085",
  "long-s-tart": "ſtart",
  restart: "restart",
  "ses_bogus": "ses_bogus",
  valid: T,
  "valid-spaced": `  ${T} `,
  // trim removes a trailing newline, not a trailing NEL.
  "valid-newline": `${T}\n`,
  "valid-nel": `${T}\u0085`,
};
const AGENT_VALUES = { omitted: undefined, empty: "", "spaced-a": "  a  " };

function resolveCases() {
  const resolve = functions["oracle.resolveHandles"];
  const cases = [];
  const add = (name, input, tsTest) => cases.push(caseOf(`resolve:${name}`, { input, output: resolve(input) }, tsTest));
  for (const hookMode of [false, true]) {
    for (const agentTracking of [false, true]) {
      for (const sessionParamIsOurs of [true, false]) {
        const flags = [
          hookMode ? "hook-mode" : "prompted",
          agentTracking ? "tracking" : "no-tracking",
          sessionParamIsOurs ? "ours" : "foreign",
        ].join("-");
        add(`${flags}:no-args`, { hookMode, agentTracking, sessionParamIsOurs });
        for (const [sessionLabel, session] of Object.entries(SESSION_VALUES)) {
          for (const [agentLabel, agent] of Object.entries(AGENT_VALUES)) {
            const args = defined({ session_id: session, agent_id: agent });
            add(`${flags}:session-${sessionLabel}:agent-${agentLabel}`, { args, hookMode, agentTracking, sessionParamIsOurs });
          }
        }
      }
    }
  }
  // The tests of handles.test.ts. Prompted mode unless hookMode says otherwise; the session param is ours
  // unless sessionParamIsOurs says otherwise.
  const ts = (name, args, flags, tsTest) =>
    add(`ts:${name}`, { args, hookMode: false, agentTracking: false, sessionParamIsOurs: true, ...flags }, tsTest);
  ts("mint-no-agent", {}, { agentTracking: true }, [
    k(`${PROMPTED}mints the task but never the agent when nothing supplied (agent tracking on)`),
  ]);
  ts("subagent", { session_id: sid("parent") }, { agentTracking: true }, [
    k(`${PROMPTED}subagent flow: supplied task + omitted agent leaves the agent unresolved`),
  ]);
  ts("supplied-verbatim", { session_id: sid("anythinggoes"), agent_id: ` ${A} ` }, { agentTracking: true }, [
    k(`${PROMPTED}uses supplied handles verbatim`),
  ]);
  ts("tracking-off", { agent_id: "agt_ignored" }, {}, [k(`${PROMPTED}enableAgentTracking:false resolves no agent id at all`)]);
  ts("malformed", { session_id: "nope" }, {}, [
    k(`${PROMPTED}rejects a malformed supplied session_id: sessionless, source invalid`),
  ]);
  ts("secret", { session_id: "sk_live_secret_token" }, {}, [k(`${PROMPTED}never stores the raw rejected value`)]);
  ts("well-formed", { session_id: sid("parent") }, {}, [k(`${PROMPTED}accepts a well-formed supplied session_id verbatim`)]);
  const foreign = k(`${PROMPTED}foreign param: sessionless regardless of what the agent sent`);
  ts("foreign-with-value", { session_id: "customer-value" }, { sessionParamIsOurs: false }, [foreign]);
  ts("foreign-without-value", {}, { sessionParamIsOurs: false }, [foreign]);
  ts("hook-over-foreign", { session_id: "customer-value" }, { hookMode: true, sessionParamIsOurs: false }, [
    k(`${PROMPTED}hook mode wins over foreign: arguments are never read`),
  ]);
  ts("still-mints", {}, {}, [k(`${PROMPTED}still mints when nothing is supplied on our own param`)]);
  ts("start", { session_id: "start" }, {}, [
    k(`${PROMPTED}start sentinel resolves exactly like omission: fresh mint, source minted`),
  ]);
  const sentinels = [["START", "START"], ["Start", "Start"], ["sTaRt", "sTaRt"], ["  start  ", "spaces-start-spaces"], [" START\t", "space-START-tab"]];
  for (const [value, label] of sentinels) {
    const title = `${PROMPTED}start sentinel is case-insensitive and whitespace-tolerant: ${JSON.stringify(value)} mints`;
    ts(`sentinel-${label}`, { session_id: value }, {}, [k(title)]);
  }
  for (const value of ["restart", "start now", "starts", "star t"]) {
    const label = value.replace(" ", "-space-");
    ts(`near-sentinel-${label}`, { session_id: value }, {}, [k(`${PROMPTED}near-sentinel values stay on the invalid path`)]);
  }
  ts("foreign-start", { session_id: "start" }, { sessionParamIsOurs: false }, [
    k(`${PROMPTED}foreign param: start is the customer's value, never a sentinel`),
  ]);
  ts("hook-provisional", {}, { hookMode: true }, [
    k(`${HOOK}returns a provisional resolution: empty id, source 'hook', hookMode on`),
  ]);
  ts("hook-ignores-session", { session_id: "ses_agent_sent" }, { hookMode: true }, [
    k(`${HOOK}ignores agent-supplied session_id — the hook wins`),
  ]);
  const hookAgent = k(`${HOOK}hook mode: supplied agent_id resolves, omitted stays unresolved`);
  ts("hook-agent-supplied", { agent_id: A }, { hookMode: true, agentTracking: true }, [hookAgent]);
  ts("hook-agent-omitted", {}, { hookMode: true, agentTracking: true }, [hookAgent]);
  return cases;
}

const SOURCES = ["minted", "supplied", "invalid", "foreign", "hook"];

/** A resolution; `agentId` undefined: none. */
function resolution(sessionId, sessionSource, hookMode, agentId) {
  return { sessionId, sessionSource, hookMode, ...(agentId !== undefined && { agentId, agentSource: "supplied" }) };
}

function mintBackCases(core) {
  const { buildMintBackText, buildStructuredMintBack } = core.handles;
  const cases = [];
  const mintText = (name, input, tsTest) =>
    cases.push(caseOf(`mint-back:${name}`, { input, output: buildMintBackText(input) }, tsTest));
  const structured = (name, input, tsTest) =>
    cases.push(caseOf(`structured:${name}`, { input, output: buildStructuredMintBack(input) }, tsTest));
  // One case per resolution kind (and hook mode), from constants.json.
  mintText("minted", resolution(SESSION_ID, "minted", false));
  mintText("invalid", resolution("", "invalid", false));
  mintText("supplied", resolution(SESSION_ID, "supplied", false));
  mintText("foreign", resolution("", "foreign", false));
  mintText("hook", resolution("", "hook", true));
  // Every source in and out of hook mode, without an agent ID, with one, and with an empty one.
  for (const source of SOURCES) {
    const sessionId = ["minted", "supplied"].includes(source) ? T : "";
    for (const hookMode of [false, true]) {
      for (const [agentLabel, agentId] of [["none", undefined], ["set", A], ["empty", ""]]) {
        const name = `${source}:${hookMode ? "hook-mode" : "prompted"}:agent-${agentLabel}`;
        const input = resolution(sessionId, source, hookMode, agentId);
        mintText(name, input);
        structured(name, input);
      }
    }
  }
  // The tests of handles.test.ts.
  mintText("ts:minted-with-agent", resolution(T, "minted", false, A), [
    k(`${MINT_TEXT}task minted: task-only block, no agent mention even when agent supplied`),
  ]);
  mintText("ts:minted-without-agent", resolution(T, "minted", false), [
    k(`${MINT_TEXT}task minted without agent tracking: same task-only block`),
  ]);
  mintText("ts:supplied", resolution(T, "supplied", false, A), [
    k(`${MINT_TEXT}task supplied -> null (nothing to announce, agent never mints)`),
  ]);
  mintText("ts:hook-mode-minted", resolution(T, "minted", true, A), [
    k(`${MINT_TEXT}hook mode never emits task instructions, even when hook-null minted`),
  ]);
  mintText("ts:invalid", resolution("", "invalid", false), [
    k(`${INVALID_FOREIGN}invalid: corrects the agent without issuing a replacement`),
    k(`${INVALID_FOREIGN}invalid: offers an escape to an agent that was never issued one`),
  ]);
  const foreignSilent = k(`${INVALID_FOREIGN}foreign: says nothing when agent_id is not in play either`);
  mintText("ts:foreign", resolution("", "foreign", false), [foreignSilent]);
  structured("ts:minted-with-agent", resolution(T, "minted", false, A), [
    k(`${STRUCTURED}task minted + agent supplied: both ids, status issued`),
  ]);
  structured("ts:supplied-with-agent", resolution(T, "supplied", false, A), [
    k(`${STRUCTURED}steady state (both supplied): ids plus status active`),
  ]);
  structured("ts:supplied-without-agent", resolution(T, "supplied", false), [
    k(`${STRUCTURED}agent tracking off: task only, status active`),
  ]);
  structured("ts:hook-mode-minted-with-agent", resolution(T, "minted", true, A), [
    k(`${STRUCTURED}hook mode never exposes session_id or status; supplied agent is echoed`),
  ]);
  structured("ts:hook-mode-without-agent", resolution(T, "hook", true), [
    k(`${STRUCTURED}hook mode with agent tracking off: nothing echoable -> null`),
  ]);
  structured("ts:invalid", resolution("", "invalid", false), [
    k(`${INVALID_FOREIGN}invalid: structured mirror carries status unrecognized but no session_id`),
  ]);
  structured("ts:invalid-with-agent", resolution("", "invalid", false, A_SHORT), [
    k(`${INVALID_FOREIGN}invalid + agent supplied: agent_id rides along with the unrecognized status`),
  ]);
  structured("ts:foreign", resolution("", "foreign", false), [foreignSilent]);
  structured("ts:foreign-with-agent", resolution("", "foreign", false, A_SHORT), [
    k(`${INVALID_FOREIGN}foreign: never echoes session state, which is the customer's`),
    k(`${INVALID_FOREIGN}foreign: still echoes agent_id, which AgentCat did inject`),
  ]);
  return cases;
}

// The agent IDs of the tags cases; the tag keeps at most 200 UTF-16 units.
const TAG_AGENT_IDS = {
  plain: A,
  crlf: "a\r\nb",
  "line-breaks": "line1\nline2\r\nline3",
  "units-199": "a".repeat(199),
  "units-200": "a".repeat(200),
  "units-201": "a".repeat(201),
  "units-250": "a".repeat(250),
  // An astral character on units 200 and 201: TS keeps its first half, Rust drops it (RD-22).
  "emoji-at-200": `${"a".repeat(199)}😀`,
  "emoji-ends-at-200": `${"a".repeat(198)}😀`,
  "crlf-at-200": `${"a".repeat(199)}\r\n`,
  "bmp-201": "é".repeat(201),
  "emoji-101": "😀".repeat(101),
};
const PROTOCOL_VERSIONS = { absent: undefined, empty: "", "2025-11-25": "2025-11-25" };

function tagCases(core) {
  const cases = [];
  const add = (name, res, protocolVersion, tsTest) => {
    const input = defined({ res, protocolVersion });
    cases.push(caseOf(`tags:${name}`, { input, output: core.handles.buildHandleTags(res, protocolVersion) }, tsTest));
  };
  for (const [versionLabel, version] of Object.entries(PROTOCOL_VERSIONS)) {
    const suffix = version === undefined ? "" : `:version-${versionLabel}`;
    for (const [label, agentId] of Object.entries(TAG_AGENT_IDS)) {
      add(`agent-id-${label}${suffix}`, resolution(T, "supplied", false, agentId), version);
    }
    add(`agent-id-empty${suffix}`, resolution(T, "supplied", false, ""), version);
    for (const source of SOURCES) {
      add(`no-agent-${source}${suffix}`, resolution(source === "supplied" ? T : "", source, source === "hook"), version);
    }
  }
  // The tests of handles.test.ts.
  add("ts:stamps-all", resolution(T, "supplied", false, A), "2026-07-28", [
    k(`${TAGS}stamps sources, agent id, and protocol version`),
  ]);
  add("ts:omits-absent", resolution(T, "hook", true), undefined, [k(`${TAGS}omits agent and protocol keys when absent`)]);
  add("ts:clamps-500", resolution(T, "supplied", false, "a".repeat(500)), undefined, [
    k(`${TAGS}clamps a >200-char agent_id to exactly 200 chars in the tag only`),
  ]);
  add("ts:strips-newlines", resolution(T, "supplied", false, "line1\nline2\r\nline3"), undefined, [
    k(`${TAGS}strips newlines from the agent_id tag`),
  ]);
  add("ts:normal-agent", resolution(T, "supplied", false, A), undefined, [
    k(`${TAGS}passes a normal agent_id through unchanged`),
  ]);
  return cases;
}

// The hook values and project IDs of the hook-value grid (undefined: absent).
const HOOK_VALUES = {
  "customer-abc": "customer-abc",
  "padded-x": "  x  ",
  empty: "",
  spaces: "   ",
  null: null,
  absent: undefined,
  "bom-corr-nel": "﻿corr\u0085",
};
const PROJECT_IDS = { absent: undefined, empty: "", proj_1: "proj_1" };

function hookValueCases() {
  const fromHook = functions["oracle.sessionFromHookValue"];
  const cases = [];
  const add = (name, value, projectId, tsTest) => {
    const input = defined({ value, projectId });
    cases.push(caseOf(`hook-value:${name}`, { input, output: fromHook(input) }, tsTest));
  };
  for (const [valueLabel, value] of Object.entries(HOOK_VALUES)) {
    for (const [projectLabel, projectId] of Object.entries(PROJECT_IDS)) {
      add(`value-${valueLabel}:project-${projectLabel}`, value, projectId);
    }
  }
  // The tests of handles.test.ts.
  add("ts:customer-42", "customer-42", "proj_1", [k(`${HOOK_VALUE}derives deterministically from the hook value + project`)]);
  add("ts:padded-corr-1", "  corr-1  ", "proj_1", [k(`${HOOK_VALUE}trims the value before deriving`)]);
  add("ts:null", null, "proj_1", [k(`${HOOK_VALUE}mints a fresh id per null — silent mint, never 'supplied'`)]);
  add("ts:spaces", "   ", "proj_1", [k(`${HOOK_VALUE}treats a whitespace-only value as null`)]);
  return cases;
}

export async function generate() {
  const core = await loadCore();
  const cases = [
    ...deriveCases(core),
    ...validCases(core),
    ...extractCases(core),
    ...resolveCases(),
    ...mintBackCases(core),
    ...tagCases(core),
    ...hookValueCases(),
  ];
  return { cases };
}

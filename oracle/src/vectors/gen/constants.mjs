// Vectors for the constants of the TS SDK: every exported plain value of
// constants.ts, handles.ts and tools.ts (enumerated from the module objects, so
// a new export shows up in the nightly diff), plus the output of the exported
// functions for fixed inputs.
import { loadCore } from "oracle/bundle";
import { tsKey } from "../support.mjs";

const k = (title) => tsKey("constants", title);
const PINS = "v4 copy pins (byte-exact) > pins ";
const CHANNEL = "handle param descriptions name the delivery channel > ";

// The TS tests of src/tests/constants.test.ts that pin each export, by export name. A test
// that pins several exports appears under each of them. Tests of AGENTCAT_SOURCE and
// DIAGNOSTICS_SCOPE_NAME are not linked (exporters and diagnostics: second pass). The
// value-contract test also runs the pattern against inputs; Task 8 claims it in handles.json.
const TS_TESTS = {
  AGENTCAT_CUSTOM_EVENT_TYPE: [k("brand constants (wire literals) > emits the agentcat custom event type on the wire")],
  MCP_SESSION_KEY: [
    k("brand constants (wire literals) > mirrors under the mcp_session key"),
    k(`${CHANNEL}session_id pre-announces both delivery channels and both text headers`),
  ],
  SESSION_ID_PARAM_DESCRIPTION: [
    k(`${PINS}SESSION_ID_PARAM_DESCRIPTION`),
    k(`${CHANNEL}session_id pre-announces both delivery channels and both text headers`),
  ],
  MINT_BACK_HEADER_ISSUED: [k(`${CHANNEL}session_id pre-announces both delivery channels and both text headers`)],
  MINT_BACK_HEADER_UNRECOGNIZED: [k(`${CHANNEL}session_id pre-announces both delivery channels and both text headers`)],
  AGENT_ID_PARAM_DESCRIPTION: [k(`${CHANNEL}agent_id describes self-chosen generation, not server delivery`)],
  MCP_SESSION_STATUS_DESCRIPTION: [k(`${PINS}MCP_SESSION_STATUS_DESCRIPTION`)],
  MINT_BACK_ISSUED_BODY: [k(`${PINS}MINT_BACK_ISSUED_BODY`)],
  MINT_BACK_UNRECOGNIZED_BODY: [k(`${PINS}MINT_BACK_UNRECOGNIZED_BODY`)],
};

const SESSION_ID = "ses_0123456789abcdefghijklmnopq";
const CONTEXT = "Looking for a tool that converts spreadsheets to charts.";

// Fixed inputs for the exported functions of constants.ts and tools.ts, by
// export name. A new function export there without an entry fails the run, so it
// cannot be left out unnoticed. The functions of handles.ts belong to later
// vector sets; only its plain values and buildMintBackText are taken here.
const FUNCTION_CASES = {
  // constants.ts
  mintBackSessionLine: (fn) => [{ input: SESSION_ID, value: fn(SESSION_ID) }],
  // tools.ts
  getReportMissingToolDescriptor: (fn) => [{ input: null, value: fn() }],
  handleReportMissing: (fn) => [{ input: { context: CONTEXT }, value: fn({ context: CONTEXT }) }],
};

// One buildMintBackText case per resolution kind (and hook mode), with fixed IDs.
const MINT_BACK_RESOLUTIONS = {
  minted: { sessionId: SESSION_ID, sessionSource: "minted", hookMode: false },
  invalid: { sessionId: "", sessionSource: "invalid", hookMode: false },
  supplied: { sessionId: SESSION_ID, sessionSource: "supplied", hookMode: false },
  foreign: { sessionId: "", sessionSource: "foreign", hookMode: false },
  hook: { sessionId: "", sessionSource: "hook", hookMode: true },
};

export async function generate() {
  const core = await loadCore();
  const cases = [];
  for (const file of ["constants", "handles", "tools"]) {
    const mod = core[file];
    for (const [name, value] of Object.entries(mod)) {
      if (typeof value !== "function") {
        cases.push({ name, value, ...(TS_TESTS[name] && { ts_test: TS_TESTS[name] }) });
        continue;
      }
      if (file === "handles") continue;
      if (!Object.hasOwn(FUNCTION_CASES, name)) {
        throw new Error(`gen/constants: new function export ${file}.${name}; add it to FUNCTION_CASES`);
      }
      for (const c of FUNCTION_CASES[name](value)) cases.push({ name, input: c.input, value: c.value });
    }
  }
  const { buildMintBackText } = core.handles;
  for (const [kind, input] of Object.entries(MINT_BACK_RESOLUTIONS)) {
    cases.push({ name: `mintBack:${kind}`, input, value: buildMintBackText(input) });
  }
  return { cases: cases.sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0)) };
}

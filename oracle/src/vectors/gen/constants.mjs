// Vectors for the constants of the TS SDK: every exported plain value of
// constants.ts, handles.ts and tools.ts (enumerated from the module objects, so
// a new export shows up in the nightly diff), plus the output of the exported
// functions for fixed inputs.
import { loadModule } from "oracle/bundle";

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
  const cases = [];
  for (const file of ["constants", "handles", "tools"]) {
    const mod = await loadModule(file);
    for (const [name, value] of Object.entries(mod)) {
      if (typeof value !== "function") {
        cases.push({ name, value });
        continue;
      }
      if (file === "handles") continue;
      if (!Object.hasOwn(FUNCTION_CASES, name)) {
        throw new Error(`gen/constants: new function export ${file}.${name}; add it to FUNCTION_CASES`);
      }
      for (const c of FUNCTION_CASES[name](value)) cases.push({ name, input: c.input, value: c.value });
    }
  }
  const { buildMintBackText } = await loadModule("handles");
  for (const [kind, input] of Object.entries(MINT_BACK_RESOLUTIONS)) {
    cases.push({ name: `mintBack:${kind}`, input, value: buildMintBackText(input) });
  }
  const seen = new Set();
  for (const { name } of cases) {
    if (seen.has(name)) throw new Error(`gen/constants: duplicate case name ${name}`);
    seen.add(name);
  }
  return { cases: cases.sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0)) };
}

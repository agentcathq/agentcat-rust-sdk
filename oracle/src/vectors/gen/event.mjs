// Vectors for the fields of a tool-call event that TS reads off the request (src/modules/session.ts):
// the client identity ladder and the protocol version tag.
//
// Case name prefixes, each with its fields:
//   client-info:<label>       input: {meta?, initialize?};  output: getClientInfoForRequest for a call
//                             whose params have the `_meta` `meta` (absent: no `_meta`), on a server
//                             whose getClientVersion() returns `initialize` (absent: undefined, no
//                             handshake). The output is absent when TS gives undefined; a narrowed
//                             candidate leaves out its undefined fields (oracle.clientInfo, functions.mjs)
//   protocol-version:<label>  input: {meta?};  output: getProtocolVersion for a call whose params have
//                             the `_meta` `meta` (absent: no `_meta`); absent when TS gives undefined
//
// No case has an `extra`: TS's v2 envelope (extra.mcpReq.envelope) has no rmcp counterpart, because
// rmcp moves the whole `_meta`, reserved keys included, into RequestContext::meta (C28). Every
// `initialize` value is one rmcp's Implementation can hold: an initialize request with any other
// clientInfo fails deserialization in rmcp (and in TS's own schema), so it never reaches the SDK.
import { functions } from "../../functions.mjs";
import { tsKey } from "../support.mjs";

const cases = [];

/** A JSON copy of `value`: no undefined, no prototype tricks. */
const plain = (value) => (value === undefined ? undefined : JSON.parse(JSON.stringify(value)));

/** Adds a case whose output is `output`, left out when it is undefined; `fields` follow it (ts_test). */
function add(name, input, output, fields = {}) {
  const value = plain(output);
  cases.push({ name, input: plain(input), ...(value === undefined ? {} : { output: value }), ...fields });
}

const CLIENT_INFO = "io.modelcontextprotocol/clientInfo";
const PROTOCOL_VERSION = "io.modelcontextprotocol/protocolVersion";

// ---------------------------------------------------------------------------------- client-info

/** The input {meta?, initialize?}, with the absent fields left out. */
function clientInput(meta, initialize) {
  return { ...(meta === undefined ? {} : { meta }), ...(initialize === undefined ? {} : { initialize }) };
}

/** Adds a client-info case; TS runs on the JSON copy of the input, the values the Rust test sees. */
function clientCase(name, meta, initialize, tsTest) {
  const input = plain(clientInput(meta, initialize));
  const fields = tsTest && { ts_test: [tsKey("client-identity", tsTest)] };
  add(`client-info:${name}`, input, functions["oracle.clientInfo"](input), fields);
}

/** The `_meta` with the clientInfo candidate `candidate`. */
const withCandidate = (candidate) => ({ [CLIENT_INFO]: candidate });

const LEGACY = { name: "legacy", version: "1" };
const HANDSHAKE = { name: "from-handshake", version: "9.9.9" };

function tsClientCases() {
  const get = "getClientInfoForRequest > ";
  clientCase("ts:meta-preferred", withCandidate({ name: "claude", version: "3.0" }), LEGACY, `${get}prefers per-request _meta clientInfo (modern client)`);
  clientCase("ts:initialize-fallback", undefined, LEGACY, `${get}falls back to getClientVersion (legacy initialize capture)`);
  clientCase("ts:both-missing", undefined, undefined, `${get}both missing -> undefined (client declined to identify)`);
  clientCase("ts:malformed-string", withCandidate("not-an-object"), undefined, `${get}ignores malformed _meta clientInfo`);
  clientCase("ts:non-string-version", withCandidate({ name: "x", version: 42 }), undefined, `${get}drops non-string fields from partially malformed _meta clientInfo`);
  // The envelope test's `extra` is {}, so it has no envelope: the _meta and handshake rungs alone.
  const ladder = "envelope-first identity ladder (v2 ctx.mcpReq.envelope) > falls back to _meta (v1 passthrough), then getClientVersion()";
  clientCase("ts:meta-name-only", withCandidate({ name: "meta-client" }), HANDSHAKE, ladder);
  clientCase("ts:handshake", undefined, HANDSHAKE, ladder);
  // The narrowing test of the envelope rung, with its candidate in `_meta`, where rmcp keeps the
  // envelope's keys. (The envelope tests of precedence over `_meta` have no case: rmcp has one place.)
  const narrowing = "envelope-first identity ladder (v2 ctx.mcpReq.envelope) > rejects non-string envelope fields (field-level narrowing)";
  clientCase("ts:non-string-name", withCandidate({ name: 42 }), HANDSHAKE, narrowing);
}

/** Candidate values at the clientInfo key, by label: the shapes narrowClientInfo tells apart. */
const CANDIDATES = {
  "name-version": { name: "client", version: "1.2.3" },
  "name-only": { name: "client" },
  "version-only": { version: "1.2.3" },
  "name-number-version": { name: "client", version: 1 },
  "number-name-version": { name: 1, version: "1.2.3" },
  "numbers": { name: 1, version: 2 },
  "null-name-version": { name: null, version: "1.2.3" },
  "nulls": { name: null, version: null },
  "empty-strings": { name: "", version: "" },
  "empty-name-number-version": { name: "", version: 0 },
  "bool-name": { name: true },
  "object-name": { name: { first: "client" } },
  "array-name": { name: ["client"] },
  "extra-fields": { name: "client", version: "1.2.3", title: "Client", websiteUrl: "https://example.com" },
  "other-fields-only": { title: "Client", description: "no name or version" },
  "unicode": { name: "клиент \u{1F600}", version: "β-2" },
  "empty-object": {},
  "array-empty": [],
  "array-of-strings": ["client", "1.2.3"],
  "array-of-object": [{ name: "client", version: "1.2.3" }],
  "null": null,
  "string": "client",
  "string-empty": "",
  "number": 7,
  "zero": 0,
  "true": true,
  "false": false,
};

/** Values getClientVersion() returns, by label: none (no handshake) and Implementation shapes. */
const INITIALIZE = {
  "no-initialize": undefined,
  "initialize": { name: "init-client", version: "2.1.0" },
};

function candidateCases() {
  for (const [label, candidate] of Object.entries(CANDIDATES)) {
    for (const [init, initialize] of Object.entries(INITIALIZE)) {
      clientCase(`${label}:${init}`, withCandidate(candidate), initialize);
    }
  }
}

function otherClientCases() {
  const init = INITIALIZE.initialize;
  // No candidate at the clientInfo key.
  for (const [label, initialize] of Object.entries(INITIALIZE)) {
    clientCase(`no-meta:${label}`, undefined, initialize);
    clientCase(`meta-empty:${label}`, {}, initialize);
    clientCase(`meta-other-keys:${label}`, { progressToken: 3, [PROTOCOL_VERSION]: "2025-11-25" }, initialize);
    clientCase(`meta-unprefixed-key:${label}`, { clientInfo: { name: "client", version: "1.2.3" } }, initialize);
  }
  // The other keys of `_meta` do not matter.
  clientCase("meta-beside-other-keys", { progressToken: "t", ...withCandidate({ name: "client", version: "1.2.3" }) }, init);
  // Implementation shapes from the handshake: TS hands back the whole object (the event reads its
  // name and version).
  const fallback = (label, initialize) => clientCase(`initialize:${label}`, withCandidate(null), initialize);
  fallback("with-optional-fields", {
    name: "init-client",
    title: "Init Client",
    version: "2.1.0",
    description: "a client",
    websiteUrl: "https://example.com",
  });
  fallback("empty-strings", { name: "", version: "" });
  fallback("unicode", { name: "клиент \u{1F600}", version: "β-2" });
}

// ------------------------------------------------------------------------------ protocol-version

/** Adds a protocol-version case for the `_meta` `meta` (undefined: no `_meta`). */
function versionCase(name, meta, tsTest) {
  const input = plain(meta === undefined ? {} : { meta });
  const request = { params: input.meta === undefined ? {} : { _meta: input.meta } };
  const fields = tsTest && { ts_test: [tsKey("client-identity", `getProtocolVersion > ${tsTest}`)] };
  add(`protocol-version:${name}`, input, functions["session.getProtocolVersion"](request), fields);
}

function versionCases() {
  const withVersion = (value) => ({ [PROTOCOL_VERSION]: value });
  versionCase("ts:reads-meta", withVersion("2026-07-28"), "reads _meta protocolVersion");
  // The TS test also passes an undefined request, which rmcp cannot hand over.
  versionCase("ts:absent", undefined, "absent -> undefined");

  versionCase("legacy", withVersion("2025-11-25"));
  versionCase("unknown-text", withVersion("not-a-version"));
  versionCase("space", withVersion(" "));
  versionCase("unicode", withVersion("\u{1F600}"));
  versionCase("empty", withVersion(""));
  versionCase("number", withVersion(20260728));
  versionCase("zero", withVersion(0));
  versionCase("null", withVersion(null));
  versionCase("true", withVersion(true));
  versionCase("object", withVersion({ version: "2026-07-28" }));
  versionCase("array", withVersion(["2026-07-28"]));
  versionCase("meta-empty", {});
  versionCase("meta-other-keys", { progressToken: 1, [CLIENT_INFO]: { name: "client" } });
  versionCase("meta-unprefixed-key", { protocolVersion: "2026-07-28" });
  versionCase("meta-beside-other-keys", { progressToken: 1, ...withVersion("2025-06-18") });
}

export async function generate() {
  cases.length = 0;
  tsClientCases();
  candidateCases();
  otherClientCases();
  versionCases();
  return { cases };
}

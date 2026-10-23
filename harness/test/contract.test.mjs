// L4 publish-body contract: every body a server publishes must validate against
// the publishEvent request schema of agentcat-server (harness/contract/), and
// the driver fails a run that publishes an invalid body.
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { before, test } from "node:test";

import { ensureTs } from "oracle/fetch-ts";

import { convert } from "../src/contract/extract-schema.mjs";
import { contractResult, validateRequests } from "../src/contract/validate.mjs";
import {
  BASIC,
  CONTRACT,
  HARNESS,
  RUST_SERVER,
  TS_SERVER,
  TS_SERVER_PLUS,
  TS_V2_SERVER,
  driver,
  outFile,
  readJson,
  run,
  tempDir,
  writeJson,
} from "./support.mjs";

const VALIDATE = join(HARNESS, "src", "contract", "validate.mjs");
const EXTRACT = join(HARNESS, "src", "contract", "extract-schema.mjs");
const CONTRACT_DIR = join(HARNESS, "contract");
const PUBLISH_PATH = "/v1/public/events/";
// The private agentcat-server checkout; absent in CI (R14), so the --check test skips there.
const SERVER_DIR = process.env.AGENTCAT_SERVER_DIR ?? join(homedir(), "agentcat-server");

/** The raw requests of a fresh basic.json TS stdio run. */
let raw;

before(async () => {
  await ensureTs();
  const res = await driver(BASIC, "contract", "stdio");
  assert.equal(res.code, 0, res.stderr);
  raw = readJson(outFile("basic", "contract", "stdio", "raw"));
});

const publishes = () => raw.filter((r) => r.method === "POST" && r.path === PUBLISH_PATH);

/** The first publish of the fresh run with its body changed by `edit`. */
function editedPublish(edit) {
  const request = structuredClone(publishes()[0]);
  edit(request.body);
  return request;
}

/** validate.mjs on a temp raw file holding `requests`. */
function validateCli(t, requests, flags = []) {
  return run(process.execPath, [VALIDATE, ...flags, writeJson(join(tempDir(t), "raw.json"), requests)]);
}

/** A one-session scenario with one echo call (one publish when tracked). */
function oneCall(t, expectPublishes) {
  return writeJson(join(tempDir(t), "one.json"), {
    name: "one",
    projectId: readJson(BASIC).projectId,
    options: {},
    expectPublishes,
    sessions: [{ steps: [{ op: "callTool", name: "echo", args: { msg: "hi" } }] }],
  });
}

test("every publish body of a fresh basic.json TS run validates", async (t) => {
  assert.equal(publishes().length, 11);
  assert.deepEqual(validateRequests(raw), { checked: 11, invalid: [] });
  const report = readJson(outFile("basic", "contract", "stdio", "report"));
  assert.equal(report.contract, "ok");
  assert.equal(report.contractChecked, 11);
  const res = await validateCli(t, raw);
  assert.equal(res.code, 0, res.stdout + res.stderr);
  assert.match(res.stdout, /11 publish bodies, all valid/);
});

test("the same body with project_id removed fails", async (t) => {
  // The TS client also sends the camelCase alias projectId; it does not count.
  const request = editedPublish((body) => {
    assert.ok("projectId" in body);
    delete body.project_id;
  });
  const { checked, invalid } = validateRequests([request]);
  assert.equal(checked, 1);
  assert.equal(invalid.length, 1);
  assert.equal(invalid[0].id, request.body.id);
  assert.deepEqual(
    invalid[0].errors.map((e) => [e.instancePath, e.keyword, e.params.missingProperty]),
    [["", "required", "project_id"]],
  );
  const res = await validateCli(t, [request]);
  assert.equal(res.code, 1, res.stdout + res.stderr);
  assert.match(res.stdout, new RegExp(`INVALID publish seq=\\d+ id=${request.body.id} .*must have required property 'project_id'`));
});

test("the same body with session_id: 5 fails", async (t) => {
  const request = editedPublish((body) => {
    body.session_id = 5;
  });
  const { invalid } = validateRequests([request]);
  assert.equal(invalid.length, 1);
  assert.deepEqual(
    invalid[0].errors.map((e) => [e.instancePath, e.keyword]),
    [["/session_id", "type"]],
  );
  const res = await validateCli(t, [request]);
  assert.equal(res.code, 1, res.stdout + res.stderr);
  assert.match(res.stdout, /\/session_id must be string,null/);
});

test("the same body with session_id: null validates (OpenAPI nullable)", () => {
  const request = editedPublish((body) => {
    body.session_id = null;
  });
  assert.deepEqual(validateRequests([request]), { checked: 1, invalid: [] });
});

test("timestamp format, ip_address format and the event_type enum are enforced", () => {
  const cases = [
    ["timestamp without a timezone", (b) => (b.timestamp = "2026-10-20T12:00:00"), "/timestamp", "format"],
    // ajv-formats "full" mode checks the calendar; "fast" mode would let this through.
    ["timestamp on a day that does not exist", (b) => (b.timestamp = "2026-02-30T12:00:00Z"), "/timestamp", "format"],
    ["ip_address in IPv6", (b) => (b.ip_address = "::1"), "/ip_address", "format"],
    ["event_type the server stores but the API does not accept", (b) => (b.event_type = "mcpcat:custom"), "/event_type", "enum"],
  ];
  for (const [name, edit, instancePath, keyword] of cases) {
    const { invalid } = validateRequests([editedPublish(edit)]);
    assert.equal(invalid.length, 1, name);
    assert.deepEqual(
      invalid[0].errors.map((e) => [e.instancePath, e.keyword]),
      [[instancePath, keyword]],
      name,
    );
  }
});

test("a run with no publish bodies fails the contract unless empty is allowed", async (t) => {
  assert.deepEqual(contractResult([]), { checked: 0, contract: "no publish bodies" });
  assert.deepEqual(contractResult([], { allowEmpty: true }), { checked: 0, contract: "ok" });
  // The only POST went to the path without the trailing slash: not a publish.
  const wrongPath = [{ ...structuredClone(publishes()[0]), path: "/v1/public/events" }];
  for (const requests of [[], wrongPath]) {
    const res = await validateCli(t, requests);
    assert.equal(res.code, 1, res.stdout + res.stderr);
    assert.match(res.stdout, /contract: 0 publish bodies; expected at least one/);
  }
  let res = await validateCli(t, [], ["--allow-empty"]);
  assert.equal(res.code, 0, res.stdout + res.stderr);
  assert.match(res.stdout, /contract: 0 publish bodies \(allowed by --allow-empty\)/);
  // --allow-empty never excuses an invalid body.
  const bad = editedPublish((body) => {
    body.session_id = 5;
  });
  res = await validateCli(t, [bad], ["--allow-empty"]);
  assert.equal(res.code, 1, res.stdout + res.stderr);
  res = await run(process.execPath, [VALIDATE, "--allow-emptyy", join(tempDir(t), "raw.json")]);
  assert.equal(res.code, 2, res.stderr);
});

test("an untracked run that expects no publishes passes the contract with 0 bodies checked", { timeout: 120_000 }, async (t) => {
  const res = await driver(oneCall(t, 0), "untracked0", "stdio", [], { env: { HARNESS_NO_TRACK: "1" } });
  assert.equal(res.code, 0, res.stderr);
  const report = readJson(outFile("one", "untracked0", "stdio", "report"));
  assert.equal(report.contract, "ok");
  assert.equal(report.contractChecked, 0);
});

test("a run that expects publishes but sends none fails the contract", { timeout: 120_000 }, async (t) => {
  const res = await driver(oneCall(t, 1), "untracked1", "stdio", [], { env: { HARNESS_NO_TRACK: "1" } });
  assert.equal(res.code, 1, res.stderr);
  assert.match(res.stderr, /driver: contract: no publish bodies to check/);
  const report = readJson(outFile("one", "untracked1", "stdio", "report"));
  assert.equal(report.publishes, 0);
  assert.equal(report.contract, "no publish bodies");
  assert.equal(report.contractChecked, 0);
});

test("validate.mjs exits 2 without a readable raw file", async (t) => {
  let res = await run(process.execPath, [VALIDATE]);
  assert.equal(res.code, 2, res.stderr);
  res = await run(process.execPath, [VALIDATE, join(tempDir(t), "missing.json")]);
  assert.equal(res.code, 2, res.stderr);
});

test("the driver exits 1 and records the errors when a publish body is invalid", { timeout: 120_000 }, async (t) => {
  const env = { FIXTURE_INVALID_PUBLISH: "1" };
  const res = await driver(oneCall(t, 2), "badbody", "stdio", [], { server: TS_SERVER_PLUS, env });
  assert.equal(res.code, 1, res.stderr);
  assert.match(res.stderr, /driver: 1 publish body\/bodies fail the contract/);
  const report = readJson(outFile("one", "badbody", "stdio", "report"));
  assert.equal(report.publishes, 2);
  assert.equal(report.nonPublishRequests, 0);
  assert.equal(report.contractChecked, 2);
  assert.equal(report.contract.length, 1);
  assert.equal(report.contract[0].id, "evt_fixture_invalid");
  assert.deepEqual(report.contract[0].errors.map((e) => [e.instancePath, e.keyword]).sort(), [
    ["", "required"],
    ["/session_id", "type"],
  ]);
});

test("convert: nullable, inlined $ref, allOf, required and additionalProperties as written", () => {
  const doc = {
    components: {
      schemas: {
        Err: {
          type: "object",
          required: ["message"],
          properties: { message: { type: "string" } },
          additionalProperties: false,
        },
      },
    },
  };
  const schema = {
    type: "object",
    required: ["a"],
    properties: {
      a: { type: "string", nullable: true, description: "nullable string" },
      b: { $ref: "#/components/schemas/Err" },
      c: { allOf: [{ $ref: "#/components/schemas/Err" }], nullable: true },
      d: { $ref: "#/components/schemas/Err", nullable: true },
      e: { nullable: true },
      f: { type: "integer", minimum: 0, exclusiveMinimum: true, nullable: false },
      g: { type: "object", additionalProperties: { type: "string" }, "x-vendor": 1 },
      h: { type: "string", enum: ["x", null], nullable: true },
    },
    additionalProperties: true,
    example: { a: "x" },
  };
  const err = {
    type: "object",
    required: ["message"],
    properties: { message: { type: "string" } },
    additionalProperties: false,
  };
  assert.deepEqual(convert(schema, doc), {
    type: "object",
    required: ["a"],
    properties: {
      a: { type: ["string", "null"], description: "nullable string" },
      b: err,
      c: { anyOf: [{ type: "null" }, { allOf: [err] }] },
      d: { anyOf: [{ type: "null" }, err] },
      e: { anyOf: [{ type: "null" }, {}] },
      f: { type: "integer", exclusiveMinimum: 0 },
      g: { type: "object", additionalProperties: { type: "string" } },
      h: { type: ["string", "null"], enum: ["x", null] },
    },
    additionalProperties: true,
    examples: [{ a: "x" }],
  });
});

test("convert rejects what it cannot convert faithfully", () => {
  const doc = {
    components: {
      schemas: {
        Loop: { type: "object", properties: { next: { $ref: "#/components/schemas/Loop" } } },
      },
    },
  };
  const cases = [
    [{ type: "object", discriminator: { propertyName: "kind" } }, /discriminator/],
    [{ type: "string", readOnly: true }, /readOnly/],
    [{ type: "string", writeOnly: true }, /writeOnly/],
    [{ $ref: "other.yml#/X" }, /only local/],
    [{ $ref: "#/components/schemas/Missing" }, /does not resolve/],
    [{ $ref: "#/components/schemas/Loop" }, /recursive/],
    [{ $ref: "#/components/schemas/Loop", minLength: 1 }, /sibling/],
    [{ type: "string", enum: ["a"], nullable: true }, /enum without null/],
    [{ type: ["string", "null"] }, /type/],
    [{ type: "string", exclusiveMinimum: true }, /exclusiveMinimum without minimum/],
    [{ type: "string", patternProperties: {} }, /patternProperties/],
  ];
  for (const [schema, message] of cases) {
    assert.throws(() => convert(schema, doc), message, JSON.stringify(schema));
  }
});

test("the committed schema names its source and keeps the OpenAPI rules", () => {
  assert.match(readFileSync(join(CONTRACT_DIR, "SOURCE"), "utf8"), /^agentcat-server [0-9a-f]{40} schema\.yml\n$/);
  const schema = readJson(join(CONTRACT_DIR, "publish-event.schema.json"));
  assert.equal(schema.$schema, "https://json-schema.org/draft/2020-12/schema");
  assert.deepEqual(schema.required, ["project_id"]);
  assert.equal(schema.additionalProperties, true);
  assert.deepEqual(schema.properties.session_id.type, ["string", "null"]);
  assert.equal(schema.properties.tags.additionalProperties.type, "string");
  assert.ok(schema.properties.event_type.enum.includes("agentcat:custom"));
});

test(
  "extract-schema --check: the committed schema is what the recorded SHA produces",
  { timeout: 60_000, skip: existsSync(join(SERVER_DIR, ".git")) ? false : `no agentcat-server checkout at ${SERVER_DIR}` },
  async () => {
    const res = await run(process.execPath, [EXTRACT, "--check", SERVER_DIR]);
    assert.equal(res.code, 0, res.stdout + res.stderr);
    assert.match(res.stdout, /up to date/);
  },
);

test("the committed tool contract is what ts-v2 lists on the 2025-11-25 lane, whole (R20)", { timeout: 120_000 }, async (t) => {
  const path = writeJson(join(tempDir(t), "list.json"), {
    name: "answerkey",
    projectId: readJson(BASIC).projectId,
    options: {},
    expectPublishes: 0,
    sessions: [{ steps: [{ op: "listTools" }] }],
  });
  const res = await driver(path, "answerkey", "stdio", [], { server: TS_V2_SERVER, env: { HARNESS_NO_TRACK: "1" } });
  assert.equal(res.code, 0, res.stderr);
  const listed = readJson(outFile("answerkey", "answerkey", "stdio", "transcript")).find((e) => e.op === "listTools").result.tools;
  const contract = readJson(CONTRACT);
  assert.deepEqual(listed, contract);
  for (const tool of contract) {
    assert.ok(!("execution" in tool), `${tool.name}: v1's execution field is not in the contract`);
    assert.deepEqual(Object.keys(tool.inputSchema.properties), Object.keys(listed.find((l) => l.name === tool.name).inputSchema.properties));
  }
});

test("ts-v1's tools/list, minus execution, equals the contract file (v1 and v2 stay in step)", { timeout: 120_000 }, async (t) => {
  const path = writeJson(join(tempDir(t), "list.json"), {
    name: "v1list",
    projectId: readJson(BASIC).projectId,
    options: {},
    expectPublishes: 0,
    sessions: [{ steps: [{ op: "listTools" }] }],
  });
  const res = await driver(path, "v1list", "stdio", [], { server: TS_SERVER, env: { HARNESS_NO_TRACK: "1" } });
  assert.equal(res.code, 0, res.stderr);
  const listed = readJson(outFile("v1list", "v1list", "stdio", "transcript")).find((e) => e.op === "listTools").result.tools;
  assert.ok(listed.every((tool) => tool.execution !== undefined), "v1 adds execution");
  assert.deepEqual(listed.map(({ execution, ...tool }) => tool), readJson(CONTRACT));
});

// The Rust reference server (harness/rust-server) embeds the contract file and must list it whole.
test("untracked, the Rust server lists the tool contract whole on stdio at 2025-11-25 and on HTTP at 2026-07-28", { timeout: 120_000 }, async (t) => {
  const path = writeJson(join(tempDir(t), "list.json"), {
    name: "rustlist",
    projectId: readJson(BASIC).projectId,
    options: {},
    expectPublishes: 0,
    sessions: [{ steps: [{ op: "listTools" }] }],
  });
  const contract = readJson(CONTRACT);
  // deepEqual ignores key order; the diff compares the order of each tool's inputSchema.properties
  // (normalizer rule property-order), so check it too.
  const propertyOrder = (tools) => tools.map((tool) => [tool.name, Object.keys(tool.inputSchema.properties)]);
  for (const [transport, lane] of [["stdio", "2025-11-25"], ["http", "2026-07-28"]]) {
    const label = `rustlist-${lane}`;
    const res = await driver(path, label, transport, ["--lane", lane], { server: RUST_SERVER, env: { HARNESS_NO_TRACK: "1" } });
    assert.equal(res.code, 0, `${transport} ${lane}: ${res.stderr}`);
    const transcript = readJson(outFile("rustlist", label, transport, "transcript"));
    assert.deepEqual(transcript.filter((e) => e.op === "initialize").map((e) => e.result.protocolVersion), [lane]);
    const listed = transcript.find((e) => e.op === "listTools").result.tools;
    assert.deepEqual(listed, contract, `${transport} ${lane}`);
    assert.deepEqual(propertyOrder(listed), propertyOrder(contract), `${transport} ${lane}`);
  }
});

/** The Rust server's environment for a run that must stop before serving. */
function rustEnv(t, extra) {
  const env = { ...process.env, DISABLE_DIAGNOSTICS: "true", HOME: tempDir(t), HARNESS_PROJECT_ID: readJson(BASIC).projectId };
  delete env.HARNESS_NO_TRACK;
  delete env.HARNESS_OPTIONS;
  for (const [key, value] of Object.entries(extra)) {
    if (value === undefined) delete env[key];
    else env[key] = value;
  }
  return env;
}

test("the Rust server refuses a non-local AGENTCAT_API_URL with exit 2", async (t) => {
  const [cmd, ...args] = RUST_SERVER;
  // The third URL's host is api.agentcat.com (127.0.0.1 is its user name); the last case leaves it unset.
  const urls = ["https://api.agentcat.com", "http://localhost.example.com", "http://127.0.0.1@api.agentcat.com", undefined];
  for (const mode of [["stdio"], ["http", "0"], ["http-stateless", "0"]]) {
    for (const url of urls) {
      const res = await run(cmd, [...args, ...mode], { env: rustEnv(t, { AGENTCAT_API_URL: url }) });
      assert.equal(res.code, 2, `${mode[0]} ${url}: ${res.stderr}`);
      assert.match(res.stderr, /refusing to start/);
      assert.equal(res.stdout, "");
    }
  }
});

test("the Rust server exits 2 on an unknown profile key or a malformed behavior", async (t) => {
  const [cmd, ...args] = RUST_SERVER;
  const cases = [
    [{ nope: true }, /HARNESS_OPTIONS: unknown key "nope"/],
    [{ identify: { returns: null, throws: "x" } }, /HARNESS_OPTIONS: identify: needs exactly one of returns, throws/],
    [{ eventTags: { returns: {}, sleepMs: -1 } }, /HARNESS_OPTIONS: eventTags: "sleepMs" must be a non-negative integer/],
  ];
  for (const [options, message] of cases) {
    const env = rustEnv(t, { AGENTCAT_API_URL: "http://127.0.0.1:9", HARNESS_OPTIONS: JSON.stringify(options) });
    const res = await run(cmd, [...args, "stdio"], { env });
    assert.equal(res.code, 2, `${JSON.stringify(options)}: ${res.stderr}`);
    assert.match(res.stderr, message);
    assert.equal(res.stdout, "");
  }
});

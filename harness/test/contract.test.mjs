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
import { validateRequests } from "../src/contract/validate.mjs";
import { BASIC, HARNESS, TS_SERVER_PLUS, driver, outFile, readJson, run, tempDir, writeJson } from "./support.mjs";

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
function validateCli(t, requests) {
  return run(process.execPath, [VALIDATE, writeJson(join(tempDir(t), "raw.json"), requests)]);
}

test("every publish body of a fresh basic.json TS run validates", async (t) => {
  assert.equal(publishes().length, 11);
  assert.deepEqual(validateRequests(raw), { checked: 11, invalid: [] });
  assert.equal(readJson(outFile("basic", "contract", "stdio", "report")).contract, "ok");
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

test("validate.mjs exits 2 without a readable raw file", async (t) => {
  let res = await run(process.execPath, [VALIDATE]);
  assert.equal(res.code, 2, res.stderr);
  res = await run(process.execPath, [VALIDATE, join(tempDir(t), "missing.json")]);
  assert.equal(res.code, 2, res.stderr);
});

test("the driver exits 1 and records the errors when a publish body is invalid", { timeout: 120_000 }, async (t) => {
  const scenario = writeJson(join(tempDir(t), "one.json"), {
    name: "one",
    projectId: readJson(BASIC).projectId,
    options: {},
    expectPublishes: 2,
    sessions: [{ steps: [{ op: "callTool", name: "echo", args: { msg: "hi" } }] }],
  });
  const env = { FIXTURE_INVALID_PUBLISH: "1" };
  const res = await driver(scenario, "badbody", "stdio", [], { server: TS_SERVER_PLUS, env });
  assert.equal(res.code, 1, res.stderr);
  assert.match(res.stderr, /driver: 1 publish body\/bodies fail the contract/);
  const report = readJson(outFile("one", "badbody", "stdio", "report"));
  assert.equal(report.publishes, 2);
  assert.equal(report.nonPublishRequests, 0);
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

// extract-schema.mjs compiles what it renders with the strict ajv settings of validate.mjs, so a schema.yml
// that the conversion leaves in a form ajv rejects stops the extraction instead of failing every driver run later.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";

import { render } from "../src/contract/extract-schema.mjs";
import { compileStrict } from "../src/contract/validate.mjs";
import { tempDir } from "./support.mjs";

/** A git checkout whose schema.yml holds the publishEvent request body `schema` (a YAML text, indented 12 spaces). */
function checkout(t, schema) {
  const dir = tempDir(t, "extract-schema-");
  const yml = [
    "openapi: 3.0.3",
    "paths:",
    "  /v1/public/events/:",
    "    post:",
    "      operationId: publishEvent",
    "      requestBody:",
    "        content:",
    "          application/json:",
    "            schema:",
    ...schema.map((line) => `              ${line}`),
    "",
  ].join("\n");
  writeFileSync(join(dir, "schema.yml"), yml);
  const env = { ...process.env, GIT_AUTHOR_NAME: "t", GIT_AUTHOR_EMAIL: "t@example.com", GIT_COMMITTER_NAME: "t", GIT_COMMITTER_EMAIL: "t@example.com" };
  for (const args of [["init", "-q"], ["add", "schema.yml"], ["commit", "-q", "-m", "schema"]]) {
    execFileSync("git", ["-C", dir, ...args], { env, stdio: "ignore" });
  }
  return dir;
}

test("render accepts a schema the strict validator compiles, and compileStrict agrees", (t) => {
  const dir = checkout(t, ["type: object", "required: [project_id]", "properties:", "  project_id:", "    type: string", "    format: date-time"]);
  const { files } = render(dir, "HEAD");
  const schema = JSON.parse(files["publish-event.schema.json"]);
  assert.equal(compileStrict(schema)({ project_id: "2026-01-01T00:00:00Z" }), true);
});

test("render throws, naming the schema, when the strict validator cannot compile its output", (t) => {
  // An unknown format is a compile error under strict ajv; the conversion copies "format" as written.
  const dir = checkout(t, ["type: object", "properties:", "  project_id:", "    type: string", "    format: no-such-format"]);
  assert.throws(() => render(dir, "HEAD"), /publish-event\.schema\.json does not compile under the strict ajv settings of validate\.mjs: .*no-such-format/s);
});

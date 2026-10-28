// Generates harness/contract/publish-event.schema.json and harness/contract/SOURCE
// from the private agentcat-server repository (R14: local only, never in CI).
// It reads schema.yml at one commit with `git show <sha>:schema.yml` (no
// checkout, nothing in the repository changes), takes the request-body schema
// of POST /v1/public/events/ (operationId publishEvent), and converts it from
// OpenAPI 3.0 to JSON Schema 2020-12 for ajv (validate.mjs).
//
//   node harness/src/contract/extract-schema.mjs <agentcat-server checkout> <sha or ref>
//       write both files; a ref (e.g. origin/main) is resolved and recorded as its full SHA
//   node harness/src/contract/extract-schema.mjs --check <agentcat-server checkout>
//       regenerate from the SHA in SOURCE into a temp dir; exit 1 on any byte difference
//
// Conversion rules (convert() below). Every other keyword is an error, so a new
// construct in a later schema.yml stops the extraction instead of being dropped:
// - nullable: true adds null: `type: [<t>, "null"]` for a plain typed schema;
//   `anyOf: [{type: "null"}, <schema>]` when the schema has no type or uses
//   $ref/allOf/oneOf/anyOf/not. nullable with an enum that lacks null is an error.
// - $ref (local "#/..." only) is inlined; a recursive $ref is an error. Siblings of
//   $ref are ignored as OpenAPI 3.0 says, except nullable (above); a sibling that
//   is not an annotation is an error.
// - exclusiveMinimum/exclusiveMaximum: true become the 2020-12 numeric form.
// - example becomes examples: [example]; x-* extensions are dropped.
// - required, additionalProperties and every other validation keyword are copied
//   exactly as written: nothing is loosened or tightened.
// The result must compile under the strict ajv settings of validate.mjs (compileStrict); if it does
// not, the extraction fails and writes nothing.
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { isDeepStrictEqual } from "node:util";

import YAML from "yaml";

import { compileStrict } from "./validate.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const CONTRACT_DIR = join(HERE, "..", "..", "contract");
const SCHEMA_FILE = "publish-event.schema.json";
const SOURCE_FILE = "SOURCE";
/** Where the publishEvent request-body schema sits in schema.yml. */
export const SOURCE_POINTER = ["paths", "/v1/public/events/", "post", "requestBody", "content", "application/json", "schema"];
const USAGE =
  "usage: node harness/src/contract/extract-schema.mjs <agentcat-server checkout> <sha or ref>\n" +
  "       node harness/src/contract/extract-schema.mjs --check <agentcat-server checkout>";

// Keywords with the same meaning in OpenAPI 3.0 and JSON Schema 2020-12, copied as written.
const COPY = new Set([
  "type",
  "enum",
  "required",
  "format",
  "pattern",
  "multipleOf",
  "maximum",
  "minimum",
  "maxLength",
  "minLength",
  "maxItems",
  "minItems",
  "uniqueItems",
  "maxProperties",
  "minProperties",
  "title",
  "description",
  "default",
  "deprecated",
]);
// Keywords whose values are schemas or lists or maps of schemas.
const SUBSCHEMA = new Set(["properties", "items", "additionalProperties", "allOf", "oneOf", "anyOf", "not"]);
const COMPOSITION = ["$ref", "allOf", "oneOf", "anyOf", "not"];
// Siblings of $ref that carry no validation (ignored, as OpenAPI 3.0 ignores every $ref sibling).
const REF_ANNOTATIONS = new Set(["description", "title", "example", "deprecated"]);
const TYPES = new Set(["string", "number", "integer", "boolean", "array", "object"]);

const isObject = (v) => v !== null && typeof v === "object" && !Array.isArray(v);

/**
 * Converts one OpenAPI 3.0 Schema Object to JSON Schema 2020-12, resolving
 * local $refs against `doc` (the whole OpenAPI document). Throws on anything it
 * cannot convert faithfully; `path` names the place in the error.
 */
export function convert(schema, doc, path = "#", refStack = []) {
  if (!isObject(schema)) throw new Error(`${path}: a schema must be an object, got ${JSON.stringify(schema)}`);

  if ("$ref" in schema) {
    const ref = schema.$ref;
    for (const key of Object.keys(schema)) {
      if (key === "$ref" || key === "nullable" || REF_ANNOTATIONS.has(key) || key.startsWith("x-")) continue;
      throw new Error(`${path}: unsupported sibling ${JSON.stringify(key)} next to $ref`);
    }
    if (typeof ref !== "string" || !ref.startsWith("#/")) throw new Error(`${path}: only local $refs ("#/...") are supported, got ${JSON.stringify(ref)}`);
    if (refStack.includes(ref)) throw new Error(`${path}: recursive $ref ${ref} cannot be inlined`);
    const target = resolvePointer(doc, ref);
    if (target === undefined) throw new Error(`${path}: $ref ${ref} does not resolve`);
    const inlined = convert(target, doc, ref, [...refStack, ref]);
    return schema.nullable === true ? { anyOf: [{ type: "null" }, inlined] } : inlined;
  }

  const out = {};
  for (const [key, value] of Object.entries(schema)) {
    const at = `${path}/${key}`;
    if (key === "nullable") {
      if (typeof value !== "boolean") throw new Error(`${at}: nullable must be a boolean`);
    } else if (key === "type") {
      if (!TYPES.has(value)) throw new Error(`${at}: unsupported type ${JSON.stringify(value)} (OpenAPI 3.0 types are single strings)`);
      out.type = value;
    } else if (COPY.has(key)) {
      out[key] = structuredClone(value);
    } else if (key === "exclusiveMinimum" || key === "exclusiveMaximum") {
      if (typeof value !== "boolean") throw new Error(`${at}: must be a boolean in OpenAPI 3.0`);
      const bound = key === "exclusiveMinimum" ? "minimum" : "maximum";
      if (value && typeof schema[bound] !== "number") throw new Error(`${at}: ${key} without ${bound}`);
      // Applied after the loop: the bound moves into the exclusive keyword.
    } else if (key === "example") {
      out.examples = [structuredClone(value)];
    } else if (SUBSCHEMA.has(key)) {
      out[key] = convertSubschemas(key, value, doc, at, refStack);
    } else if (key.startsWith("x-")) {
      // Vendor extension: no validation meaning.
    } else {
      throw new Error(`${at}: unsupported keyword ${JSON.stringify(key)}`);
    }
  }
  for (const [flag, bound] of [
    ["exclusiveMinimum", "minimum"],
    ["exclusiveMaximum", "maximum"],
  ]) {
    if (schema[flag] === true) {
      out[flag] = out[bound];
      delete out[bound];
    }
  }

  if (schema.nullable !== true) return out;
  if (Array.isArray(schema.enum) && !schema.enum.includes(null)) {
    throw new Error(`${path}: nullable with an enum without null is ambiguous in OpenAPI 3.0`);
  }
  if (typeof schema.type === "string" && !COMPOSITION.some((k) => k in schema)) {
    return Object.fromEntries(Object.entries(out).map(([k, v]) => [k, k === "type" ? [v, "null"] : v]));
  }
  return { anyOf: [{ type: "null" }, out] };
}

function convertSubschemas(key, value, doc, at, refStack) {
  switch (key) {
    case "properties":
      if (!isObject(value)) throw new Error(`${at}: must be an object`);
      return Object.fromEntries(Object.entries(value).map(([name, s]) => [name, convert(s, doc, `${at}/${name}`, refStack)]));
    case "allOf":
    case "oneOf":
    case "anyOf":
      if (!Array.isArray(value) || value.length === 0) throw new Error(`${at}: must be a non-empty array`);
      return value.map((s, i) => convert(s, doc, `${at}/${i}`, refStack));
    case "additionalProperties":
      return typeof value === "boolean" ? value : convert(value, doc, at, refStack);
    default: // items, not
      return convert(value, doc, at, refStack);
  }
}

/** Resolves a local JSON pointer ("#/a/b~1c") in `doc`; undefined when it does not resolve. */
function resolvePointer(doc, ref) {
  let node = doc;
  for (const raw of ref.slice(2).split("/")) {
    const key = raw.replaceAll("~1", "/").replaceAll("~0", "~");
    if (!isObject(node) || !Object.hasOwn(node, key)) return undefined;
    node = node[key];
  }
  return node;
}

/** Runs git read-only in `dir`, with the caller's GIT_* variables removed. */
function git(dir, args) {
  const env = Object.fromEntries(Object.entries(process.env).filter(([k]) => !k.startsWith("GIT_")));
  try {
    return execFileSync("git", ["--no-optional-locks", "-C", dir, ...args], {
      env,
      encoding: "utf8",
      maxBuffer: 64 * 1024 * 1024,
      stdio: ["ignore", "pipe", "pipe"],
    });
  } catch (err) {
    throw new Error(`git -C ${dir} ${args.join(" ")} failed: ${String(err.stderr ?? err.message).trim()}`, { cause: err });
  }
}

/** The two generated files' texts for one agentcat-server commit. */
export function render(checkout, rev) {
  const sha = git(checkout, ["rev-parse", "--verify", "--end-of-options", `${rev}^{commit}`]).trim();
  if (!/^[0-9a-f]{40}$/.test(sha)) throw new Error(`git rev-parse gave ${JSON.stringify(sha)} for ${rev}`);
  const text = git(checkout, ["show", `${sha}:schema.yml`]);
  // drf-spectacular writes schema.yml with PyYAML (YAML 1.1 rules). Read it both
  // ways and require the same result, so no scalar changes meaning between the two.
  const schemas = ["1.2", "1.1"].map((version) => {
    const doc = YAML.parse(text, { version, uniqueKeys: true });
    const source = SOURCE_POINTER.reduce((node, key) => (isObject(node) && Object.hasOwn(node, key) ? node[key] : undefined), doc);
    if (source === undefined) throw new Error(`schema.yml at ${sha} has no ${SOURCE_POINTER.join(" > ")}`);
    return convert(source, doc, `schema.yml#/${SOURCE_POINTER.join(" > ")}`);
  });
  if (!isDeepStrictEqual(schemas[0], schemas[1])) throw new Error("schema.yml reads differently as YAML 1.1 and YAML 1.2");
  const schema = {
    $schema: "https://json-schema.org/draft/2020-12/schema",
    $comment:
      "Generated by harness/src/contract/extract-schema.mjs from the publishEvent request body " +
      "(paths./v1/public/events/.post.requestBody.content.application/json.schema) of agentcat-server " +
      "schema.yml at the commit in SOURCE. Do not edit.",
    ...schemas[0],
  };
  // The validator compiles this schema with strict ajv on every run: a failure there must stop the extraction.
  try {
    compileStrict(schema);
  } catch (err) {
    throw new Error(`${SCHEMA_FILE} does not compile under the strict ajv settings of validate.mjs: ${err.message}`, { cause: err });
  }
  return {
    sha,
    files: {
      [SCHEMA_FILE]: `${JSON.stringify(schema, null, 2)}\n`,
      [SOURCE_FILE]: `agentcat-server ${sha} schema.yml\n`,
    },
  };
}

function writeFiles(dir, files) {
  mkdirSync(dir, { recursive: true });
  for (const [name, text] of Object.entries(files)) writeFileSync(join(dir, name), text);
}

function check(checkout) {
  const source = readFileSync(join(CONTRACT_DIR, SOURCE_FILE), "utf8");
  const m = source.match(/^agentcat-server ([0-9a-f]{40}) schema\.yml\n$/);
  if (!m) throw new Error(`harness/contract/${SOURCE_FILE} is malformed: ${JSON.stringify(source)}`);
  const tmp = mkdtempSync(join(tmpdir(), "harness-publish-contract-"));
  try {
    writeFiles(tmp, render(checkout, m[1]).files);
    const stale = [SCHEMA_FILE, SOURCE_FILE].filter((name) => {
      let have;
      try {
        have = readFileSync(join(CONTRACT_DIR, name));
      } catch {
        return true;
      }
      return !readFileSync(join(tmp, name)).equals(have);
    });
    if (stale.length > 0) {
      process.stderr.write(
        `harness/contract/{${stale.join(",")}} out of date: run node harness/src/contract/extract-schema.mjs <checkout> ${m[1]}\n`,
      );
      return 1;
    }
    process.stdout.write(`harness/contract/${SCHEMA_FILE} up to date (agentcat-server ${m[1]})\n`);
    return 0;
  } finally {
    rmSync(tmp, { recursive: true, force: true });
  }
}

function main(args) {
  if (args[0] === "--check") {
    if (args.length !== 2) throw Object.assign(new Error(USAGE), { usage: true });
    return check(args[1]);
  }
  if (args.length !== 2 || args.some((a) => a.startsWith("--"))) throw Object.assign(new Error(USAGE), { usage: true });
  const { sha, files } = render(args[0], args[1]);
  writeFiles(CONTRACT_DIR, files);
  process.stdout.write(`publishEvent request schema of agentcat-server ${sha} -> ${join(CONTRACT_DIR, SCHEMA_FILE)}\n`);
  return 0;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    process.exitCode = main(process.argv.slice(2));
  } catch (err) {
    process.stderr.write(`${err.usage ? err.message : `extract-schema: ${err?.message ?? err}`}\n`);
    process.exitCode = err.usage ? 2 : 1;
  }
}

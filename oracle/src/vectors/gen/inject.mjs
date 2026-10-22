// Vectors for the tool-list pipeline of the TS SDK: buildInjectedList (src/engine/listWrap.ts)
// with handle injection (src/modules/handle-injection.ts), context injection
// (src/modules/context-parameters.ts) and the get_more_tools descriptor (src/modules/tools.ts).
//
// Every case: input {options, hookMode, pages}; output oracle.buildInjectedList(input)
// (functions.mjs): one {tools, registry, outputRegistry, declaredSessionParams, logs} per page,
// the pages listed in turn on one tracking-data object. `options` holds the four switches
// enableReportMissing, enableTracing, enableToolCallContext and enableAgentTracking, and
// customContextDescription when the case sets one; hookMode adds a resolveSessionId hook.
//
// The tools of a case are what rmcp's Tool holds (name, title, description, inputSchema,
// outputSchema, annotations, _meta), so they survive a round trip through it. A tool it cannot
// hold (no object inputSchema, a non-string name, a non-object outputSchema) is alone in an
// "rd20:" case, which carries "deviation": "RD-20": such a listing entry cannot reach the SDK.
//
// Case name prefixes:
//   ts:<file>:<label>       the listing-side inputs of the TS tests, each with its ts_test
//   flags:<label>           every combination of the switches and hook mode; the descriptions "" and "why"
//   schema:<label>          input-schema shapes
//   output:<label>          output-schema shapes
//   report-missing:<label>  get_more_tools: appended, customer-owned (B-028), off
//   pages:<label>           several pages: which log lines repeat and which TS logs once
//   rd20:<label>            one tool rmcp's Tool cannot hold
//   seeded:<n>              random listings of the shapes above, from seeded(8)
import { functions } from "../../functions.mjs";
import { int, pick, seeded, tsKey } from "../support.mjs";

const build = functions["oracle.buildInjectedList"];

/** The options track() fills in (src/index.ts), changed by `overrides`. */
function trackOptions(overrides = {}) {
  return {
    enableReportMissing: true,
    enableTracing: true,
    enableToolCallContext: true,
    enableAgentTracking: false,
    ...overrides,
  };
}

/**
 * The input of an addHandleParametersToTools test (`injectSessionId`, `injectAgentId`) as
 * buildInjectedList options: session_id needs tracing without hook mode, agent_id tracing and agent
 * tracking. No get_more_tools, and context only when `context` is set.
 */
function handleFlags(injectSessionId, injectAgentId, context = false) {
  const tracing = injectSessionId || injectAgentId;
  return {
    options: trackOptions({
      enableReportMissing: false,
      enableTracing: tracing,
      enableToolCallContext: context,
      enableAgentTracking: injectAgentId,
    }),
    hookMode: tracing && !injectSessionId,
  };
}

/** The input of an addContextParameterToTools test: context injection alone. */
const CONTEXT_ONLY = {
  options: trackOptions({ enableReportMissing: false, enableTracing: false }),
  hookMode: false,
};

/** Every injection on, no get_more_tools: the shape cases. */
const ALL_ON = {
  options: trackOptions({ enableReportMissing: false, enableAgentTracking: true }),
  hookMode: false,
};

const cases = [];

/** Adds the case `name` for `pages` under `flags` ({options, hookMode}), with `fields` after the output. */
async function add(name, flags, pages, fields = {}) {
  const input = { options: flags.options, hookMode: flags.hookMode, pages };
  cases.push({ name, input, output: await build(input), ...fields });
}

/** Adds a case for one page with the tools `tools`. */
const one = (name, flags, tools, fields) => add(name, flags, [tools], fields);

/** Adds an "rd20:" case: the one tool `tool`, which rmcp's Tool cannot hold. */
const rd20 = (name, flags, tool, tsTest) =>
  one(`rd20:${name}`, flags, [tool], { deviation: "RD-20", ...(tsTest && { ts_test: tsTest }) });

// The fixtures of handle-injection.test.ts.
const customerSchema = () => ({
  type: "object",
  properties: { text: { type: "string" } },
  required: ["text"],
});
const objectOutputSchema = () => ({
  type: "object",
  properties: { count: { type: "number" } },
  required: ["count"],
  additionalProperties: false,
});
const structuredTool = (name, outputSchema) => ({ name, inputSchema: customerSchema(), outputSchema });

// The listing of the TS tests' todo server (src/tests/test-utils/client-server-factory.ts), as
// the MCP SDK 1.31 lists it, without the `execution` key that rmcp's Tool does not have (RD-13).
const DRAFT_07 = "http://json-schema.org/draft-07/schema#";
const TODO_SERVER = [
  {
    name: "add_todo",
    description: "Add a new todo item",
    inputSchema: {
      type: "object",
      properties: { text: { type: "string", description: "The text of the todo item" } },
      required: ["text"],
      additionalProperties: false,
      $schema: DRAFT_07,
    },
  },
  {
    name: "list_todos",
    description: "List all todo items",
    inputSchema: { type: "object", properties: {}, $schema: DRAFT_07 },
  },
  {
    name: "complete_todo",
    description: "Mark a todo item as completed",
    inputSchema: {
      type: "object",
      properties: { id: { type: "string", description: "The ID of the todo to complete" } },
      required: ["id"],
      additionalProperties: false,
      $schema: DRAFT_07,
    },
  },
  {
    name: "get_stats",
    description: "Get todo stats",
    inputSchema: { type: "object", properties: {}, $schema: DRAFT_07 },
    outputSchema: {
      type: "object",
      properties: { count: { type: "number", description: "Number of todos" } },
      required: ["count"],
      additionalProperties: false,
      $schema: DRAFT_07,
    },
  },
];

async function handleInjectionCases() {
  const k = (title) => [tsKey("handle-injection", title)];
  const H = "addHandleParametersToTools > ";
  const O = "outputSchema injection > ";
  const f = "ts:handle-injection:";
  await one(`${f}after-customer-params`, handleFlags(true, true), [{ name: "add_todo", inputSchema: customerSchema() }], {
    ts_test: k(`${H}injects session_id and agent_id after customer params, both required`),
  });
  await one(`${f}creates-required`, handleFlags(true, true), [{ name: "bare", inputSchema: { type: "object", properties: {} } }], {
    ts_test: k(`${H}creates the required array when the schema has none`),
  });
  const weirdSession = { name: "weird_session", inputSchema: { type: "object", properties: {}, required: ["session_id"] } };
  await one(`${f}required-holds-session_id`, handleFlags(true, false), [weirdSession], {
    ts_test: k(`${H}does not duplicate session_id in an existing required array`),
  });
  const weird = { name: "weird", inputSchema: { type: "object", properties: {}, required: ["agent_id"] } };
  await one(`${f}required-holds-agent_id`, handleFlags(false, true), [weird], {
    ts_test: k(`${H}does not duplicate agent_id in an existing required array`),
  });
  const ownsAgent = {
    name: "owns_agent",
    inputSchema: { type: "object", properties: { agent_id: { type: "string", description: "mine" } }, required: [] },
  };
  await one(`${f}owns-agent_id`, handleFlags(false, true), [ownsAgent], {
    ts_test: k(`${H}leaves required untouched when the customer declares agent_id`),
  });
  await one(`${f}full-order`, handleFlags(true, true, true), [{ name: "add_todo", inputSchema: customerSchema() }], {
    ts_test: k(`${H}full order with context injector: customer, session_id, agent_id, context`),
  });
  await one(`${f}hook-mode`, handleFlags(false, true), [{ name: "add_todo", inputSchema: customerSchema() }], {
    ts_test: k(`${H}hook mode: no session_id, agent_id uses the same single description`),
  });
  const deploy = customerSchema();
  deploy.properties.session_id = { type: "string", description: "customer's own" };
  await one(`${f}owns-session_id`, handleFlags(true, true), [{ name: "deploy", inputSchema: deploy }], {
    ts_test: k(`${H}skips a param the tool already defines, and does not record it`),
  });
  await one(`${f}complex`, handleFlags(true, true), [{ name: "odd", inputSchema: { oneOf: [{ type: "object" }] } }], {
    ts_test: k(`${H}skips complex schemas entirely`),
  });
  // The test lists `strict` with `bare`, a tool without an inputSchema, which rmcp cannot hold.
  const strictAndBare = k(`${H}removes additionalProperties:false and handles missing schemas`);
  await one(`${f}strict`, handleFlags(true, true), [{ name: "strict", inputSchema: { ...customerSchema(), additionalProperties: false } }], {
    ts_test: strictAndBare,
  });
  await rd20("ts:handle-injection:bare", handleFlags(true, true), { name: "bare" }, strictAndBare);
  const gmt = {
    name: "get_more_tools",
    inputSchema: { type: "object", properties: { context: { type: "string", description: "bespoke" } }, required: ["context"] },
  };
  await one(`${f}get_more_tools`, handleFlags(true, true, true), [gmt], {
    ts_test: k(`${H}does NOT skip get_more_tools (handles are injected; context stays bespoke)`),
  });
  await one(`${f}output-object`, handleFlags(true, true), [structuredTool("get_stats", objectOutputSchema())], {
    ts_test: k(`${O}injects mcp_session into a plain object outputSchema and registers the tool`),
  });
  const modes = k(`${O}sub-properties track modes: hook mode drops session_id and status, tracking off drops agent_id`);
  await one(`${f}output-hook-mode`, handleFlags(false, true), [structuredTool("t", objectOutputSchema())], { ts_test: modes });
  await one(`${f}output-tracking-off`, handleFlags(true, false), [structuredTool("t", objectOutputSchema())], { ts_test: modes });
  const complex = { oneOf: [{ type: "object", properties: { a: { type: "string" } } }] };
  await one(`${f}output-complex`, handleFlags(true, true), [structuredTool("poly", complex)], {
    ts_test: k(`${O}skips complex outputSchema (oneOf/allOf/anyOf) and does not register`),
  });
  await one(`${f}output-input-composed`, handleFlags(true, true), [{ ...structuredTool("mixed", objectOutputSchema()), inputSchema: complex }], {
    ts_test: k(`${O}composed inputSchema: early-return skips output injection too, nothing registered`),
  });
  const claims = { type: "object", properties: { mcp_session: { type: "string" } } };
  await one(`${f}output-collision`, handleFlags(true, true), [structuredTool("t", claims)], {
    ts_test: k(`${O}collision: a customer-declared mcp_session is never clobbered`),
  });
  await one(`${f}output-absent`, handleFlags(true, true), [{ name: "t", inputSchema: customerSchema() }], {
    ts_test: k(`${O}no outputSchema: untouched, nothing registered`),
  });
  await one(`${f}output-no-handles`, handleFlags(false, false), [structuredTool("t", objectOutputSchema())], {
    ts_test: k(`${O}no injection at all (neither handle) leaves outputSchema alone`),
  });
}

async function contextCases() {
  const k = (title) => [tsKey("context-parameters", `Context Parameters > ${title}`)];
  const A = "addContextParameterToTools > ";
  const f = "ts:context-parameters:";
  await rd20("ts:context-parameters:no-input-schema", CONTEXT_ONLY, { name: "simple_tool", description: "A simple tool" }, k(
    `${A}should add context parameter to tools without inputSchema`,
  ));
  const existing = {
    name: "existing_tool",
    description: "A tool with existing schema",
    inputSchema: {
      type: "object",
      properties: { text: { type: "string", description: "Some text" } },
      required: ["text"],
    },
  };
  await one(`${f}existing-schema`, CONTEXT_ONLY, [existing], {
    ts_test: k(`${A}should add context parameter to tools with existing inputSchema`),
  });
  const withContext = {
    name: "tool_with_context",
    description: "A tool that already has context",
    inputSchema: {
      type: "object",
      properties: { context: { type: "string", description: "Existing context" } },
      required: ["context"],
    },
  };
  await one(`${f}owns-context`, CONTEXT_ONLY, [withContext], {
    ts_test: k(`${A}should not duplicate context parameter if already exists`),
  });
  const optional = {
    name: "optional_tool",
    description: "A tool with no required fields",
    inputSchema: { type: "object", properties: { optional: { type: "string" } }, required: [] },
  };
  await one(`${f}empty-required`, CONTEXT_ONLY, [optional], {
    ts_test: k(`${A}should handle tools with empty required array`),
  });

  // The listing tests of context-parameters.test.ts and custom-context-description.test.ts track
  // the todo server; track() fills in the other options.
  const I = "Integration with MCP server tracking > ";
  const c = (title) => tsKey("custom-context-description", `Custom Context Description > ${title}`);
  await one("ts:todo-server:default-description", { options: trackOptions(), hookMode: false }, TODO_SERVER, {
    ts_test: [
      ...k(`${I}should inject context into tool schemas when listing tools`),
      ...k(`${I}should use default context description when no custom description is provided`),
      c("should use default description when customContextDescription is not provided"),
    ],
  });
  const custom = [
    ["json-schema", "Explain your reasoning for this action", "should use custom description instead of default for JSON Schema tools"],
    ["zod-object", "Provide context about why you're doing this", "should use custom description for Zod object schemas"],
    ["all-tools", "Why are you calling this tool?", "should apply custom description to all tools when listing"],
    [
      "long",
      "Please provide a comprehensive explanation of your reasoning, including the broader context of this action within your workflow, the expected outcomes, and how this contributes to your overall objectives. Be as detailed as possible.",
      "should use custom description even with long descriptions",
    ],
    ["special-characters", 'Why? (explain in detail) - "Be specific!"', "should use custom description with special characters"],
  ];
  for (const [label, description, title] of custom) {
    const flags = { options: trackOptions({ customContextDescription: description }), hookMode: false };
    await one(`ts:todo-server:custom-${label}`, flags, TODO_SERVER, { ts_test: [c(title)] });
  }
}

async function contextEdgeCases() {
  const k = (title) => [tsKey("context-parameters-edge-cases", `Context Parameters Edge Cases > ${title}`)];
  const f = "ts:context-parameters-edge-cases:";
  const C = "existing context parameter collision > ";
  await one(`${f}owns-context`, CONTEXT_ONLY, [{
    name: "test-tool",
    inputSchema: { type: "object", properties: { context: { type: "number", description: "existing" } } },
  }], { ts_test: k(`${C}should skip injection for tools with existing context parameter`) });
  await one(`${f}warns-with-name`, CONTEXT_ONLY, [{
    name: "my-special-tool",
    inputSchema: { type: "object", properties: { context: { type: "boolean" } } },
  }], { ts_test: k(`${C}should warn with tool name when context exists`) });
  const X = "complex schema handling > ";
  await one(`${f}one-of`, CONTEXT_ONLY, [{
    name: "union-tool",
    inputSchema: {
      oneOf: [
        { type: "object", properties: { email: { type: "string" } } },
        { type: "object", properties: { phone: { type: "string" } } },
      ],
    },
  }], { ts_test: k(`${X}should skip injection for oneOf schemas`) });
  await one(`${f}all-of`, CONTEXT_ONLY, [{
    name: "intersection-tool",
    inputSchema: {
      allOf: [
        { type: "object", properties: { a: { type: "string" } } },
        { type: "object", properties: { b: { type: "string" } } },
      ],
    },
  }], { ts_test: k(`${X}should skip injection for allOf schemas`) });
  await one(`${f}any-of`, CONTEXT_ONLY, [{ name: "anyof-tool", inputSchema: { anyOf: [{ type: "string" }, { type: "number" }] } }], {
    ts_test: k(`${X}should skip injection for anyOf schemas`),
  });
  const P = "additionalProperties constraint > ";
  const strict = {
    name: "strict-tool",
    inputSchema: { type: "object", properties: { name: { type: "string" } }, additionalProperties: false },
  };
  await one(`${f}additional-false`, CONTEXT_ONLY, [strict], {
    ts_test: k(`${P}should inject context and remove additionalProperties:false constraint`),
  });
  await one(`${f}additional-true`, CONTEXT_ONLY, [{
    name: "flexible-tool",
    inputSchema: { type: "object", properties: { name: { type: "string" } }, additionalProperties: true },
  }], { ts_test: k(`${P}should preserve additionalProperties:true when injecting`) });
  const E = "edge case schemas > ";
  await one(`${f}empty-schema`, CONTEXT_ONLY, [{ name: "empty-tool", inputSchema: {} }], {
    ts_test: k(`${E}should handle empty object schema {}`),
  });
  await rd20("ts:context-parameters-edge-cases:no-input-schema", CONTEXT_ONLY, { name: "no-schema-tool" }, k(
    `${E}should handle schema with no inputSchema`,
  ));
  await one(`${f}type-only`, CONTEXT_ONLY, [{ name: "type-only-tool", inputSchema: { type: "object" } }], {
    ts_test: k(`${E}should handle schema with only type specified`),
  });
  const B = "addContextParameterToTools batch handling > ";
  await one(`${f}skips-get_more_tools`, CONTEXT_ONLY, [
    { name: "get_more_tools", inputSchema: {} },
    { name: "other-tool", inputSchema: {} },
  ], { ts_test: k(`${B}should skip get_more_tools entirely`) });
  await one(`${f}mixed`, CONTEXT_ONLY, [
    { name: "valid-tool", inputSchema: { type: "object", properties: {} } },
    { name: "complex-tool", inputSchema: { oneOf: [{ type: "string" }] } },
    { name: "collision-tool", inputSchema: { type: "object", properties: { context: { type: "number" } } } },
    strict,
  ], { ts_test: k(`${B}should handle mixed valid and complex schemas`) });
}

async function conflictCases() {
  const k = (title) => tsKey("handle-conflict-report", title);
  const R = "session_id collision report > ";
  const D = "customer-declared session_id tracking > ";
  const f = "ts:handle-conflict-report:";
  // data() of the test file.
  const data = (overrides = {}) => ({
    options: trackOptions({ enableReportMissing: false, enableToolCallContext: false, ...overrides }),
    hookMode: false,
  });
  const ownSession = () => ({ name: "own_session", inputSchema: { type: "object", properties: { session_id: { type: "string" } } } });
  const dedupe = k(`${R}buildInjectedList dedupes per AgentCatData object: repeat tools/list on the same server logs once, a second server logs its own once`);
  // One case is one tracking-data object: server A lists three times, server B (a fresh object) twice.
  await add(`${f}server-a-three-listings`, data(), [[ownSession()], [ownSession()], [ownSession()]], {
    ts_test: [k(`${R}reports a session_id collision once per tool, as an error with remediation`), dedupe],
  });
  await add(`${f}server-b-two-listings`, data(), [[ownSession()], [ownSession()]], { ts_test: [dedupe] });
  const own = (name) => ({ name, inputSchema: { type: "object", properties: { session_id: { type: "string" } } } });
  await add(`${f}two-tools-two-listings`, data(), [[own("tool_a"), own("tool_b")], [own("tool_a"), own("tool_b")]], {
    ts_test: [k(`${R}reports two different colliding tools each once`)],
  });
  const composedSchema = () => ({
    type: "object",
    oneOf: [{ properties: { kind: { const: "a" } } }, { properties: { kind: { const: "b" } } }],
  });
  const plain = { name: "normal", inputSchema: { type: "object", properties: { q: { type: "string" } } } };
  await one(`${f}records-own-schemas-only`, data(), [ownSession(), { name: "union_tool", inputSchema: composedSchema() }, plain], {
    ts_test: [k(`${D}records only the tools whose own schema declares session_id`)],
  });
  await one(`${f}records-composed-with-session_id`, data(), [{
    name: "union_with_own_session",
    inputSchema: { ...composedSchema(), properties: { session_id: { type: "string" } } },
  }], { ts_test: [k(`${D}records a composed schema that also declares session_id at its root`)] });
  await one(`${f}keys-per-data-object`, data(), [ownSession()], {
    ts_test: [k(`${D}keys per AgentCatData object, like reportedConflicts`)],
  });
  await one(`${f}hook-mode-records-nothing`, { ...data(), hookMode: true }, [ownSession()], {
    ts_test: [k(`${D}records nothing in hook mode, where session_id is never injected`)],
  });
}

async function schemaEdgeCases() {
  const k = (title) => [tsKey("v2/schema-edges", title)];
  const I = "v2 schema edges: input injection > ";
  const O = "v2 schema edges: outputSchema injection > ";
  const f = "ts:v2/schema-edges:";
  const flags = (overrides = {}) => ({ options: trackOptions({ enableAgentTracking: true, ...overrides }), hookMode: false });
  const composed = {
    type: "object",
    oneOf: [
      { type: "object", properties: { a: { type: "string" } } },
      { type: "object", properties: { b: { type: "string" } } },
    ],
  };
  await one(`${f}composed-input`, flags(), [{ name: "composed", description: "composed", inputSchema: composed }], {
    ts_test: k(`${I}skips handle injection entirely on composed (oneOf) input schemas`),
  });
  await rd20("ts:v2/schema-edges:bare", flags(), { name: "bare", description: "no schema" }, k(
    `${I}builds a default object schema for a tool listed without one`,
  ));
  await one(`${f}own-handles`, flags(), [{
    name: "own_handles",
    description: "declares its own",
    inputSchema: {
      type: "object",
      properties: {
        session_id: { type: "string", description: "customer session id" },
        agent_id: { type: "string", description: "customer agent id" },
      },
    },
  }], { ts_test: k(`${I}preserves a customer-declared session_id/agent_id instead of overwriting`) });
  await one(`${f}strict`, flags(), [{
    name: "strict",
    description: "strict",
    inputSchema: { type: "object", properties: { text: { type: "string" } }, additionalProperties: false },
  }], { ts_test: k(`${I}drops additionalProperties:false so optional handles can ride along`) });
  const stats = (outputSchema) => ({
    name: "stats",
    description: "returns stats",
    inputSchema: { type: "object", properties: {} },
    outputSchema,
  });
  await one(`${f}output-object`, flags(), [stats({
    type: "object",
    properties: { count: { type: "number" } },
    required: ["count"],
    additionalProperties: false,
  })], { ts_test: k(`${O}declares mcp_session in a listed plain-object outputSchema`) });
  await one(`${f}output-tracking-off`, flags({ enableAgentTracking: false }), [stats({
    type: "object",
    properties: { count: { type: "number" } },
  })], { ts_test: k(`${O}omits agent_id from mcp_session when agent tracking is off`) });
  await one(`${f}output-composed`, flags(), [{
    name: "either",
    description: "composed output",
    inputSchema: { type: "object", properties: {} },
    outputSchema: {
      anyOf: [
        { type: "object", properties: { ok: { type: "boolean" } } },
        { type: "object", properties: { err: { type: "string" } } },
      ],
    },
  }], { ts_test: k(`${O}leaves composed (anyOf) outputSchemas untouched`) });
  await one(`${f}output-collision`, flags(), [{
    name: "claims_key",
    description: "already declares the key",
    inputSchema: { type: "object", properties: {} },
    outputSchema: { type: "object", properties: { mcp_session: { type: "string", description: "customer-owned field" } } },
  }], { ts_test: k(`${O}never overwrites a customer-declared mcp_session output field`) });
}

async function listWrapCases() {
  const k = (title) => [tsKey("engine-listwrap", `buildInjectedList > ${title}`)];
  const f = "ts:engine-listwrap:";
  const data = (overrides = {}) => ({ options: trackOptions(overrides), hookMode: false });
  const echo = {
    name: "echo",
    description: "Echo",
    inputSchema: { type: "object", properties: { msg: { type: "string" } }, required: ["msg"] },
  };
  await one(`${f}appends-get_more_tools`, data(), [echo], { ts_test: k("appends get_more_tools and injects session_id + context") });
  await one(`${f}report-missing-off`, data({ enableReportMissing: false }), [echo], {
    ts_test: k("skips get_more_tools when enableReportMissing is false"),
  });
  await one(`${f}tracing-off`, data({ enableTracing: false, enableToolCallContext: false }), [echo], {
    ts_test: k("skips handle injection entirely when tracing is off, but still registers empty entries"),
  });
  // The fault-containment test lists a customer-owned get_more_tools twice and counts the shadow warning.
  const own = { name: "get_more_tools", description: "customer version", inputSchema: { type: "object", properties: {} } };
  await add(`${f}shadow-once`, data(), [[own, echo], [own, echo]], {
    ts_test: [tsKey("engine-listwrap", "tools/list fault containment > warns once when the customer owns get_more_tools while report-missing is enabled")],
  });
}

// A listing for the flag cases: a plain tool with an output schema and a composed one.
const FLAG_TOOLS = [
  {
    name: "search",
    description: "Search",
    inputSchema: { type: "object", properties: { q: { type: "string" } }, required: ["q"], additionalProperties: false },
    outputSchema: { type: "object", properties: { hits: { type: "number" } } },
  },
  { name: "union", inputSchema: { type: "object", anyOf: [{ required: ["a"] }, { required: ["b"] }] } },
];

async function flagCases() {
  for (const reportMissing of [false, true]) {
    for (const tracing of [false, true]) {
      for (const hookMode of [false, true]) {
        for (const agentTracking of [false, true]) {
          for (const context of [false, true]) {
            const label = [
              reportMissing ? "report" : "no-report",
              tracing ? "tracing" : "no-tracing",
              hookMode ? "hook" : "no-hook",
              agentTracking ? "agent" : "no-agent",
              context ? "context" : "no-context",
            ].join("-");
            const options = {
              enableReportMissing: reportMissing,
              enableTracing: tracing,
              enableToolCallContext: context,
              enableAgentTracking: agentTracking,
            };
            await one(`flags:${label}`, { options, hookMode }, FLAG_TOOLS);
            // The context descriptions "" (the default) and "why", where context is on, without
            // get_more_tools and agent tracking (they do not touch the description).
            if (!context || reportMissing || agentTracking) continue;
            for (const [descLabel, description] of [["empty", ""], ["why", "why"]]) {
              await one(`flags:${label}-description-${descLabel}`, {
                options: { ...options, customContextDescription: description },
                hookMode,
              }, FLAG_TOOLS);
            }
          }
        }
      }
    }
  }
}

async function schemaCases() {
  const s = (name, inputSchema, flags = ALL_ON) => one(`schema:${name}`, flags, [{ name: "t", inputSchema }]);
  const text = { text: { type: "string" } };
  await s("no-properties", { type: "object" });
  await s("empty", {});
  for (const [label, value] of [["null", null], ["false", false], ["zero", 0], ["empty-string", ""]]) {
    await s(`properties-${label}`, { type: "object", properties: value, required: ["text"] });
  }
  await s("required-absent", { type: "object", properties: text });
  await s("required-present", { type: "object", properties: text, required: ["text"] });
  await s("required-holds-session_id", { type: "object", properties: text, required: ["session_id"] });
  await s("required-holds-all", { type: "object", properties: text, required: ["context", "agent_id", "session_id"] });
  await s("required-string", { type: "object", properties: text, required: "text" });
  await s("required-null", { required: null, type: "object", properties: text });
  await s("required-object", { type: "object", properties: text, required: { text: true } });
  await s("additional-false", { type: "object", additionalProperties: false, properties: text });
  await s("additional-true", { type: "object", properties: text, additionalProperties: true });
  await s("additional-object", { type: "object", properties: text, additionalProperties: { type: "string" } });
  await s("additional-zero", { type: "object", properties: text, additionalProperties: 0 });
  // Truthy compositions skip injection; falsy ones do not.
  for (const key of ["oneOf", "allOf", "anyOf"]) {
    await s(`${key}-list`, { type: "object", properties: text, [key]: [{ required: ["text"] }] });
    await s(`${key}-empty-list`, { type: "object", properties: text, [key]: [] });
    await s(`${key}-null`, { type: "object", properties: text, [key]: null });
  }
  await s("oneOf-object", { type: "object", oneOf: {}, properties: text });
  await s("anyOf-string", { anyOf: "x" });
  await s("allOf-true", { allOf: true, properties: text });
  await s("oneOf-zero", { type: "object", properties: text, oneOf: 0 });
  await s("anyOf-false", { type: "object", properties: text, anyOf: false });
  await s("allOf-empty-string", { type: "object", properties: text, allOf: "" });
  await s("composed-declares-session_id", { oneOf: [{ required: ["a"] }], properties: { session_id: { type: "string" } } });
  await s("composed-declares-session_id-false", { oneOf: [{ required: ["a"] }], properties: { session_id: false } });
  await s("composed-declares-context", { anyOf: [{ required: ["a"] }], properties: { context: { type: "string" } } });
  await s("composed-declares-session_id-hook-mode", { oneOf: [{}], properties: { session_id: {} } }, {
    options: ALL_ON.options,
    hookMode: true,
  });
  // A customer property of an injected name: truthy keeps it, falsy is overwritten in its place (B-043).
  for (const param of ["session_id", "agent_id", "context"]) {
    await s(`own-${param}`, { type: "object", properties: { a: {}, [param]: { type: "string", description: "mine" }, b: {} } });
    for (const [label, value] of [["false", false], ["null", null], ["zero", 0], ["empty-string", ""]]) {
      await s(`own-${param}-${label}`, { type: "object", properties: { a: {}, [param]: value, b: {} }, required: [param] });
    }
    await s(`own-${param}-empty-object`, { type: "object", properties: { [param]: {} } });
  }
  await s("own-all-three", {
    type: "object",
    properties: { context: { type: "string" }, agent_id: { type: "string" }, session_id: { type: "string" } },
    required: ["session_id"],
  });
  // Property order: customer keys first, then session_id, agent_id, context (B-032).
  await s("property-order", {
    properties: { z: { type: "string" }, a: { type: "number" }, m: { type: "boolean" } },
    type: "object",
    required: ["m", "z"],
  });
  await s("index-keys", { type: "object", properties: { 10: {}, b: {}, 2: {} } });
  await s("schema-keyword", { $schema: DRAFT_07, type: "object", properties: text, title: "T", description: "d" });
  // `properties` that is not an object: a string, number or boolean throws where TS assigns a
  // property (the tool is listed as it came, B-044); an array takes it without a trace.
  for (const [label, value] of [["string", "abc"], ["number", 5], ["true", true], ["empty-array", []], ["array", [{ type: "string" }]]]) {
    await s(`properties-${label}`, { type: "object", properties: value, additionalProperties: false });
    await s(`properties-${label}-agent-only`, { type: "object", properties: value }, {
      options: ALL_ON.options,
      hookMode: true,
    });
    await s(`properties-${label}-context-only`, { type: "object", properties: value, additionalProperties: false }, CONTEXT_ONLY);
  }
  // Names: TS keys a tool named "" as "unknown"; tools that share a name merge their records.
  await one("schema:empty-name", ALL_ON, [{ name: "", inputSchema: { type: "object" } }]);
  await one("schema:empty-name-and-unknown", ALL_ON, [
    { name: "", inputSchema: { type: "object", properties: { session_id: {} } } },
    { name: "unknown", inputSchema: { type: "object" } },
  ]);
  await one("schema:shared-name", ALL_ON, [
    { name: "dup", inputSchema: { type: "object", properties: { session_id: { type: "string" } } } },
    { name: "dup", inputSchema: { type: "object", properties: {} } },
  ]);
  await one("schema:shared-name-one-fails", ALL_ON, [
    { name: "dup", inputSchema: { type: "object", properties: {} } },
    { name: "dup", inputSchema: { type: "object", properties: "abc" } },
  ]);
  await one("schema:shared-name-context-fails", CONTEXT_ONLY, [
    { name: "dup", inputSchema: { type: "object", properties: {} } },
    { name: "dup", inputSchema: { type: "object", properties: 1 } },
  ]);
  await one("schema:shared-name-own-session-twice", ALL_ON, [
    { name: "dup", inputSchema: { type: "object", properties: { session_id: { type: "string" } } } },
    { name: "dup", inputSchema: { type: "object", properties: { session_id: { type: "number" } } } },
  ]);
  await one("schema:other-tool-keys", ALL_ON, [{
    name: "t",
    title: "A title",
    description: "",
    inputSchema: { type: "object", properties: text },
    annotations: { title: "Annotated", readOnlyHint: false, openWorldHint: true },
    _meta: { "vendor/key": [1, "x"] },
  }]);
  await one("schema:non-ascii", ALL_ON, [{
    name: "wörter 😀",
    description: "  é",
    inputSchema: { type: "object", properties: { "ключ": { type: "string", description: "\"quoted\" \\ \n" } } },
  }]);
}

async function outputCases() {
  const o = (name, outputSchema, flags = ALL_ON, inputSchema = { type: "object", properties: {} }) =>
    one(`output:${name}`, flags, [{ name: "t", inputSchema, outputSchema }]);
  const count = { count: { type: "number" } };
  await o("object", { type: "object", properties: count, required: ["count"], additionalProperties: false });
  await o("no-properties", { type: "object" });
  await o("empty", {});
  await o("properties-null", { type: "object", properties: null });
  await o("array", { type: "array" });
  await o("array-items", { type: "array", items: { type: "string" } });
  for (const key of ["oneOf", "allOf", "anyOf"]) {
    await o(`${key}`, { [key]: [{ type: "object", properties: count }] });
    await o(`${key}-null`, { type: "object", properties: count, [key]: null });
  }
  await o("anyOf-empty-list", { type: "object", properties: count, anyOf: [] });
  await o("declares-mcp_session", { type: "object", properties: { mcp_session: { type: "object" }, ...count } });
  await o("declares-mcp_session-false", { type: "object", properties: { ...count, mcp_session: false, z: {} } });
  await o("declares-mcp_session-null", { type: "object", properties: { mcp_session: null } });
  await o("properties-string", { type: "object", properties: "x" });
  await o("properties-number", { type: "object", properties: 7 }, handleFlags(false, true));
  await o("properties-array", { type: "object", properties: [] });
  await o("properties-string-input-owns-session_id", { type: "object", properties: "x" }, ALL_ON, {
    type: "object",
    properties: { session_id: { type: "string" }, agent_id: { type: "string" } },
  });
  await o("input-owns-session_id", { type: "object", properties: count }, ALL_ON, {
    type: "object",
    properties: { session_id: { type: "string" } },
  });
  await o("input-composed", { type: "object", properties: count }, ALL_ON, { anyOf: [{}] });
  // Hook mode (B-064): mcp_session with agent_id only, or no handle step at all.
  await o("hook-mode-agent", { type: "object", properties: count }, handleFlags(false, true));
  await o("hook-mode-no-agent", { type: "object", properties: count }, {
    options: trackOptions({ enableReportMissing: false, enableAgentTracking: false }),
    hookMode: true,
  });
  await o("session-only", { type: "object", properties: count }, handleFlags(true, false));
  await o("tracing-off", { type: "object", properties: count }, CONTEXT_ONLY);
}

async function reportMissingCases() {
  const echo = { name: "echo", inputSchema: { type: "object", properties: { msg: { type: "string" } } } };
  const customer = { name: "get_more_tools", description: "customer version", inputSchema: { type: "object", properties: {} } };
  const flags = (overrides, hookMode = false) => ({ options: trackOptions(overrides), hookMode });
  await add("report-missing:customer-owned-two-listings", flags({}), [[customer, echo], [customer, echo]]);
  await add("report-missing:customer-owned-twice-in-one-listing", flags({}), [[customer, echo, customer]]);
  await one("report-missing:customer-owned-off", flags({ enableReportMissing: false }), [customer, echo]);
  await one("report-missing:customer-owned-tracing-off", flags({ enableTracing: false }), [customer, echo]);
  await one("report-missing:appended-agent-tracking", flags({ enableAgentTracking: true }), [echo]);
  await one("report-missing:appended-hook-mode", flags({ enableAgentTracking: true }, true), [echo]);
  await one("report-missing:appended-hook-mode-no-agent", flags({}, true), [echo]);
  await one("report-missing:appended-tracing-off", flags({ enableTracing: false }), [echo]);
  await one("report-missing:off", flags({ enableReportMissing: false }), [echo]);
}

async function pageCases() {
  const flags = { options: trackOptions({ enableAgentTracking: true }), hookMode: false };
  const own = (name, param) => ({ name, inputSchema: { type: "object", properties: { [param]: { type: "string" } } } });
  const composed = { name: "composed", inputSchema: { oneOf: [{}] }, outputSchema: { anyOf: [{}] } };
  // The session_id collision logs once per tool (B-040); the agent_id and context skips and the
  // composed-schema warnings log on every listing (B-041).
  await add("pages:two", flags, [[own("s", "session_id"), own("a", "agent_id"), own("c", "context"), composed], [own("s", "session_id"), own("a", "agent_id"), own("c", "context"), composed]]);
  await add("pages:three", flags, [[own("s", "session_id")], [own("a", "agent_id"), own("s", "session_id")], [own("s", "session_id"), own("s2", "session_id"), composed]]);
  // A name listed on two pages: page 2's tool no longer declares session_id, but the declared set only grows.
  await add("pages:shared-name", flags, [[own("x", "session_id")], [{ name: "x", inputSchema: { type: "object" } }]]);
  await add("pages:shared-name-reversed", flags, [[{ name: "x", inputSchema: { type: "object" } }], [own("x", "session_id")]]);
  await add("pages:different-tools", { options: trackOptions(), hookMode: false }, [
    [{ name: "a", inputSchema: { type: "object" } }],
    [{ name: "b", inputSchema: { type: "object" }, outputSchema: { type: "object" } }],
  ]);
  await add("pages:hook-mode", { options: trackOptions({ enableAgentTracking: true }), hookMode: true }, [
    [own("s", "session_id"), own("a", "agent_id")],
    [own("s", "session_id"), own("a", "agent_id")],
  ]);
}

async function rd20Cases() {
  const t = (fields) => ({ name: "t", inputSchema: { type: "object" }, ...fields });
  await rd20("input-schema-missing", ALL_ON, { name: "t" });
  for (const [label, value] of [["null", null], ["string", "x"], ["array", []], ["true", true], ["false", false], ["zero", 0]]) {
    await rd20(`input-schema-${label}`, ALL_ON, t({ inputSchema: value }));
  }
  await rd20("name-missing", ALL_ON, { inputSchema: { type: "object" } });
  for (const [label, value] of [["null", null], ["number", 5], ["object", { a: 1 }], ["true", true]]) {
    await rd20(`name-${label}`, ALL_ON, t({ name: value }));
  }
  for (const [label, value] of [["string", "x"], ["array", []], ["true", true], ["false", false], ["zero", 0]]) {
    await rd20(`output-schema-${label}`, ALL_ON, t({ outputSchema: value }));
  }
}

// The random listings: shapes like those above, also in the Rust property test's tool_list(). No
// tool is named "": the registry name TS gives it, "unknown", has its own cases (schema:empty-name*).
const NAMES = ["echo", "search", "get_more_tools", "own", "unknown"];
const PROPERTY_VALUES = [{ type: "string" }, { type: "number", description: "n" }, {}, false, null, 0, ""];

function randomProperties(rand) {
  const keys = ["text", "q", "session_id", "agent_id", "context", "10", "2"].filter(() => rand() < 0.35);
  return Object.fromEntries(keys.map((key) => [key, pick(rand, PROPERTY_VALUES)]));
}

function randomInputSchema(rand) {
  const entries = [];
  if (rand() < 0.7) entries.push(["type", "object"]);
  const properties = pick(rand, ["object", "object", "object", "absent", "falsy", "primitive", "array"]);
  if (properties === "object") entries.push(["properties", randomProperties(rand)]);
  if (properties === "falsy") entries.push(["properties", pick(rand, [null, false, 0, ""])]);
  if (properties === "primitive") entries.push(["properties", pick(rand, ["abc", 5, true])]);
  if (properties === "array") entries.push(["properties", pick(rand, [[], [{ type: "string" }]])]);
  const required = pick(rand, ["absent", "absent", "list", "list", "other"]);
  if (required === "list") entries.push(["required", ["text", "session_id", "agent_id", "context"].filter(() => rand() < 0.3)]);
  if (required === "other") entries.push(["required", pick(rand, ["text", null, {}])]);
  if (rand() < 0.4) entries.push(["additionalProperties", pick(rand, [false, false, true, { type: "string" }])]);
  if (rand() < 0.25) entries.push([pick(rand, ["oneOf", "allOf", "anyOf"]), pick(rand, [[], [{ required: ["q"] }], {}, null, 0, false, ""])]);
  // Shuffle, so the keys come in any order.
  for (let i = entries.length - 1; i > 0; i--) {
    const j = int(rand, 0, i);
    [entries[i], entries[j]] = [entries[j], entries[i]];
  }
  return Object.fromEntries(entries);
}

function randomOutputSchema(rand) {
  const kind = pick(rand, ["object", "object", "no-properties", "array", "composed", "mcp_session", "primitive", "properties-array"]);
  const count = { count: { type: "number" } };
  switch (kind) {
    case "object":
      return { type: "object", properties: count, ...(rand() < 0.5 && { required: ["count"], additionalProperties: false }) };
    case "no-properties":
      return pick(rand, [{ type: "object" }, {}]);
    case "array":
      return { type: "array" };
    case "composed":
      return { [pick(rand, ["oneOf", "allOf", "anyOf"])]: pick(rand, [[{ type: "object" }], [], null]) };
    case "mcp_session":
      return { type: "object", properties: { ...count, mcp_session: pick(rand, [{ type: "object" }, false]) } };
    case "primitive":
      return { type: "object", properties: pick(rand, ["x", 3]) };
    default:
      return { type: "object", properties: [] };
  }
}

function randomTool(rand) {
  return {
    name: pick(rand, NAMES),
    ...(rand() < 0.5 && { description: pick(rand, ["Search", "", "d"]) }),
    inputSchema: randomInputSchema(rand),
    ...(rand() < 0.4 && { outputSchema: randomOutputSchema(rand) }),
    ...(rand() < 0.15 && { annotations: { title: "T", readOnlyHint: true } }),
  };
}

async function seededCases() {
  const rand = seeded(8);
  for (let i = 0; i < 30; i++) {
    const options = {
      enableReportMissing: rand() < 0.5,
      enableTracing: rand() < 0.7,
      enableToolCallContext: rand() < 0.6,
      enableAgentTracking: rand() < 0.5,
      ...(rand() < 0.3 && { customContextDescription: pick(rand, ["", "why", "Say why."]) }),
    };
    const hookMode = rand() < 0.3;
    const pages = Array.from({ length: int(rand, 1, 2) }, () => Array.from({ length: int(rand, 1, 5) }, () => randomTool(rand)));
    await add(`seeded:${i}`, { options, hookMode }, pages);
  }
}

export async function generate() {
  cases.length = 0;
  await handleInjectionCases();
  await contextCases();
  await contextEdgeCases();
  await conflictCases();
  await schemaEdgeCases();
  await listWrapCases();
  await flagCases();
  await schemaCases();
  await outputCases();
  await reportMissingCases();
  await pageCases();
  await rd20Cases();
  await seededCases();
  return { cases: [...cases] };
}

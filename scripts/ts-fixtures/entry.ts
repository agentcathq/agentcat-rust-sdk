// Golden vectors for the Rust SDK, computed by the TypeScript SDK's own code.
//
// Run scripts/ts-fixtures/generate.sh to rebuild tests/fixtures/ts-vectors.json
// from a checkout of agentcat-typescript-sdk. Every value here comes from a
// real TS function call, never from a transcription of TS logic.
import { deriveSessionId, buildMintBackText, buildStructuredMintBack, mirrorStructuredMintBack, appendMintBack, buildHandleTags, resolveHandles, isValidSessionId, extractHandle } from "@ts/modules/handles";
import { toUUIDv7 } from "@ts/modules/exporters/posthog";
import KSUID from "@ts/thirdparty/ksuid/index.js";
import { estimateInputTokens, estimateOutputTokens, estimateTokens } from "@ts/modules/tokenEstimate";
import { buildInjectedList } from "@ts/engine/listWrap";
import { stripInjectedArguments } from "@ts/modules/handle-injection";
import { truncateEvent, normalize } from "@ts/modules/truncation";
import { sanitizeEvent } from "@ts/modules/sanitization";
import { captureException } from "@ts/modules/exceptions";
import { traceContext } from "@ts/modules/exporters/trace-context";
import { getReportMissingToolDescriptor, handleReportMissing } from "@ts/modules/tools";

const out: Record<string, unknown> = {};

// ── KSUID and session derivation ────────────────────────────────────────────
const deriveInputs: Array<[string, string | undefined]> = [
  ["customer-abc", "proj_1"],
  ["customer-abc", undefined],
  ["x", "p"],
  ["customer-42", "proj_2cOHEO0LYGADMzRvWTXXVbbgxgm"],
  ["user@example.com", "proj_abc123xyz"],
  ["日本語-セッション", "proj_1"],
  ["a", ""],
];
out.derive = deriveInputs.map(([id, p]) => ({ id, project_id: p ?? null, session_id: deriveSessionId(id, p) }));

const ksuidParts: Array<[number, number[]]> = [
  [1400000000000, Array(16).fill(0)],
  [1704067200000, Array.from({ length: 16 }, (_, i) => i)],
  [1760000000123, Array(16).fill(255)],
  [5694967295000, Array(16).fill(255)],
];
out.ksuid_from_parts = ksuidParts.map(([ms, bytes]) => ({ time_ms: ms, payload: bytes, encoded: KSUID.fromParts(ms, Buffer.from(bytes)).string }));
out.ksuid_parse = ["0ujtsYcgvSTl8PAuAdqWYSMnLOv", "000000000000000000000000000", "aWgEPTl1tmebfsQzFP4bxwgy80V", "2cOHEO0LYGADMzRvWTXXVbbgxgm"].map((s) => {
  const k = KSUID.parse(s);
  return { encoded: s, timestamp: k.timestamp, payload: [...k.payload] };
});

out.uuidv7 = [
  "ses_2cOHEO0LYGADMzRvWTXXVbbgxgm",
  "evt_0ujtsYcgvSTl8PAuAdqWYSMnLOv",
  "ses_000000000000000000000000000",
  "my-custom-session",
  "ses_short",
  "",
  "agentcat_ses_2cOHEO0LYGADMzRvWTXXVbbgxgm",
].map((id) => ({ id, uuid: toUUIDv7(id) }));

out.trace_context = ["ses_2cOHEO0LYGADMzRvWTXXVbbgxgm", "evt_0ujtsYcgvSTl8PAuAdqWYSMnLOv", "x"].map((id) => ({
  id,
  trace_id: traceContext.getTraceId(id),
  span_id: traceContext.getSpanId(id),
  dd_trace_id: traceContext.getDatadogTraceId(id),
  dd_span_id: traceContext.getDatadogSpanId(id),
}));

out.is_valid_session_id = ["ses_2cOHEO0LYGADMzRvWTXXVbbgxgm", "ses_2cOHEO0LYGADMzRvWTXXVbbgxg", "ses_2cOHEO0LYGADMzRvWTXXVbbgxgm1", "SES_2cOHEO0LYGADMzRvWTXXVbbgxgm", "ses_2cOHEO0LYGADMzRvWTXXVbbgx-m", "start", ""].map((v) => ({ value: v, valid: isValidSessionId(v) }));

out.extract_handle = [
  [{ session_id: "  ses_x  " }, "session_id"],
  [{ session_id: "" }, "session_id"],
  [{ session_id: "   " }, "session_id"],
  [{ session_id: 42 }, "session_id"],
  [{ session_id: null }, "session_id"],
  [{}, "session_id"],
  [{ agent_id: "\topus|cc|k3n9x\n" }, "agent_id"],
].map(([args, name]) => ({ args, name, value: extractHandle(args, name as string) ?? null }));

// ── Handles: resolution, mint-back, tags ────────────────────────────────────
const resolutionCases = [
  { label: "omitted", options: {}, args: { q: 1 }, ours: true },
  { label: "start", options: {}, args: { session_id: " START " }, ours: true },
  { label: "supplied", options: {}, args: { session_id: "ses_2cOHEO0LYGADMzRvWTXXVbbgxgm" }, ours: true },
  { label: "invalid", options: {}, args: { session_id: "ses_invented" }, ours: true },
  { label: "foreign", options: {}, args: { session_id: "customer-owned" }, ours: false },
  { label: "hook", options: { resolveSessionId: () => "x" }, args: { session_id: "ses_2cOHEO0LYGADMzRvWTXXVbbgxgm" }, ours: true },
  { label: "agent", options: { enableAgentTracking: true }, args: { session_id: "ses_2cOHEO0LYGADMzRvWTXXVbbgxgm", agent_id: " opus-4.80-1m|claude-code|k3n9x " }, ours: true },
  { label: "agent_off", options: { enableAgentTracking: false }, args: { agent_id: "opus" }, ours: true },
  { label: "agent_foreign", options: { enableAgentTracking: true }, args: { session_id: "x", agent_id: "line1\nline2\r" + "z".repeat(300) }, ours: false },
];
out.resolutions = resolutionCases.map((c) => {
  const res = resolveHandles(c.options as any, "proj_1", { params: { name: "t", arguments: c.args } }, undefined, c.ours);
  const minted = res.sessionSource === "minted";
  const stable = { ...res, sessionId: minted ? "<minted>" : res.sessionId };
  const text = buildMintBackText(res);
  const structured = buildStructuredMintBack(res);
  return {
    label: c.label,
    hook_mode: res.hookMode,
    session_source: res.sessionSource,
    session_id: stable.sessionId,
    agent_id: res.agentId ?? null,
    agent_source: res.agentSource ?? null,
    mint_back_text: text === null ? null : text.replace(res.sessionId || "<none>", minted ? "<minted>" : res.sessionId),
    structured: structured === null ? null : JSON.parse(JSON.stringify({ ...structured, ...(structured.session_id && minted ? { session_id: "<minted>" } : {}) })),
    tags: buildHandleTags(res, "2026-07-28"),
    tags_no_protocol: buildHandleTags(res),
  };
});

out.mirror = [
  { result: { content: [], structuredContent: { a: 1, b: { c: 2 } } }, mint: { session_id: "ses_2cOHEO0LYGADMzRvWTXXVbbgxgm", status: "active" } },
  { result: { content: [], structuredContent: { mcp_session: "customer", a: 1 } }, mint: { status: "issued" } },
  { result: { content: [], structuredContent: [1, 2] }, mint: { status: "issued" } },
  { result: { content: [] }, mint: { status: "issued" } },
].map((c) => ({ ...c, out: mirrorStructuredMintBack(c.result, c.mint as any) }));
out.append_mint_back = { result: { content: [{ type: "text", text: "hi" }], isError: true }, text: "BLOCK", out: appendMintBack({ content: [{ type: "text", text: "hi" }], isError: true }, "BLOCK") };

// ── Schema injection ────────────────────────────────────────────────────────
const tools = [
  { name: "search", description: "Search", inputSchema: { type: "object", properties: { query: { type: "string" } }, required: ["query"], additionalProperties: false } },
  { name: "no_schema", description: "No schema" },
  { name: "empty_schema", inputSchema: {} },
  { name: "composed", inputSchema: { type: "object", oneOf: [{ properties: { a: { type: "string" } } }], properties: { session_id: { type: "string" } } } },
  { name: "composed_plain", inputSchema: { anyOf: [{ type: "object" }] } },
  { name: "own_session", inputSchema: { type: "object", properties: { session_id: { type: "number" }, x: { type: "string" } }, required: ["session_id"] } },
  { name: "own_context", inputSchema: { type: "object", properties: { context: { type: "string", description: "mine" } } } },
  { name: "own_agent", inputSchema: { type: "object", properties: { agent_id: { type: "string" } } } },
  { name: "structured", inputSchema: { type: "object", properties: { n: { type: "integer" } } }, outputSchema: { type: "object", properties: { result: { type: "string" } }, required: ["result"], additionalProperties: false } },
  { name: "structured_composed", inputSchema: { type: "object" }, outputSchema: { allOf: [{ type: "object" }] } },
  { name: "structured_own", inputSchema: { type: "object" }, outputSchema: { type: "object", properties: { mcp_session: { type: "string" } } } },
  { name: "structured_noprops", inputSchema: { type: "object" }, outputSchema: { type: "object" } },
  { name: "required_existing", inputSchema: { type: "object", properties: { a: { type: "string" } }, required: ["a", "context"] } },
];
const injectionConfigs: Record<string, any> = {
  defaults: {},
  agent_tracking: { enableAgentTracking: true },
  hook_mode: { resolveSessionId: () => "x", enableAgentTracking: true },
  no_context: { enableToolCallContext: false },
  no_report_missing: { enableReportMissing: false },
  tracing_off: { enableTracing: false },
  custom_context: { customContextDescription: "Why are you calling this?" },
};
out.injection = Object.entries(injectionConfigs).map(([label, opts]) => {
  const options = {
    enableReportMissing: opts.enableReportMissing ?? true,
    enableTracing: opts.enableTracing ?? true,
    enableToolCallContext: opts.enableToolCallContext ?? true,
    customContextDescription: opts.customContextDescription,
    enableAgentTracking: opts.enableAgentTracking ?? false,
    resolveSessionId: opts.resolveSessionId,
  };
  const data = { projectId: "proj_1", options };
  const result = buildInjectedList(data as any, JSON.parse(JSON.stringify(tools)));
  const registry: Record<string, string[]> = {};
  for (const [k, v] of result.registry) registry[k] = [...v];
  return {
    label,
    config: { ...opts, resolveSessionId: opts.resolveSessionId ? true : undefined },
    tools_json: JSON.stringify(result.tools),
    registry,
    output_registry: [...result.outputRegistry],
  };
});
out.injection_input_json = JSON.stringify(tools);
out.injection_customer_get_more_tools = (() => {
  const data = { projectId: "proj_1", options: { enableReportMissing: true, enableTracing: true, enableToolCallContext: true, enableAgentTracking: false } };
  const list = [{ name: "get_more_tools", description: "mine", inputSchema: { type: "object", properties: { q: { type: "string" } } } }];
  const result = buildInjectedList(data as any, list);
  const registry: Record<string, string[]> = {};
  for (const [k, v] of result.registry) registry[k] = [...v];
  return { input_json: JSON.stringify(list), tools_json: JSON.stringify(result.tools), registry };
})();
out.report_missing = { descriptor_json: JSON.stringify(getReportMissingToolDescriptor()), response_json: JSON.stringify(handleReportMissing({ context: "x" })) };

out.strip = [
  { args: { a: 1, session_id: "s", agent_id: "g", context: "c" }, tool: "t", registry: null },
  { args: { a: 1, session_id: "s", agent_id: "g", context: "c" }, tool: "get_more_tools", registry: null },
  { args: { a: 1, session_id: "s", agent_id: "g", context: "c" }, tool: "t", registry: { t: ["context"] } },
  { args: { a: 1, session_id: "s", agent_id: "g", context: "c" }, tool: "t", registry: { t: [] } },
].map((c) => {
  const reg = c.registry ? new Map(Object.entries(c.registry).map(([k, v]) => [k, new Set(v)])) : undefined;
  return { ...c, out: stripInjectedArguments(c.args, c.tool, reg as any) };
});

// ── Token estimates ─────────────────────────────────────────────────────────
out.tokens_input = [{ q: "hello" }, {}, { name: "café" }, { html: "<a>&</a>" }, { t: "你好" }, { ids: [1, 2, 3], opts: { deep: true, n: null } }, null, { emoji: "😀👍🏽" }, { s: "line\nbreak\t\"quote\"\\" }, { ctrl: "\u0001\u001f" }].map((args) => ({ args, tokens: estimateInputTokens(args) ?? null }));
out.tokens_output = [
  { content: [{ type: "text", text: "hello world" }] },
  { content: [{ type: "text", text: "abcd" }, { type: "text", text: "e" }] },
  { content: [{ type: "text", text: "" }] },
  { content: [{ type: "image", data: "aGk=", mimeType: "image/png" }] },
  { content: [{ type: "resource", resource: { uri: "file:///a", text: "resource body" } }] },
  { content: [{ type: "resource", resource: { uri: "file:///a", blob: "aGk=" } }] },
  { content: [{ type: "text", text: "hi" }], structuredContent: { big: "x".repeat(1000) } },
  { content: [], structuredContent: { result: "ok" } },
  { content: [] },
  { result: "ok" },
  null,
  { content: [{ type: "text", text: "x".repeat(4096) }] },
  { content: [{ type: "text", text: "😀 emoji" }, null, 5, { type: "text" }] },
].map((response) => ({ response, tokens: estimateOutputTokens(response) ?? null }));
out.tokens_bytes = [0, 1, 7, 3.5, 4, 2147483647 * 3.5, 2147483647 * 4].map((b) => ({ bytes: b, tokens: estimateTokens(b) }));

// ── Sanitization ────────────────────────────────────────────────────────────
const b64 = "QUJD".repeat(2600);
out.sanitize = [
  { response: { content: [{ type: "text", text: "ok" }, { type: "image", data: "x", mimeType: "image/png" }, { type: "audio", data: "x", mimeType: "audio/wav" }, { type: "resource", resource: { uri: "u", blob: "x" } }, { type: "resource", resource: { uri: "u", text: "t" } }, { type: "resource", resource: { uri: "u", blob: null } }, { type: "resource_link", uri: "u", name: "n" }, { type: "video" }, { noType: true }, null], structuredContent: { file: b64, small: "QUJD" } }, parameters: { request: { params: { arguments: { data: b64, nested: [b64, "short"], almost: "QUJD".repeat(2559), urlsafe: "QU-D".repeat(2600), withEq: "QUJD".repeat(2600) + "==", newline: "QUJD\n".repeat(2100) } } } } },
  { response: "plain string", parameters: [b64] },
  { response: { content: "not-an-array", structuredContent: "str" } },
].map((ev) => ({ input: ev, output: sanitizeEvent(ev as any) }));

// ── Truncation ──────────────────────────────────────────────────────────────
const longStr = "a".repeat(40000);
const deep: any = {}; let cur = deep; for (let i = 0; i < 15; i++) { cur.next = { level: i }; cur = cur.next; }
const wide: any = {}; for (let i = 0; i < 150; i++) wide["k" + i] = i;
const frames = Array.from({ length: 60 }, (_, i) => ({ filename: "f" + i + ".rs", function: "fn" + i, in_app: true }));
out.truncate = [
  { label: "fields", event: { userIntent: "i".repeat(3000), resourceName: "r".repeat(300), serverName: "s".repeat(300), serverVersion: "v".repeat(300), clientName: "c".repeat(300), clientVersion: "w".repeat(300), error: { message: "m".repeat(3000), frames } } },
  { label: "content_text", event: { response: { content: [{ type: "text", text: longStr }, { type: "image", data: longStr }] } } },
  { label: "normalize", event: { parameters: { deep, wide, arr: Array.from({ length: 120 }, (_, i) => i), long: longStr, unicode: "é".repeat(33000) } } },
  { label: "size_depth", event: { parameters: { a: { b: { c: { d: { e: { f: { g: { h: { i: "x".repeat(30000) } } } } } } } } }, response: { content: [{ type: "text", text: "y".repeat(30000) }, { type: "text", text: "z".repeat(30000) }, { type: "text", text: "w".repeat(30000) }] } } },
  { label: "size_last_resort", event: { parameters: { a: "x".repeat(30000), b: "y".repeat(30000), c: "z".repeat(30000), d: "w".repeat(30000) }, response: { r1: "q".repeat(30000) }, tags: { t: "small" }, properties: { p: "p".repeat(5000) } } },
].map(({ label, event }) => ({ label, input: event, output: truncateEvent(event as any) }));
out.normalize = [
  { input: { a: [1, { b: [2, { c: 3 }] }] }, depth: 2 },
  { input: { a: [1, { b: [2, { c: 3 }] }] }, depth: 1 },
  { input: [1, [2, [3]]], depth: 2 },
  { input: "x".repeat(10), depth: 10, max_string: 4 },
].map((c) => ({ ...c, output: normalize(c.input, c.depth, 100, (c as any).max_string ?? 32768) }));

// ── Error capture for tool results ──────────────────────────────────────────
out.capture_tool_result = [
  { content: [{ type: "text", text: "  first " }, { type: "image", data: "x" }, { type: "text", text: "second  " }], isError: true },
  { content: [], isError: true },
  { content: [null, 5, { type: "text", text: "only" }], isError: true },
].map((r) => ({ result: r, error: captureException(r) }));

console.log(JSON.stringify(out, null, 2));

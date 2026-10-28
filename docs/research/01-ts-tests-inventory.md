# agentcat-typescript-sdk test-suite inventory

Repo: /home/ubuntu/agentcat-typescript-sdk (package `agentcat` 2.1.1-beta.1, depends on `agentcat-api` 1.0.3). Read from HEAD.
All paths are under `src/tests/` unless noted. Line counts are from `git ls-files | wc -l`.
Totals: 79 `*.test.ts` files plus 9 helper files (about 23.6k lines).
Breakdown: top-level `*.test.ts` = 56, `v2/*.test.ts` = 17, `e2e-http/*.test.ts` = 5, `workers/*.test.ts` = 1 (total 79). Helpers = 9: setup.ts, test-utils.ts, test-utils/client-server-factory.ts, v2/harness.ts, e2e-http/{lanes,scenario-types,toolkit}.ts, workers/{worker.ts,env.d.ts}.

---------------------------------------------------------------------------
## 1. How tests run (package.json, vitest configs, CI)

### package.json scripts
- `test` = `pnpm run test:v1 && pnpm run test:v2` (default chain; e2e-http and workers are NOT in it).
- `test:v1` = `vitest run --project v1`; `test:v2` = `vitest run --project v2 --passWithNoTests`.
- `test:e2e:http` = `vitest run --project e2e-http` (opt-in).
- `test:workers` = `pnpm run build && vitest run --config vitest.workers.config.ts` (needs built `dist/`).
- `test:compatibility` = v1 project, file `mcp-version-compatibility.test.ts`; `test:esm-consume` = v1 project, `esm-consumer.test.ts`.
- `test:coverage` = `vitest run --coverage` (@vitest/coverage-v8); `lint` = eslint src/; `typecheck` = tsc --noEmit; `prepublishOnly` = build + test + lint + typecheck.
- Engines: node >=20. pnpm 10.11.0. vitest ^4.0.14.
- Dev deps pin MCP SDKs: `@modelcontextprotocol/sdk ~1.31.0` (v1), `@modelcontextprotocol/server ^2.0.0`, `/client ^2.2.0`, `/node ^2.0.0` (v2); zod ^3.25 and `zod4` alias (`npm:zod@^4.2.0`). Both v1 and v2 SDK are installed at once (coexistence).
- Peer deps: `@modelcontextprotocol/sdk >=1.11 <2` (optional), `@modelcontextprotocol/server >=2.0.0 <3` (optional).
- Note: diagnostics tests comment says "devDep is ~1.30.0" but package.json says ~1.31.0 (stale comment; tests only assert `^1\.` and `^2\.`).
- Also `exports` has a `workerd` condition -> `dist/index.workerd.mjs`.

### vitest.config.ts (3 projects)
All share: `setupFiles: ["./src/tests/setup.ts"]`, `pool: "forks"`, `poolOptions.forks.singleFork: true` (sequential, to avoid `agentcat.log` file conflicts).
- `v1`: include `src/tests/**/*.test.ts`; exclude `src/tests/v2/**`, `src/tests/e2e-http/**`, `src/tests/workers/**`.
- `v2`: include `src/tests/v2/**/*.test.ts`, passWithNoTests.
- `e2e-http`: include `src/tests/e2e-http/**/*.test.ts`, `testTimeout: 30_000`, passWithNoTests.
- Default testTimeout elsewhere (vitest default 5s).

### vitest.workers.config.ts
Uses `@cloudflare/vitest-pool-workers` `cloudflareTest({wrangler:{configPath: wrangler.workers-test.jsonc}})`; include `src/tests/workers/**/*.test.ts`; `testTimeout: 15_000`, `teardownTimeout: 30_000`. wrangler.workers-test.jsonc: main `src/tests/workers/worker.ts`, compatibility_date `2026-08-01`, flags nodejs_compat + several enable_nodejs_* modules.

### src/tests/setup.ts (6 lines)
Sets `process.env.DISABLE_DIAGNOSTICS = "true"` for every run (belt-and-suspenders; SDK also auto-disables diagnostics in test envs). Tests exercising diagnostics set `DISABLE_DIAGNOSTICS=false` and mock fetch. setup.ts:6.

### Test env vars seen in tests
- `DISABLE_DIAGNOSTICS` (setup.ts; diagnostics-optout.test.ts; diagnostics-test-env.test.ts).
- `DIAGNOSTICS_TOKEN` (diagnostics-auth.test.ts).
- `AGENTCAT_API_URL` and legacy `MCPCAT_API_URL` (api-base-url.test.ts).
- Test-env auto-detect signals: `VITEST=true`, `JEST_WORKER_ID=1`, `NODE_ENV=test` (diagnostics-optout.test.ts:74-78).
- The `AGENTCAT_*` hits in grep are mostly TS constants (`AGENTCAT_TAG_SESSION_SOURCE`, `AGENTCAT_TAG_AGENT_ID`, `AGENTCAT_TAG_AGENT_SOURCE`, `AGENTCAT_TAG_PROTOCOL_VERSION`, `AGENTCAT_CUSTOM_EVENT_TYPE`, `AGENTCAT_SOURCE`), not env vars. Only `AGENTCAT_API_URL` is a real env var.
- Log file: `agentcat.log` in cwd is deleted before/after each test by `setupTestHooks()` (test-utils.ts:6, 29-37). Real log target is `<homedir>/agentcat.log` (logging-target.test.ts:14).

### .github/workflows/ci.yml ("Continuous Integration Tests")
Triggers: push/PR to main, develop; workflow_dispatch.
- `test-node-versions`: Node 20.x and 22.x; steps: `pnpm install`, `typecheck`, `lint`, `pnpm run test` (v1 then v2 projects), `build`.
- `test-coverage-report`: Node 20.x; `pnpm run test --coverage`; upload to Codecov (`fail_ci_if_error: true`).
- Does not run e2e-http, workers, or packaging probe.

### .github/workflows/mcp-compatibility.yml ("MCP SDK Version Compatibility Testing")
Triggers: push/PR to main, daily cron `0 2 * * *`, dispatch. All matrices Node 20 and 22, `fail-fast: false`.
- `test-v1-sdk-versions`: `pnpm view @modelcontextprotocol/sdk versions`, keeps stable `1.x` with minor >= 11, takes latest patch per minor (so 1.11 ... latest 1.x), `pnpm add -D @modelcontextprotocol/sdk@$version` then `pnpm run test:v1` for each.
- `test-v2-sdk-versions`: enumerates `@modelcontextprotocol/server` versions >= 2.0.0, latest patch per minor; installs `server@v` + `client@v` pinned together (v1 devDep stays installed, so also checks v1+v2 coexistence); runs full `pnpm test` (v1 + v2 projects).
- `e2e-http`: `pnpm run test:e2e:http` on Node 20/22.
- `packaging-probes`: Node 20, `node scripts/packaging-probe.mjs` (install-topology import checks).
- `compatibility-summary`: echo-only report.

---------------------------------------------------------------------------
## 2. Cross-cutting exact values a porter needs (with file:line)

### Wire literals / constants (constants.test.ts)
- Custom event type `"agentcat:custom"` (constants.test.ts:20; also publishCustomEvent.test.ts "always use 'agentcat:custom'").
- Exporter source `"agentcat"` (constants.test.ts:24); diagnostics scope name `"agentcat-diagnostics"` (:28).
- Structured mint-back key `"mcp_session"` (MCP_SESSION_KEY, :32).
- Session param regex `^(start|ses_[0-9A-Za-z]{27})$` (SESSION_ID_PARAM_PATTERN, :86); start sentinel `"start"` (:87); `START` fails the regex (:94, case handling is in resolution).
- Byte-exact copy pinned (constants.test.ts:60-83): SESSION_ID_PARAM_DESCRIPTION (:62-64), MCP_SESSION_STATUS_DESCRIPTION (:68-70: "issued: first call of a task; the session_id above was just created. active: the session_id you sent was accepted; keep sending it. unrecognized: the value sent was not issued by this server -- re-send the one issued earlier for this task; if none was issued yet, send start to be issued a new one."), MINT_BACK_ISSUED_BODY (:74-76 "This is the first-call issuance described in this tool's session_id parameter description."), MINT_BACK_UNRECOGNIZED_BODY (:80-82 "The value sent was not issued by this server. Re-send the session_id issued earlier for this task; if none was issued yet, send start and one will be issued."). Copy these strings verbatim from the test file when porting.
- Mint-back headers: `[session_id issued` and `[session_id unrecognized` (constants.test.ts:45-46).
- agent_id description contains `opus-4.80-1m|claude-code|k3n9x`, "never inherited", "generates a new one", "required on every call including your first", must not contain "the server will issue" nor "session_id" (constants.test.ts:50-55; handle-injection.test.ts:265-279).
- Event tag keys (src/modules/constants.ts:80-84, asserted via literals in handles.test.ts:184-200 etc.): `agentcat_session_id_source`, `agentcat_agent_id`, `agentcat_agent_id_source`, `agentcat_protocol_version`, `agentcat_mrtr` (values `input_required` | `continuation`, engine-callwrap.test.ts:36-37,137,151).
- Session source values observed: `minted`, `supplied`, `hook`, `invalid`, `foreign`; agent source `supplied` (handles.test.ts, handles-integration.test.ts:359,375, v2/lowlevel-handles.test.ts:~240).
- Protocol version tag value `"2026-07-28"` (v2/era-2026.test.ts:85; handles.test.ts:~196).
- Event types: `mcp:tools/call`, `mcp:tools/list`, `mcp:initialize`, `agentcat:custom`; PostHog also maps `mcp:resources/read`, `mcp:resources/list`, `mcp:prompts/get`, `mcp:prompts/list`. tools/list and initialize are NO LONGER published (event-tags-properties.test.ts:428-507; handles-integration.test.ts:225-237; identify.test.ts:792; there is also no separate identify event, report-missing.test.ts:450).

### Injected parameters, tool names, ordering
- Injected input params: `session_id`, `agent_id`, `context` in this order after customer params: customer, session_id, agent_id, context (handle-injection.test.ts:143-166). Both handles and context are added to `required` (soft enforcement; call still succeeds) (:38-60). Hook mode (resolveSessionId set): no `session_id`, only `agent_id` (:168-181). agent_id injected only when `enableAgentTracking: true` (OFF by default) (handles.test.ts:299-318; v2/agent-tracking.test.ts:131-159).
- `session_id` schema gets `pattern` = SESSION_ID_PARAM_PATTERN; `agent_id` has no pattern (handle-injection.test.ts:56-64).
- Skips injection when the tool's schema is composed (`oneOf`/`allOf`/`anyOf`) (context-parameters-edge-cases.test.ts:57-110; handle-injection.test.ts:206-217); skips a param the tool already declares and does not record it/strip it (handle-injection.test.ts:183-205, 302-311); removes `additionalProperties:false` (handle-injection.test.ts:219; context-parameters-edge-cases.test.ts:112); schema `{}` / no inputSchema handled by building a default object schema (context-parameters-edge-cases.test.ts:150-178; v2/schema-edges.test.ts:57).
- Registry-scoped stripping: injected args are removed before the customer handler, but kept on the published event (engine-callwrap.test.ts:89, handles-integration.test.ts:319). Fallback when no registry entry: strip all three (`session_id`,`agent_id`,`context`) (handle-injection.test.ts:312-323) but never strip get_more_tools' own `context` (:325).
- Tool `get_more_tools` is appended to tools/list when `enableReportMissing` true (default) (engine-listwrap.test.ts:42-48; report-missing.test.ts:35). Its inputSchema requires `context` (bespoke description kept), plus `session_id`, `agent_id` when applicable (v2/get-more-tools.test.ts:42-52). Annotations exactly `{title:"Get More Tools", readOnlyHint:true, destructiveHint:false, idempotentHint:true, openWorldHint:false}` (v2/get-more-tools.test.ts:53-59). Description contains "Check for additional tools" (:48). Reply content[1].text contains "Unfortunately" and "we have shown you the full tool list" (v2/get-more-tools.test.ts:70-73; full text in src/modules/tools.ts:42: "Unfortunately, we have shown you the full tool list. We have noted your feedback and will work to improve the tool list in the future."). Content[0] is the mint-back block when minted. Event: `eventType mcp:tools/call`, `resourceName "get_more_tools"`, `userIntent` = context arg (engine-callwrap.test.ts:171-172). If customer owns a tool named get_more_tools it warns once (engine-listwrap.test.ts:212).
- Context param: type string, default description constant `DEFAULT_CONTEXT_PARAMETER_DESCRIPTION` (compared by import, not literal: context-parameters.test.ts:344-346); `customContextDescription` overrides it (custom-context-description.test.ts, v2/options.test.ts:119-175); `enableToolCallContext:false` => no context param and no `userIntent` (v2/options.test.ts:90-117). Existing customer `context` param is preserved (context-preservation.test.ts; context-parameters-edge-cases.test.ts:18-55).
- outputSchema injection: adds property `mcp_session` (type object, sub-properties in order `session_id`, `agent_id`, `status`; `status` = `{type:"string", enum:["issued","active","unrecognized"], description: MCP_SESSION_STATUS_DESCRIPTION}`) to plain-object outputSchemas; hook mode => only `agent_id`; agent tracking off => `session_id`,`status`; composed outputSchema untouched; customer-declared `mcp_session` never overwritten; `additionalProperties:false` kept on output; `required` untouched (handle-injection.test.ts:384-530; v2/schema-edges.test.ts:119-211).

### Mint-back wire format (handles.test.ts)
- Session id: `ses_` + 27 base62 chars `^ses_[0-9A-Za-z]{27}$` (handles.test.ts:24-28; isValidSessionId rejects wrong prefix, no prefix, too short, 28-char body, empty, "ses_", non-base62 body, inner whitespace :708-721).
- Text block (prepended as FIRST content element, also on isError results; never mutates original; non-array content untouched) (handles.test.ts:101-104, 150-181):
  `"[session_id issued -- see this tool's session_id parameter description]\nsession_id: <id>\nThis is the first-call issuance described in this tool's session_id parameter description."` (em dash in source).
  Unrecognized: `"[session_id unrecognized -- see this tool's session_id parameter description]\nThe value sent was not issued by this server. Re-send the session_id issued earlier ... send start and one will be issued."` (handles.test.ts:735-742). No ID-shaped value in it.
- Text block only for MINTED task; never for supplied task, hook mode, or foreign; agent_id never minted (self-chosen) (handles.test.ts:106-148, 775-779).
- Structured mirror `mcp_session` object key order: `session_id, agent_id, status` (status issued|active|unrecognized); hook mode only `agent_id`; invalid => `[status]` or `[agent_id,status]`; foreign => only `agent_id` or null (handles.test.ts:587-653, 756-797). Mirrored as the FIRST key of `structuredContent` (survives client-side truncation), never clobbers existing key, only when structuredContent is a plain object, also on isError results; never mutates (handles.test.ts:655-696). Published `event.response` never contains the mint-back/mcp_session (handles-integration.test.ts:716-815; structured-mintback.test.ts:164).
- Session id resolution (handles.test.ts:258-445): omitted or `start` (case-insensitive, trimmed: "START","Start","sTaRt","  start  "," START\t") => fresh mint; near-sentinels "restart","start now","starts","star t" => `invalid` (sessionId ""); malformed supplied => sessionless, source `invalid`, raw value never stored; well-formed `ses_` => `supplied` verbatim; customer's own `session_id` param => `foreign`, sessionless, never adopted, still reaches handler (handles.test.ts:346-367, 432-443; handles-integration.test.ts:339-378); hook mode ignores agent-supplied session_id.
- Hook mode: `resolveSessionId` return trimmed then `deriveSessionId`; whitespace-only/nullish => silent fresh mint, source `minted`/never "supplied" (handles.test.ts:554-585; v2/session-modes.test.ts:153-184); hook throw/reject is contained => null (:533-552); hook receives (request, extra) verbatim (:492).
- Deterministic derivation golden values (handles.test.ts:59-75): `deriveSessionId("customer-abc","proj_1")` = `ses_2cOHEO0LYGADMzRvWTXXVbbgxgm`; `deriveSessionId("customer-abc")` = `ses_2cZY3tvyI25O2AmL2CGVo2B1IIj`; `deriveSessionId(" x ","p")` = `ses_2c3yR5mYKQdLaXsJNgZH6erbfQK`; `deriveSessionId("x","p")` = `ses_2bw285VY9apdgUgTPXKFnT6P4G0`. Construction (comment :50-53): sha256 of `id:projectId`, 2024 epoch plus hash-derived offset, bytes 4..20 as KSUID payload. NOT trimmed in derive.
- Agent id tag: clamped to exactly 200 chars in the TAG only (event field/ resolution keep full value); CR/LF replaced with spaces ("line1\nline2\r\nline3" -> "line1 line2  line3") (handles.test.ts:211-240).

### Tags/properties validation (validation.test.ts:11-113)
- Keys: regex-allowed chars incl. `.` `:` `-` space `$` and word chars; invalid chars dropped ("invalid!key"); key max 32 chars (33 dropped); value max 200 chars (201 dropped); values with newline dropped; non-string dropped; empty key dropped; max 50 entries kept (60 -> 50, log "Dropping 10"); all-invalid or empty => `null`. Log messages contain "exceeds max length", "newline", "non-string".
- SDK tag wins when a customer tag collides with an SDK tag name (event-tags-properties.test.ts:109; v2/hooks.test.ts:331). Tags/properties bypass `redactSensitiveInformation` (event-tags-properties.test.ts:509). Empty properties normalized to omission (:380).

### Truncation (truncation.test.ts, e2e-truncation.test.ts)
- Ellipsis marker `"..."` appended; resulting length = limit + 3. Default normalize maxStringLength 32768 (truncation.test.ts:11-23); text content blocks in `response.content[]` limited to 32768(+3) (:308-322; e2e-truncation.test.ts:64).
- `userIntent` limit 2048(+3) (:198); `error.message` 2048(+3) (:252); `resourceName`, `serverName`, `serverVersion`, `clientName`, `clientVersion` each 256(+3) (:205-224).
- `error.frames` max 50: first 25 + last 25 (frames 0-24 and 55-79 of 80) (:264-282); exactly 50 unchanged (:284).
- Whole-event cap 102,400 bytes (100KB) of `JSON.stringify` UTF-8; reduce depth progressively, then truncate largest fields as a last resort; `timestamp` stays a `Date` (truncation.test.ts:337-400; e2e-truncation.test.ts:127,282).
- normalize markers: `[Function: name]`, `[Function: <anonymous>]`, `[Symbol(test)]`, `[Symbol()]`, `[undefined]`, `[BigInt: 123]`, `[NaN]`, `[Infinity]`, `[Invalid Date]`, Date -> ISO string with millis, `[Object]`/`[Array]` past max depth (depth 0 collapses top-level), breadth overflow: sentinel key `"..."` = `"[MaxProperties ~]"` for objects and sentinel element `"[MaxProperties ~]"` appended for arrays (maxBreadth 5 -> 6 entries), cycles `"[Circular ~]"` (same object in sibling branches is not a cycle), undefined object properties omitted (JSON.stringify-like) but top-level undefined -> `[undefined]` (truncation.test.ts:26-196).

### Sanitization (sanitization.test.ts, e2e-sanitization.test.ts)
- Response content blocks replaced: image => `{type:"text", text:"[image content redacted - not supported by AgentCat]"}` (:46), audio => `"[audio content redacted - not supported by AgentCat]"` (:50), embedded resource with `blob` => `"[binary resource content redacted - not supported by AgentCat]"` (:92), unknown type => `'[unsupported content type "video" redacted - not supported by AgentCat]'` (:126); text blocks, resource with text, `resource_link` pass through.
- Large base64 strings (anywhere in parameters or structuredContent, nested, arrays) => `"[binary data redacted - not supported by AgentCat]"`; size gate 10240 chars inclusive (10240 redacted, 10239 kept); >10KB non-base64 left alone (sanitization.test.ts:202-307). Test base64 fixture is `"A"*(n-1)+"="`. Does not mutate input event.

### Redaction (redaction.test.ts)
- Protected top-level fields never passed to the redact fn: sessionId, id, projectId, server, serverName, serverVersion, clientName, clientVersion, identifyActorGivenId, identifyActorName, resourceName, eventType, actorId, and identifyData/identifyActorData contents (redaction.test.ts:73-109, 229-260, 316-361). Nested keys with the same name ARE redacted (:281-299).
- Cycles terminate; shared references redacted once; Dates preserved; functions skipped; null/undefined preserved; redact fn errors handled gracefully.
- Event-level hook `redactEvent`: sync/async, returning null/undefined drops the event, throw drops the event (eventQueue.test.ts:596-637), system-managed fields (`id` etc.) restored if hook changes them (redaction.test.ts:513-532), runs BEFORE string redaction and composes (eventQueue.test.ts:639; redaction.test.ts:943), applied to custom events too (:979). e2e redaction scenario: replace `SECRET_TOKEN` with `[REDACTED]` (e2e-http/scenario-types.ts:35-36).

### Token estimates (tokenEstimate.test.ts, token-estimates.test.ts)
Pinned against the Python and Go SDKs (tokenEstimate.test.ts:10-11).
- Divisor `TOKEN_ESTIMATE_BYTES_PER_TOKEN = 3.5` (:15). `estimateTokens(bytes)` = ceil(bytes/3.5), clamped to int32 max: 0->0, 1->1, 7->2, 13->4, 4096->1171, 7516192765->2147483647 (:19-24).
- UTF-8 byte length: ""0, hello 5, cafe(accent) 5, "你好" 6, emoji 4, lone surrogate 3 (:32-37).
- Input tokens = estimate of JSON.stringify(raw arguments incl. injected params? -- raw args as sent): `{q:"hello"}`->4, `{}`->1, `{name:"café"}`->5, `{html:"<a>&</a>"}`->6, `{t:"你好"}`->4, `{ids:[1,2,3],opts:{deep:true,n:null}}`->13; absent/null/unserializable (circular, throwing toJSON) => field omitted, never throws (:46-75). Integration: `{text:"hi there"}` -> inputTokens 6 (token-estimates.test.ts:60).
- Output tokens: sum of UTF-8 bytes of text blocks (and embedded `resource.text`) then ceil: "hello world"->4; "abcd"+"e" -> 2 (summed bytes first, 5/3.5); ""->0; image/blob -> 0; `x*4096`->1171; isError result counted the same; structuredContent counted ONLY when content list is empty (`{content:[],structuredContent:{result:"ok"}}`->5; with text present, structuredContent ignored; image + structuredContent -> 0); no content list => whole response serialized (`{result:"ok"}`->5); absent response => omitted; non-string text ignored; throwing getter => omitted (:79-150). Wire fields on the publish request: `inputTokens`, `outputTokens`, left undefined when absent (eventQueue.test.ts:201-235). Counts are of raw (pre-redaction, pre-truncation) payloads (token-estimates.test.ts:147-210); estimator failure never breaks the tool call (:212).

### Exceptions (exceptions.test.ts, error-capture.test.ts)
- Error info shape: `message`, `type` (constructor name; undefined for non-Error), `stack`, `frames[]`, `chained_errors[]`, `platform:"javascript"` (always, exceptions.test.ts:48-60). Non-Error values: string -> message as-is, number -> "42", boolean -> "false", null -> "null", undefined -> "undefined", object -> JSON `{"code":404,"message":"Not found"}`, no type/stack/frames (:304-348). Event `isError:true` and `error` populated; thrown strings give `error.type` undefined (error-capture.test.ts:200-221).
- Frame fields: `filename`, `abs_path`, `function`, `lineno`, `colno`, `in_app`, `context_line` (in_app frames only, read from file if it exists). Limits: max 50 frames (exceptions.test.ts:163), cause chain max 10 (:254), circular cause detected, non-Error cause -> `{message:"string cause"}` with no type (:267).
- Path normalization (exceptions.test.ts:407-826): strips macOS/Linux/Windows home dirs, deployment prefixes (/var/www, /app, AWS Lambda /var/task, /usr/src/app, /opt/x, /srv/x), project-boundary markers `/src/`, `/lib/`, `/dist/` (last occurrence wins), node_modules -> `node_modules/pkg/...` incl. scoped and nested (last node_modules wins) and Windows backslashes, `node:internal/*` -> `node:internal`, `node:fs/promises` -> `node:fs`, `file://` URLs handled, `native` kept, already-relative paths kept; library frames `in_app:false`; `abs_path` keeps the original path. Key literal examples: `/app/dist/server.js` -> `dist/server.js`; `/var/task/src/handler.ts` -> `src/handler.ts`; `/usr/src/app/src/index.ts` -> `src/index.ts`.
- Thrown tool error is published with `error` and rethrown/returned as `isError:true` result to the client; internal propagation channel is `extra.__agentcat_error` (engine-innertap.test.ts:43; engine-callwrap.test.ts:175).

### Event queue / delivery
- Background task registrar (Workers `waitUntil`): task stays pending until ingestion completes; settles fail-open after retries with log "Failed to deliver AgentCat event after retries"; telemetry-only events not registered; custom exporters not awaited; overflow drop logs `"Event queue full, dropping oldest event"` and settles; registrar throwing logs "Failed to register AgentCat event delivery as a background task" and delivery continues (eventQueue.test.ts:237-414). Tests override private `maxRetries`, `concurrency`, `maxQueueSize` but NO test pins their default numeric values.
- Wire mapping: absent session id => `sessionId` "" on the in-memory event but sent as explicit `null` (eventQueue.test.ts:174-199). Duration computed when absent, preserved when present (:143-172). ID prefixes: event ids `evt_` (KSUID with prefix `evt`), sessions `ses_` (ksuid.test.ts:181-226 tests arbitrary prefixes producing `<prefix>_[0-9A-Za-z]{27}`; posthog-exporter.test.ts uses `KSUID.withPrefix("ses"/"evt")`).
- Success logs only metadata, never payload (eventQueue.test.ts:492; diagnostics-no-payload.test.ts).
- Signal shutdown: on SIGTERM/SIGINT flush, then re-raise the signal unless customer has their own listener; re-raise even if flush fails; never throws (signal-shutdown.test.ts).
- API base URL precedence: `apiBaseUrl` option > `AGENTCAT_API_URL` > legacy `MCPCAT_API_URL` > default (not asserted in these tests); `eventQueue.configure(url)` sets API client `basePath` (api-base-url.test.ts:49-75,132-215).

### Identity / client info
- Client info ladder: v2 envelope `ctx.mcpReq.envelope.clientInfo` > per-request `_meta["io.modelcontextprotocol/clientInfo"]` > legacy `getClientVersion()`; non-string fields dropped; malformed ignored; protocol version from envelope then `_meta` (client-identity.test.ts:17-190; v2/lowlevel-handles.test.ts:249-312).
- `identify` runs per tool call (no dedup, no caching), result stamped on event as `identifyActorGivenId`, `identifyActorName`, `identifyActorData` (from `userId`,`userName`,`userData`), failures/invalid data/null => anonymous, never blocks the call; deferred hooks do not delay the tool call (identify.test.ts; engine-callwrap.test.ts:400-523; v2/hooks.test.ts:245).
- HTTP context capture: `event.parameters.extra`: v1 shape `extra.requestInfo.headers` (verbatim), `extra.requestInfo.url` as string (URL instance replaced by href), `extra.authInfo.token` kept; v2 shape `extra.http.req = {method, url, headers}` plain object with header names lowercased (Headers semantics), e.g. header `x-agentcat-health-run` (extra-projection.test.ts; engine-callwrap.test.ts:292-330; e2e-http/header-capture.test.ts:18; v2/http-context-capture.test.ts:81). Event `parameters.request.params.{name,arguments}` is the request (report-missing.test.ts).

### PostHog exporter wire (posthog-exporter.test.ts, posthog-uuidv7.test.ts)
- POST `https://us.i.posthog.com/batch` default; `host` option overrides, trailing slash stripped (:86-109); headers `Content-Type: application/json`; body `{api_key, batch:[...]}`; each item `{event, type:"capture", distinct_id, timestamp (ISO ms), properties}` (:48-85).
- `distinct_id` = identifyActorGivenId else sessionId; event names: mcp_tool_call, mcp_tools_list, mcp_initialize, mcp_resource_read, mcp_resources_list, mcp_prompt_get, mcp_prompts_list; unknown "mcp:custom/type" -> `mcp_custom_type` (:356-380). properties: `$session_id` (UUIDv7 derived from KSUID; omitted for sessionless), `tool_name` (tools/call only), `resource_name`, `duration_ms`, `server_name`, `server_version`, `client_name`, `client_version`, `project_id`, `is_error`, `source:"agentcat"`, `userIntent` included, customer tags/properties spread flat, `$set` = `{name, ...identifyActorData}` only when identity present (:250-284).
- On `isError`: extra `$exception` item with `$exception_message`, `$exception_type`, `$exception_stacktrace`, `$exception_source:"backend"` (:165-208). With `enableAITracing`: extra `$ai_span` for tool calls only, props `$ai_session_id = "agentcat_" + sessionId`, `$ai_trace_id` (UUIDv7 from session; per-event when sessionless), `$ai_span_id` (UUIDv7 from event id, deterministic), `$ai_span_name`, `$ai_latency` seconds (250ms -> 0.25), `$ai_is_error`, `$ai_input_state`, `$ai_output_state`; customer tags can override `$ai_*` defaults (:450-680). Fetch failure / non-ok never throws.
- `toUUIDv7`: valid v7, deterministic, version nibble 7, variant 10xx, embeds KSUID timestamp (second precision), works with `evt_` prefix, fallback timestamp deterministic for non-KSUID/empty input (posthog-uuidv7.test.ts).

### Diagnostics (diagnostics-*.test.ts)
- Export: POST to `.../v1/logs` OTLP JSON `resourceLogs[0].scopeLogs[0].logRecords[0]`, `body.stringValue` = raw message verbatim; resource attrs `agentcat.project_id` (or stable `agentcat.install_id` when no project), `agentcat.sdk.language = "typescript"`, `agentcat.sdk.version`, `os.type`, `process.runtime.name`, `process.runtime.version`, `agentcat.mcp_sdk.version` (v1, `^1.`), `agentcat.mcp_sdk_v2.version` (v2, `^2.`) (diagnostics-attributes.test.ts, diagnostics-export.test.ts:27-45, diagnostics-record.test.ts).
- Severity inference: "Warning: x" -> WARN; contains "Fail"/"error" -> ERROR (wins over Warning prefix); else INFO; empty -> INFO; `timeUnixNano` is digit string.
- Auth: default `Authorization: Bearer dgk_sdk_diag_...`; `DIAGNOSTICS_TOKEN` overrides; explicitly empty token => default token used (diagnostics-auth.test.ts:28-56 -- test title says "omits" but asserts default bearer is still sent; read the file before porting).
- Opt-out: disabled via option `disabled`/`disableDiagnostics`, env `DISABLE_DIAGNOSTICS` in {true,TRUE,1,yes,on}; {false,FALSE,0,no,off} keep it enabled AND override test-env detection; whitespace-only = unset; auto-disabled when `VITEST`, `JEST_WORKER_ID` or `NODE_ENV=test` (diagnostics-optout.test.ts). Fire-and-forget: fetch rejection never surfaces. Sink never receives payloads: no identity, no report-missing text (only `context length: N`); setup beacons `AgentCat setup started ... <projectId> ... sdk v1/high ... signals ...` and `AgentCat setup complete ... tracing=true ... reportMissing=true` (diagnostics-no-payload.test.ts:94-143).
- Every log line gets a version-info prefix (sdk, node, mcp versions), cached; skipped on Cloudflare Workers (navigator.userAgent "Cloudflare-Workers") (logging-version-prefix.test.ts). Log target: file `<homedir>/agentcat.log`; console only if fs/os modules unavailable (edge isolates); SILENT (never console, since stdout is the stdio JSON-RPC wire) when homedir is falsy/throws/not implemented (logging-target.test.ts:11-45).

### Detection / compatibility
- `detectServer` classifies: v1 high-level McpServer, v1 low-level Server, v2 high-level McpServer (no `.tool`, has `registerTool`), v2 low-level Server; `null` for non-servers and for shells with neither `.tool` nor `registerTool`; `describeSignals` lists only true signals; `fingerprintServerShape` returns `""` for null/non-objects (v2/detect.test.ts, detect-shapes.test.ts). Rejection logs `unrecognized server shape | signals <list>` ("(none)" when no signals) plus `SUPPORT_MATRIX_SUFFIX` and throws `/does not match any supported MCP SDK shape/` (compatibility.test.ts:71-112). Copy rule: message never says "v1.11 or higher".
- `track()` returns an incompatible object untouched instead of throwing (v2/options.test.ts:204); calling `track()` twice on one server is a no-op (v2/options.test.ts:186; tracing-initialization.test.ts:23).

---------------------------------------------------------------------------
## 3. Per-file inventory

Format: path (lines) -- summary. Then tests. "Pins:" lists exact values.

### Helpers
- `setup.ts` (6) -- vitest setup; sets `DISABLE_DIAGNOSTICS=true`.
- `test-utils.ts` (110) -- `LOG_FILE="agentcat.log"`; `sid(label)` builds valid 27-char session ids `ses_<label padded with 0 to 27>`; `cleanupLogFile`, `setupTestHooks` (delete log before/after each test); `EventCapture` class: `start()` wraps `eventQueue.add` (captures the event synchronously, then runs the real pipeline) and stubs `sendEvent` so no HTTP; `flush(timeoutMs=5000)` waits until every captured event has `id` (id is minted as the last pipeline step); `getEvents`, `clear`, `findEventByType`, `findEventsByResourceName`. Pins: flush default 5,000 ms; poll 5 ms.
- `test-utils/client-server-factory.ts` (157) -- builds real v1 McpServer/Server + Client over `InMemoryTransport` with todo tools (`add_todo`, `list_todos`, `complete_todo`), `resetTodos`, `setupTestServerAndClient`, etc. (used by most v1 tests).
- `v2/harness.ts` (36) -- `connectClient(server)` v2 client over linked in-memory pair (client name `agentcat-test-client` 1.0.0), `mintBackOf(result)` (reads content[0], must start with `[session_id`), `handleFrom(text,name)` regex `name: (\S+)`.
- `e2e-http/lanes.ts` (389) -- five real-HTTP lanes on ephemeral 127.0.0.1 ports at `/mcp`: `v1-stateful` (session map keyed by `mcp-session-id`, `sessionIdGenerator: randomUUID`, `enableJsonResponse:true`), `v1-stateless` (new server+transport per request, `sessionIdGenerator: undefined`; GET/DELETE -> 405), `v2-handler-legacy`, `v2-handler-auto` (createMcpHandler topologies), `v2-longlived` (NodeStreamableHTTPServerTransport). `E2E_PROJECT_ID="proj_test"`. Teardown order client -> transports/handler -> sockets -> server.
- `e2e-http/scenario-types.ts` (128) -- 22 curated scenario configs and shared fixtures: `SESSION_HOOK_VALUE="corr-7"`, `AGENT_SESSION_HOOK_VALUE="corr-agent-7"`, `CUSTOM_CONTEXT_DESCRIPTION="why?"`, `REDACTION_SECRET="SECRET_TOKEN"`, `REDACTION_REPLACEMENT="[REDACTED]"`, identity `e2e-user-1` / `E2E User` / `{plan:"enterprise"}`, customer tag `e2e_customer_tag=customer-tag-present`, colliding tag value `customer-collision-loses`, prop `e2e_customer_prop=prop-value`. Scenario names: baseline-defaults, tracing-off, tracing-off-x-report-missing-on, report-missing-off, report-missing-call, context-off, context-custom-description, agent-tracking-mint, agent-tracking-supplied, agent-tracking-x-hook, task-hook-deterministic, task-hook-null, task-supplied-continuity, mint-back-structured-mirror, strict-schema-injection, strip-proof-loose, error-path, hooks-identity-tags-props, redaction, client-identity-ladder, rebuild-on-demand (per-request lanes only: v1-stateless, v2-handler-legacy, v2-handler-auto), session-continuity-stateful (v1-stateful, v2-longlived only).
- `e2e-http/toolkit.ts` (178) -- shared tool set for e2e (v1 and v2 builders): `echo` (text of `msg`), `loose` (loose schema, returns JSON args), `strict`, `structured` (returns structuredContent), `boom` (throws); zod3 for v1, zod4 for v2.
- `workers/worker.ts` (70) -- workerd worker: `/mcp` handler and `/mcp-hook` handler (tool `echo`; `track(server,"proj_workers_test",{apiBaseUrl:"https://ingest.agentcat.test", disableDiagnostics:true, resolveSessionId/identify each sleeping 100 ms})`; server name `agentcat-workers-hook-test`).
- `workers/env.d.ts` (7) -- type declarations.

### Top-level tests (v1 project)
- `api-base-url.test.ts` (217) -- apiBaseUrl option, `EventQueue.configure()`, URL resolution precedence. Tests: option accepted/undefined; configure sets basePath; configure called with option / MCPCAT_API_URL / AGENTCAT_API_URL; AGENTCAT_API_URL beats legacy MCPCAT_API_URL; falls back to legacy; option beats both env vars; no configure() when neither set. Pins: env names `AGENTCAT_API_URL`, `MCPCAT_API_URL`.
- `basic-server.test.ts` (545) -- smoke of track() on real v1 servers. Tests: calls without tracking; server info from Server/McpServer; isCompatibleServerType uses server info; traces tools added after track(); shorthand Zod schemas (context injection regression); events published exactly once (no duplicates); tools defined BEFORE track() with shorthand Zod.
- `client-identity.test.ts` (226) -- see Identity above. describes: getClientInfoForRequest (5), getProtocolVersion (2), buildSessionInfo (2), envelope-first identity ladder (4), resolveIdentity (4: returns result; none -> null; throw -> null; runs on every call). Pins: `"io.modelcontextprotocol/clientInfo"`.
- `compatibility.test.ts` (550) -- isCompatibleServerType (null/undefined/non-object/missing setRequestHandler/_requestHandlers not Map/getClientVersion/_serverInfo{,.name}), unknown-shape error text, shape-fingerprint beacon, getMCPCompatibleErrorMessage (Error, custom props, string, plain object, null, undefined, number, boolean, circular, arrays), McpServer wrapper handling, logCompatibilityWarning, real McpServer integration. Pins: `unrecognized server shape | signals`, `(none)`, `hasServerProp`.
- `concurrency.test.ts` (88) -- interleaved concurrent tool calls keep their own session_id, agent_id, actor.
- `constants.test.ts` (96) -- see section 2 (byte-exact copy pins).
- `context-parameters-edge-cases.test.ts` (232) -- existing `context` param collision (skip + warn with tool name), oneOf/allOf/anyOf skipped, additionalProperties false removed / true preserved, `{}`/no inputSchema/type-only schemas, get_more_tools skipped by addContextParameterToTools, mixed batch.
- `context-parameters.test.ts` (448) -- addContextParameterToTools (no inputSchema, existing, no duplicate, empty required) and integration (context captured as `userIntent` only when provided, injected into tools/list, default description, removed before callback).
- `context-preservation.test.ts` (58) -- preserves a customer `context` param with its custom description; adds when absent.
- `custom-context-description.test.ts` (355) -- customContextDescription for JSON-schema and Zod tools, all tools in list, capture, default fallback, multi-call e2e, long descriptions, special characters.
- `debug-tool-schema.test.ts` (123) -- two diagnostic-style tests printing schema structure of shorthand / described tools (low-value, mostly console output).
- `detect-shapes.test.ts` (33) -- detectServer on fake shells: null when neither .tool nor registerTool; detects fake `.tool` (v1) and `registerTool` (v2).
- `diagnostics-attributes.test.ts` (68), `diagnostics-auth.test.ts` (56), `diagnostics-export.test.ts` (60), `diagnostics-integration.test.ts` (41; invalid server track() ships diagnostics; none when disabled), `diagnostics-no-payload.test.ts` (143), `diagnostics-optout.test.ts` (84), `diagnostics-record.test.ts` (62), `diagnostics-sink.test.ts` (29; sink forwards, sink throw swallowed, cleared sink stops), `diagnostics-test-env.test.ts` (41) -- see Diagnostics above.
- `e2e-sanitization.test.ts` (303) -- real MCP calls: image blocks, audio blocks, large base64 params sanitized in published events; normal todo calls preserved.
- `e2e-truncation.test.ts` (289) -- real calls: text >32KB truncated (<=32768+3), whole event <=102,400 bytes for oversized params, normal calls preserved, sanitization+truncation together (asserts the redaction placeholders on image blocks remain).
- `edge-runtime-compatibility.test.ts` (369) -- graceful degradation without fs/process (exception capture, process.cwd missing/throwing, process.once unavailable, process undefined, writeToLog, SDK exports track and publishCustomEvent, environment-detection patterns for Node / Cloudflare Workers / generic edge, path normalization without cwd, circular/deep error.cause).
- `engine-callwrap.test.ts` (654) -- tools/call wrapper (synthetic seam): MRTR helpers (`isInputRequiredShape` true only for `resultType==="input_required"`; `mrtrContinuationTags` => `{agentcat_mrtr:"continuation"}` only when `mcpReq.inputResponses` present), strip registry-scoped args, tag input_required rounds and skip mint-back, get_more_tools answered directly and published (not published on degraded path), `__agentcat_error` stack, degraded path calls original handler with UNSTRIPPED request when tracing setup fails, error event + rethrow, extra projection v1/v2 shapes, get_more_tools gating by enableReportMissing (3 paths), deferred customer hooks (hanging identify/resolveSessionId/eventTags/eventProperties never delay the call; handler runs first; sync throw contained; redactEvent sees resolved hook state; error path carries late hook results), registry rebuild bounding (hanging tools/list handler: rebuild bounded -- test advances fake timers 5,000 ms (engine-callwrap.test.ts:75) and expects log containing "rebuild-on-demand failed"; keeps serving calls if list handler fails every time), finish() fault containment (null content entry returns customer isError result; post-handler decoration throw returns raw customer result).
- `engine-innertap.test.ts` (217) -- wrapToolEntry (v2 executor in place, strips injected args, captures thrown errors to `extra.__agentcat_error`, v1 no-args arity, ctx position for schema-less v2, idempotent), rewrapAllTools, fault containment (enumeration throws, registry proxy set-trap, assignment throws), patchSetRequestHandler on v1 (Zod literal introspection; re-arm callback throw tolerated).
- `engine-listwrap.test.ts` (261) -- buildInjectedList (appends get_more_tools, names `["echo","get_more_tools"]`, registry sets, skip when enableReportMissing false, tracing-off, no mutation), installListWrap, tools/list fault containment (boolean schema, null entry, circular schema, BigInt schema, customer-owned get_more_tools warns once, whole pipeline failure returns the original list).
- `error-capture.test.ts` (659) -- end-to-end exception capture via real server (Error, cause chain, TypeError, non-Error thrown, one `it.skip` for in_app frame detection at :251, errors still propagate to client, identify throw publishes no identify event, success calls no error, invalid enum / unknown tool / missing required validation errors captured, exactly one event per call).
- `esm-consumer.test.ts` (75) -- packs the SDK tarball, installs into a temp dir via pnpm, runs `node` ESM import and asserts `track` is a function (prints "ok"); exit code 2 on failure.
- `event-tags-properties.test.ts` (617) -- eventTags/eventProperties callbacks (attach, collision, throw, null, validate), publishCustomEvent tags/properties, no callbacks, "initialize/tools-list is no longer an event", redaction bypass, callback args (request, extra).
- `eventQueue.test.ts` (676) -- see Event queue above; also singleton API (add, stats, destroy, no add after destroy), end-to-end processing, multiple events, redactEvent hook behaviors, SIGINT handling.
- `exceptions.test.ts` (863) -- captureException: basics, stack parsing, cause chain, non-Error, edge cases, path normalization (see above), backwards compatibility (abs_path kept).
- `extra-projection.test.ts` (125) -- projectExtraForEvent: v2 web Request -> `{method,url,headers}` lowercase headers, idempotent, returns original if headers.entries throws; v1 URL -> href; passthrough by identity; snapshot semantics.
- `handle-conflict-report.test.ts` (239) -- session_id collision reporting once per tool as an error with remediation; no dedupe set logs each pass; per-AgentCatData dedupe; customer-declared session_id tracking (records only tools whose own schema declares it, composed schema root, keyed per data object, nothing in hook mode).
- `handle-injection.test.ts` (539) -- see section 2. describes: addHandleParametersToTools (13), stripInjectedArguments (6), registry storage + request cloning (2), outputSchema injection (9).
- `handler-property.test.ts` (105) -- SDK exposes `callback` or `handler`; track() preserves the original property name (also for late registrations).
- `handles-integration.test.ts` (818) -- low-level and high-level servers with explicit handles (injection, no tools/list+initialize events, first call `start` mints and tags, echoed handles reused, subagent flow, isError mint-back, strip before handler, customer's own session_id never adopted, composed-schema tool still mints, hook mode, enableTracing:false => nothing, enableAgentTracking:false, enableReportMissing:false keeps handles, `_meta` clientInfo, outputSchema mcp_session) and "published events exclude SDK-authored mint-back text" (5 tests).
- `handles.test.ts` (804) -- primitives, see section 2 (largest pin source: golden session ids, mint-back text, structured mirror, isValidSessionId table).
- `identify.test.ts` (836) -- identify feature: stamps identity on event per call, no dedup, tools added after track(), persistence across calls, null/undefined, async, throwing, invalid data, tools/list does not identify or publish.
- `ksuid.test.ts` (229) -- vendored KSUID: randomSync/random (string length 27, raw 20 bytes, payload 16 bytes), fromParts (timestamp/payload validation; payload must be 16 bytes), encode/parse (MIN/MAX string constants), compare/equals, buffer validation (20 bytes), JSON/toString, epoch boundary, prefixed KSUIDs `^<prefix>_[0-9A-Za-z]{27}$` for event/user/task.
- `logging-target.test.ts` (45), `logging-version-prefix.test.ts` (77) -- see Diagnostics.
- `mcp-version-compatibility.test.ts` (22) -- single test "compatible with currently installed MCP version" (the CI matrix target).
- `posthog-exporter.test.ts` (696), `posthog-uuidv7.test.ts` (109) -- see PostHog.
- `publishCustomEvent.test.ts` (342) -- public API: with tracked server, `eventData.sessionId` is the session (not ambient), empty sessionId + warning when none, error data, throws if server untracked, high-level server objects, session-ID string form used verbatim and overridden by eventData.sessionId, all event data fields, validation (throws if projectId missing / invalid first param), event type always `agentcat:custom`, includes timestamp, undefined/empty event data.
- `redaction.test.ts` (1009) -- redactEvent (19 tests), applyEventRedaction (9), integration (3), event hook integration (4). Test fixture redact fn returns `[REDACTED-<len>]` style strings (e.g. "[REDACTED-23]").
- `report-missing.test.ts` (692) -- get_more_tools (named "report_missing" in older comments): injection, no context injection into it, execution with only context / description+context / missing description, event structure, task continuity, identity, analytics-value tests (multi-session, multiple reports per task).
- `sanitization.test.ts` (382) -- see Sanitization.
- `signal-shutdown.test.ts` (51) -- runSignalShutdown (4 tests).
- `structured-mintback.test.ts` (263) -- mint-back via `structuredContent.mcp_session` on V2 high-level and V1 low-level: structuredContent-only consumer discovers handles; steady state re-echo each response while the text block stays mint-only; wire-only (not on published event.response); content-only tools untouched; composed outputSchema => no mirror but text still delivered; hook mode mirrored payload has agent_id but no session_id/status; no tools/list yet => mirror still applies.
- `token-estimates.test.ts` (260), `tokenEstimate.test.ts` (151) -- see Token estimates.
- `tracing-initialization.test.ts` (148) -- no duplicate events when track() called repeatedly; events published for successful calls with handler-level wrapping.
- `truncation.test.ts` (527) -- see Truncation. describes: normalize string / non-serializable / depth / breadth / circular / undefined handling; truncateEvent field limits / error limits / response text / size targeting / non-mutation / edge cases / integration after sanitize.
- `validation.test.ts` (113) -- validateTags (11 tests), see section 2.

### v2/ (project `v2`, real @modelcontextprotocol/server + client over in-memory transport)
- `v2/agent-tracking.test.ts` (159) -- enableAgentTracking true: both handles required incl. get_more_tools; omission tolerated (task-only mint-back); self-chosen agent_id echoed with source `supplied`; subagent flow; OFF by default injects nothing.
- `v2/concurrency.test.ts` (95) -- interleaved calls keep attribution.
- `v2/detect.test.ts` (98) -- detectServer/describeSignals/fingerprintServerShape (see Detection).
- `v2/era-2026.test.ts` (105) -- `createMcpHandler` 2026-07-28 era: per-request instances, envelope identity, `agentcat_protocol_version="2026-07-28"` tag, rebuild-on-demand when instances never listed.
- `v2/get-more-tools.test.ts` (183) -- see section 2.
- `v2/hooks.test.ts` (580) -- identify/eventTags/eventProperties parity on v2, hooks receive raw request and v2 ServerContext, real-wire client identity.
- `v2/http-context-capture.test.ts` (115) -- v2 `extra.http.req` headers/method/url capture; stdio-shaped extra unchanged; malformed `req` (headers = 42) does not throw.
- `v2/late-registration.test.ts` (181) -- registerTool after track(): advertised with injected params, loose-schema stripping, error stacks via executor tap, identity.
- `v2/lowlevel-handles.test.ts` (314) -- low-level v2 Server: canonical order injection, first call `start`, strip, isError mint-back, rethrow, echo, invalid session => sessionless `invalid`, `_meta` and envelope client info.
- `v2/options.test.ts` (209) -- enableTracing false / enableReportMissing false / enableToolCallContext false / customContextDescription / track() edge paths.
- `v2/pipeline.test.ts` (251) -- redactSensitiveInformation, image sanitization, publishCustomEvent on v2 servers (session attribution, tag validation, empty properties omission, string session id verbatim, untracked throws, param validation).
- `v2/registration-patch.test.ts` (45) -- patchSetRequestHandler on v2 (string `tools/call` fires re-arm; 3-arg custom registrations forwarded intact; non-tool methods ignored).
- `v2/schema-edges.test.ts` (211) -- input and output schema edge cases (oneOf skip, default object schema, preserved customer params, additionalProperties false dropped, mcp_session output declared, agent_id omitted when tracking off, anyOf untouched, never overwrite customer `mcp_session`).
- `v2/session-modes.test.ts` (261) -- resolveSessionId hook mode (no session_id param; deterministic `ses_` id across calls and instances; no mint-back; supplied agent_id tagged supplied; hook null mints silently with source minted) and supplied-vs-minted modes (supplied tagged `supplied`, omission mints and announces, `start` case-insensitive).
- `v2/smoke.test.ts` (29) -- harness round trip.
- `v2/track-highlevel.test.ts` (168) -- track() on v2 McpServer: tools/list injection + mirrored outputSchema; capture, strip, mint-back, structuredContent mirror; echo of supplied session_id and error stacks via executor tap; re-wrap after `update()`.
- `v2/track-lowlevel.test.ts` (103) -- wraps string-method handlers, preserves `nextCursor`; rebuilds registries when tools/call arrives before tools/list.

### e2e-http/ (project `e2e-http`, real node:http; Node 20/22 in CI)
- `e2e-http/header-capture.test.ts` (116) -- sentinel header `x-agentcat-health-run` lands in `event.parameters.extra.http.req.headers` (v2 long-lived) and `extra.requestInfo.headers` with string `url` (v1 stateful).
- `e2e-http/sse-smoke-v1.test.ts` (160) -- baseline-defaults over streamed POST responses (`enableJsonResponse:false`).
- `e2e-http/sse-smoke-v2.test.ts` (150) -- baseline-defaults over real `text/event-stream`, v2 long-lived lane.
- `e2e-http/v1-matrix.test.ts` (843) -- `describe.each(lanes)` x `it.each(scenarios)` (names = scenario names above) for v1 lanes (v1-stateful, v1-stateless).
- `e2e-http/v2-matrix.test.ts` (854) -- same for v2 lanes (v2-handler-legacy, v2-handler-auto, v2-longlived).

### workers/ (separate config; needs build)
- `workers/event-delivery.test.ts` (189) -- in workerd: (1) ingestion stays alive via waitUntil after a v2 handler returns (fetch to `https://ingest.agentcat.test` held by a gate; any other outbound origin throws); (2) deferred hooks (100 ms sleeps) resolve inside waitUntil after the response returns; (3) genuine workerd Request headers captured (`x-agentcat-health-run: workers-sentinel`).

---------------------------------------------------------------------------
## 4. Source modules with no dedicated test or light coverage
(Based on import analysis of src/tests; "n" = number of test files importing the module.)

No test imports at all:
- `src/modules/exporters/datadog.ts` (234 lines), `exporters/otlp.ts` (160), `exporters/sentry.ts` (543), `exporters/trace-context.ts` (34) -- ZERO tests. Only PostHog has an exporter test. Biggest gap (Sentry alone is 543 lines).
- `src/modules/telemetry.ts` (62) -- no test imports it (only mocked/touched indirectly by eventQueue tests: "does not register telemetry-only events").
- `src/modules/tools.ts` -- no direct import; only the `get_more_tools` behavior is asserted through engine/e2e tests.
- `src/modules/pendingEvent.ts` (101) -- no direct import; covered indirectly via deferred-hook tests in engine-callwrap.test.ts:400-523 and v2/hooks.
- `src/modules/mcp-sdk-compat.ts` (155) -- no direct test; exercised indirectly through compatibility/detect tests.
- `src/index.workerd.ts` -- no direct import; covered only by the opt-in workers project (needs `pnpm build`), which CI ci.yml does not run.
- `src/adapters/v1.ts` (1 test file), `src/adapters/v2.ts` (3) -- indirect only.

Lightly tested (1-2 importing test files, mostly indirect):
- `src/engine/callWrap.ts`, `innerTap.ts` (1 each, but each has its own dedicated file); `listWrap.ts`, `registrationPatch.ts`, `registry.ts` have dedicated tests (engine-listwrap, v2/registration-patch).
- `src/modules/backgroundTasks.ts` (1: eventQueue.test.ts), `runtime-versions.ts` (1: logging-version-prefix), `session.ts` (2), `internal.ts` (9 but only as plumbing).
- `src/detect.ts` has dedicated tests (v2/detect.test.ts, detect-shapes.test.ts).
- `src/thirdparty/ksuid/*` -- dedicated ksuid.test.ts.
- Behavior with no pinned default: EventQueue defaults for retries, concurrency, queue size, retry backoff, request timeout, and HTTP headers sent to the AgentCat API are never asserted (only overridden). The default API base URL is never asserted. Timeouts for hooks (identify/resolveSessionId/eventTags/eventProperties) are not pinned beyond the 5,000 ms registry-rebuild bound.
- `src/modules/index.ts` / `src/index.ts` / `src/types.ts` are imported by nearly all tests (barrel/type files).

Other observations:
- Some stale text: diagnostics-attributes comments say devDep ~1.30.0 (package.json: ~1.31.0).
- `error-capture.test.ts:251` is `it.skip` (in_app frame detection).
- diagnostics-auth.test.ts:45 title says "omits the Authorization header when token is explicitly empty" but asserts the default bearer `dgk_sdk_diag_...` is still used (line 54).
- Tests rely heavily on `vi.mock` of `agentcat-api` (EventsApi, Configuration) and the logging module; real wire HTTP to the AgentCat API is never exercised (only fetch mocks).

# TS test index (part C) for the Rust port

Source: `agentcat-typescript-sdk` at commit `9b56e9e`, package `agentcat`
`2.1.1-beta.1` (the committed `package.json` says `2.1.1`; the working tree
says `2.1.1-beta.1`). All paths below are relative to
`/home/ubuntu/agentcat-typescript-sdk/src/tests/` unless they start with
`src/`.

Scope: `redaction.test.ts`, `report-missing.test.ts`, `sanitization.test.ts`,
`setup.ts`, `signal-shutdown.test.ts`, `structured-mintback.test.ts`,
`test-utils.ts`, `test-utils/client-server-factory.ts`,
`token-estimates.test.ts`, `tokenEstimate.test.ts`,
`tracing-initialization.test.ts`, `truncation.test.ts`, `validation.test.ts`,
every file in `v2/`, and every file in `workers/`.

Counting rule: one row per `it`/`test`. Each row of an `it.each` table is its
own row, because Vitest reports each row as its own test.

Method: every file was read in full. The "Actual" values in the truncation,
sanitization and token rows that the tests do not assert were produced by
running the TS modules (`src/modules/truncation.ts`, `sanitization.ts`,
`tokenEstimate.ts`, `redaction.ts`, `handles.ts`), transpiled with the repo's
own esbuild into a scratch folder. Nothing in the TS repo was changed.

## Summary

**292 tests** in 28 test files, plus 6 helper files with no tests.

| Rust column | Tests |
| --- | --- |
| `port` | 214 |
| `adapt` | 34 |
| `N/A` | 44 |
| **Total** | **292** |

| File | Tests | port | adapt | N/A |
| --- | --- | --- | --- | --- |
| `redaction.test.ts` | 33 | 22 | 5 | 6 |
| `report-missing.test.ts` | 12 | 11 | 1 | 0 |
| `sanitization.test.ts` | 19 | 19 | 0 | 0 |
| `setup.ts` | 0 | - | - | - |
| `signal-shutdown.test.ts` | 4 | 0 | 3 | 1 |
| `structured-mintback.test.ts` | 9 | 8 | 1 | 0 |
| `test-utils.ts` | 0 | - | - | - |
| `test-utils/client-server-factory.ts` | 0 | - | - | - |
| `token-estimates.test.ts` | 6 | 4 | 2 | 0 |
| `tokenEstimate.test.ts` | 39 | 35 | 0 | 4 |
| `tracing-initialization.test.ts` | 2 | 1 | 1 | 0 |
| `truncation.test.ts` | 49 | 33 | 1 | 15 |
| `validation.test.ts` | 11 | 10 | 1 | 0 |
| `v2/agent-tracking.test.ts` | 5 | 5 | 0 | 0 |
| `v2/concurrency.test.ts` | 1 | 1 | 0 | 0 |
| `v2/detect.test.ts` | 9 | 0 | 0 | 9 |
| `v2/era-2026.test.ts` | 2 | 0 | 2 | 0 |
| `v2/get-more-tools.test.ts` | 5 | 5 | 0 | 0 |
| `v2/harness.ts` | 0 | - | - | - |
| `v2/hooks.test.ts` | 20 | 16 | 4 | 0 |
| `v2/http-context-capture.test.ts` | 3 | 0 | 2 | 1 |
| `v2/late-registration.test.ts` | 4 | 2 | 2 | 0 |
| `v2/lowlevel-handles.test.ts` | 11 | 9 | 2 | 0 |
| `v2/options.test.ts` | 10 | 8 | 1 | 1 |
| `v2/pipeline.test.ts` | 9 | 7 | 2 | 0 |
| `v2/registration-patch.test.ts` | 3 | 0 | 0 | 3 |
| `v2/schema-edges.test.ts` | 8 | 7 | 1 | 0 |
| `v2/session-modes.test.ts` | 8 | 8 | 0 | 0 |
| `v2/smoke.test.ts` | 1 | 0 | 1 | 0 |
| `v2/track-highlevel.test.ts` | 4 | 2 | 1 | 1 |
| `v2/track-lowlevel.test.ts` | 2 | 1 | 1 | 0 |
| `workers/env.d.ts` | 0 | - | - | - |
| `workers/worker.ts` | 0 | - | - | - |
| `workers/event-delivery.test.ts` | 3 | 0 | 0 | 3 |

How the suites run in TS: `vitest.config.ts` has three projects (`v1` =
`src/tests/**/*.test.ts` minus `v2/`, `e2e-http/`, `workers/`; `v2` =
`src/tests/v2/**`; `e2e-http`). All use `setupFiles: ["./src/tests/setup.ts"]`
and run in one fork, one file after another. `pnpm test` runs `v1` then `v2`.
The `workers/` suite runs only through `pnpm test:workers`
(`vitest.workers.config.ts`, Cloudflare pool, `wrangler.workers-test.jsonc`).

### Pipeline facts the Rust port must reproduce byte for byte

The queue (`src/modules/eventQueue.ts:100-166`) runs these stages in this order
on every event. A failure in stages 2 to 5 logs one line and drops the event.
A failure in stage 0 is logged, and processing continues.

1. **Stage 0, deferred hooks** (`src/modules/pendingEvent.ts:140-182`). The
   four hooks were started on the request path. Each gets a 30 000 ms budget.
   - `resolveSessionId`: a non-empty trimmed string gives
     `deriveSessionId(value.trim(), projectId)` and tag
     `agentcat_session_id_source: "hook"`. Null, throw or timeout gives a fresh
     random `ses_` id and `"minted"`.
   - `identify`: when the hook returns an object, `identifyActorGivenId =
     userId`, `identifyActorName = userName`, `identifyActorData = userData ||
     {}`. When it returns null or throws, the event keeps the anonymous
     defaults: given id and name absent, `identifyActorData = {}`.
   - `eventTags`: `validateTags(result)`, then merged UNDER the SDK tags
     (`{...customer, ...sdk}`), so an SDK tag key always wins.
   - `eventProperties`: the value as returned (null means absent).
2. **Event-level hook `redactEvent`** (`src/modules/redaction.ts:171-199`). The
   hook gets the event without `redactionFn`/`eventRedactionFn`. `null` or
   `undefined` drops the event (log `Event dropped by redactEvent hook`). A
   throw drops it (log `Failed to redact event (event-level hook): <err>`).
   Otherwise the returned object replaces every field (deleted fields stay
   deleted), then `id`, `sessionId`, `projectId`, `eventType`, `timestamp` are
   restored from the original (deleted if the original had none).
3. **String hook `redactSensitiveInformation`**
   (`src/modules/redaction.ts:48-141`). The function runs on every string in
   the event, including `""`. Strings under these TOP-LEVEL keys are skipped,
   at any depth below them: `sessionId`, `id`, `projectId`, `server`,
   `serverName`, `serverVersion`, `clientName`, `clientVersion`,
   `identifyActorGivenId`, `identifyActorName`, `identifyData`,
   `resourceName`, `eventType`, `actorId`, `tags`, `properties`. A nested key
   with one of these names is NOT protected. `Date` values, numbers, booleans
   and `null` pass through. Keys whose value is a function or `undefined` are
   dropped. A throw drops the event (log `Failed to redact event: <err>`).
   Strings that ARE redacted include `identifyActorData` (the real identity
   field, see the redaction quirks), `userIntent`, `eventId`, `sdkLanguage`,
   `agentcatVersion`, `ipAddress`, everything in `parameters` (including
   `request.method` and HTTP headers), `response` (including each content
   block's `type`), and `error`.
4. **Sanitization** (`src/modules/sanitization.ts`), on the full-length
   strings:
   - Each entry of `response.content` (top level only):
     - `type: "text"`: unchanged.
     - `type: "image"`: `{"type":"text","text":"[image content redacted - not supported by AgentCat]"}`.
     - `type: "audio"`: `{"type":"text","text":"[audio content redacted - not supported by AgentCat]"}`.
     - `type: "resource"` whose `resource.blob !== undefined`:
       `{"type":"text","text":"[binary resource content redacted - not supported by AgentCat]"}`.
       A resource with `text` is unchanged.
     - `type: "resource_link"`: unchanged.
     - Any other `type`, including a missing one:
       `{"type":"text","text":"[unsupported content type \"<String(type)>\" redacted - not supported by AgentCat]"}`.
       A missing type prints `undefined`.
     - A `null` or non-object entry: unchanged.
   - `response.structuredContent` (non-null object) and `parameters`: walked
     recursively. A string whose `.length >= 10240` and which matches
     `/^[A-Za-z0-9+/\n\r]+=*$/` becomes
     `"[binary data redacted - not supported by AgentCat]"`.
   - Nothing else is scanned: not other `response` keys, not `error`, not
     `userIntent`.
5. **Truncation** (`src/modules/truncation.ts`). The result is a new object.
   - Layer 1, field limits. If `.length > limit`, the value becomes
     `slice(0, limit) + "..."`, so the output length is limit + 3. A value
     exactly at the limit is unchanged.
     - `userIntent`: 2048.
     - `resourceName`, `serverName`, `serverVersion`, `clientName`,
       `clientVersion`: 256.
     - `error.message`: 2048.
     - `error.frames`: if there are more than 50, keep the first 25 and the
       last 25.
     - Each `response.content[i]` with `type: "text"` and a string `text`:
       32768.
   - Layer 2, `normalize(value, depth = 10, breadth = 100, maxString =
     32768)` on `parameters`, `response`, `identifyActorData` and `error`
     (when each is non-null).
     - Strings over 32768: `slice + "..."`.
     - JS-only markers: `"[undefined]"`, `"[NaN]"`, `"[Infinity]"`,
       `"[-Infinity]"`, `"[BigInt: <n>]"`, `"[Symbol(<desc>)]"`,
       `"[Symbol()]"`, `"[Function: <name>]"`, `"[Function: <anonymous>]"`.
       A `Date` becomes its ISO string; an invalid one becomes
       `"[Invalid Date]"`.
     - A cycle (an ancestor seen again) becomes `"[Circular ~]"`.
     - A container at nesting level k (the root is level 1) is expanded only
       if k <= depth. A deeper container becomes `"[Array]"` or `"[Object]"`.
     - Breadth: an array keeps its first N items, then pushes
       `"[MaxProperties ~]"`. An object keeps its first N defined keys, then
       adds key `"..."` with value `"[MaxProperties ~]"`. Keys whose value is
       `undefined` are skipped and not counted, but the breadth check runs
       before that skip.
   - Layer 3, size budget. Size = UTF-8 bytes of `JSON.stringify(event)`, on
     the INTERNAL event object (camelCase keys, ISO timestamps, `tags`,
     `properties`, `identifyActorData`), not on the wire request.
     - If size <= 102400, return.
     - Else for depth = 9 down to 1: re-normalize the four fields at that
       depth. Return the first result that fits.
     - Else normalize the four fields at depth 1 and run
       `truncateLargestFields` (up to 10 passes). In each pass:
       - Collect every string longer than 100 chars anywhere in the event.
       - Sort by length, longest first. The sort is stable, so ties keep
         traversal order.
       - Set `remaining = (size - 102400) + 200`.
       - For each string, `reduction = min(remaining, floor(len * 0.5))`.
         Skip it if `reduction < 10`. Otherwise the new value is
         `s.slice(0, len - reduction) + "..."` and `remaining -= reduction`.
       - Stop when `remaining <= 0`.
       - Stop the passes when one pass truncates nothing.
6. `id = id || KSUID("evt")`, then send.

### Other facts the tests pin

**Token estimates** (`src/modules/tokenEstimate.ts`):
- `tokens = ceil(utf8Bytes / 3.5)`, `0` when bytes are not > 0, clamped to
  `2147483647`.
- `inputTokens` is computed on the path, from the RAW `request.params.arguments`
  (injected `session_id`/`agent_id`/`context` included), as compact
  `JSON.stringify` bytes. It is absent when the arguments are null or
  undefined, or cannot be serialized.
- `outputTokens` is computed on the customer's ORIGINAL result, before the
  mint-back and before every queue stage:
  - Absent when the result is null or undefined.
  - When `content` is not an array: the bytes of the whole response JSON.
  - When `content` is `[]`: the JSON bytes of `structuredContent` if it is
    non-null, else 0.
  - Otherwise the sum of `text` bytes of `type:"text"` blocks plus
    `resource.text` bytes of `type:"resource"` blocks. Other blocks count 0,
    and `structuredContent` is then ignored.

**Mint-back** (`src/modules/handles.ts:100-133`). The block is the FIRST
content element. It is never recorded on the event.
- Issued: `"[session_id issued — see this tool's session_id parameter description]\nsession_id: <ses_id>\nThis is the first-call issuance described in this tool's session_id parameter description."`
  (the dash is U+2014).
- Unrecognized: `"[session_id unrecognized — see this tool's session_id parameter description]\nThe value sent was not issued by this server. Re-send the session_id issued earlier for this task; if none was issued yet, send start and one will be issued."`
- Structured mirror: `structuredContent = {"mcp_session": {...}, ...customer}`,
  with `mcp_session` as the first key. Its sub-keys appear in the order
  `session_id`, `agent_id`, `status`. `status` is `issued`, `active` or
  `unrecognized`.

**Session id derivation** (`src/modules/handles.ts:35-49`):
- `sha256("<id>:<projectId>")`.
- KSUID time = `Date.UTC(2024,0,1) + (readUInt32BE(hash,0) % 31536000000)` ms,
  converted to KSUID seconds since `1.4e12` ms.
- KSUID payload = `hash[4..20]`, base62, prefix `ses_`.
- Golden values from the TS code:
  - `deriveSessionId("customer-correlation-7","proj_test")` = `ses_2cVoolHakk4pa3LZQSgvJRkTwcK`.
  - `deriveSessionId("customer-correlation-8","proj_test")` = `ses_2br1PpV6hNsjOTG3pZ1pECw1pu4`.
  - `deriveSessionId("corr-1","proj_test")` = `ses_2brsWjk74N1K8PjsmWKQitu7puu`.
- Valid id shape: `/^ses_[0-9A-Za-z]{27}$/`. The advertised pattern is
  `^(start|ses_[0-9A-Za-z]{27})$`. `start` is matched after trim, without
  regard to case.

### Shared helpers and fixtures a Rust suite needs

1. **Event capture** (`test-utils.ts` `EventCapture`). TS wraps `eventQueue.add`
   to record each event object at enqueue time, then lets the real pipeline
   run, and replaces `sendEvent` with a no-op. Because the TS pipeline mutates
   the queued object in place, assertions see post-pipeline values once
   processing ends.
   - `flush(timeoutMs = 5000)` polls every 5 ms until every captured event
     has a non-empty `id` (minted just before send). Otherwise it throws
     `EventCapture.flush timed out waiting for captured events to finish processing`.
   - `getEvents()` returns a copy. `clear()`, `findEventByType(t)` and
     `findEventsByResourceName(n)` exist.
   - Rust equivalent: a test sink on the publisher that records each event
     after the pipeline, plus a counter of events enqueued versus finished
     (sent or dropped). `flush` waits until the counter is zero. Dropped
     events must count as finished: the TS `flush` hangs until timeout on a
     dropped event, which is why the drop test uses a send-boundary spy
     instead.
2. **Send-boundary spy** (`redaction.test.ts` drop test). TS replaces
   `eventQueue.sendEvent` to record only events that survive the pipeline.
   Rust: the same sink, read after `flush`.
3. **v1 todo fixture** (`test-utils/client-server-factory.ts`), with exact
   wording:
   - Server `McpServer({name:"test server", version:"1.0"})`. Client
     `{name:"test client", version:"1.0"}` with capability `sampling: {}`.
   - The client's `sampling/createMessage` handler returns
     `{model:"test-model", role:"assistant", content:{type:"text", text:"This is a test response"}}`.
   - Transport: an in-memory linked pair. `cleanup()` closes both halves.
   - Todo state lives at module level. `resetTodos()` clears it and resets ids
     to start at `"1"`.
   - `add_todo`, "Add a new todo item", input `{text: string "The text of the todo item"}`.
     Returns text `Added todo: "<text>" with ID <id>`.
   - `list_todos`, "List all todo items", no input. Returns the lines
     `<id>: <text> ✓` (done) or `<id>: <text> ○` (open), joined with `\n`,
     or `No todos found`.
   - `complete_todo`, "Mark a todo item as completed", input
     `{id: string "The ID of the todo to complete"}`. Returns
     `Completed todo: "<text>"`. Throws `Todo with ID <id> not found`.
   - `get_stats`, "Get todo stats", input `{}`, output
     `{count: number "Number of todos"}`. Returns text `<n> todos` and
     `structuredContent {count: n}`.
   - Rust: use per-server state, not module state.
4. **v2 client harness** (`v2/harness.ts`).
   - `connectClient(server)` connects a Client
     `{name:"agentcat-test-client", version:"1.0.0"}` over an in-memory pair.
   - `mintBackOf(result)` returns `content[0].text` only if `content[0]` is a
     text block that starts with `[session_id`. It asserts the placement.
   - `handleFrom(text, name)` returns group 1 of `` `${name}: (\S+)` ``.
   - Rust: a `tokio::io::duplex` pair with rmcp client and server; the same
     two helpers.
5. **`sid(label)`** (`test-utils.ts`):
   `"ses_" + (label.replace(/[^0-9A-Za-z]/g,"") + "0".repeat(27)).slice(0,27)`.
   Values used:
   - `fixed` → `ses_fixed0000000000000000000000`
   - `parent` → `ses_parent000000000000000000000`
   - `session_A` → `ses_sessionA0000000000000000000` (B and C follow the same pattern)
   - `report_missing_continuity` → `ses_reportmissingcontinuity0000`
   - `report_missing_workflow` → `ses_reportmissingworkflow000000`
   - `gmt_continuity` → `ses_gmtcontinuity00000000000000`
   - `x` → `ses_x00000000000000000000000000`
   - `abc` → `ses_abc000000000000000000000000`
   - `late` → `ses_late00000000000000000000000`
   - `late_err` → `ses_lateerr00000000000000000000`
   - `boom` → `ses_boom00000000000000000000000`
   - `redact` → `ses_redact000000000000000000000`
   - `ctxoff` → `ses_ctxoff000000000000000000000`
   - `supplied` → `ses_supplied0000000000000000000`
   - `fixed123` → `ses_fixed1230000000000000000000`
6. **Direct handler calls.** Six tests (`structured-mintback` row 9,
   `v2/http-context-capture` rows 1-3, `v2/lowlevel-handles` rows 10-11) call
   the wrapped `tools/call` handler directly (`server._requestHandlers.get("tools/call")(request,
   extra)`) to control `_meta`, the v2 envelope, the HTTP request, or to skip
   `tools/list`. Rust needs a way to call `Tracked::call_tool` with a
   synthetic `RequestContext` (request id, `meta`, `extensions` holding
   `http::request::Parts`).
7. **Tag split** (`v2/hooks.test.ts`). `SDK_TAG_KEYS = [agentcat_session_id_source,
   agentcat_agent_id, agentcat_agent_id_source, agentcat_protocol_version]`.
   `customerTags()` removes these keys. `sdkTags()` keeps only these keys.
8. **Mock string redactor**: `async t => "[REDACTED-" + t.length + "]"`.
9. **Event factories** for the unit tests:
   - Sanitization: `makeEvent = {id:"evt_1", sessionId:"ses_1", eventType:"mcp:tools/call", timestamp:new Date(), ...overrides}`.
   - Truncation: the same, with `timestamp: new Date("2025-01-15T12:00:00Z")`.
   - `makeLargeBase64(n=12000) = "A".repeat(n-1) + "="`.
   - `makeLargeNonBase64(n=12000) = "Hello, world! This is NOT base64. ".repeat(ceil(n/34))`
     (34-char unit, 353 copies, 12002 chars).
10. **Log capture.** `validation.test.ts` mocks `writeToLog`.
    `v2/track-lowlevel.test.ts` reads `~/agentcat.log` from a byte offset.
    Rust: capture `tracing` output in tests.
11. **Diagnostics off.** `setup.ts` sets `DISABLE_DIAGNOSTICS=true` before any
    import. The Rust suite must also never reach the live collector.
12. **Mock ingest server** (Workers tests' `vi.stubGlobal("fetch")`, which
    answers `202`). Rust: a local HTTP mock at `api_base_url` to assert the
    serialized body.
13. **Exact constants used in assertions** (`src/modules/constants.ts`,
    `src/modules/tools.ts`):
    - `DEFAULT_CONTEXT_PARAMETER_DESCRIPTION` = `Explain why you are calling this tool and how it fits into the user's overall goal. This parameter is used for analytics and user intent tracking. YOU MUST provide 15-25 words (count carefully). NEVER use first person ('I', 'we', 'you') - maintain third-person perspective. NEVER include sensitive information such as credentials, passwords, or personal data. Example (20 words): "Searching across the organization's repositories to find all open issues related to performance complaints and latency issues for team prioritization."`
    - `get_more_tools`:
      - description: `Check for additional tools whenever your task might benefit from specialized capabilities - even if existing tools could work as a fallback.`
      - `inputSchema`: `{type:"object", properties:{context:{type:"string", description:"A description of your goal and what kind of tool would help accomplish it."}}, required:["context"]}`
      - `annotations`: `{title:"Get More Tools", readOnlyHint:true, destructiveHint:false, idempotentHint:true, openWorldHint:false}`
    - Canned answer: `Unfortunately, we have shown you the full tool list. We have noted your feedback and will work to improve the tool list in the future.`
    - Tag keys: `agentcat_agent_id`, `agentcat_session_id_source`,
      `agentcat_agent_id_source`, `agentcat_protocol_version`, `agentcat_mrtr`.
    - Custom event type: `agentcat:custom`. Tool call type: `mcp:tools/call`.

### Cross-cutting porting hazards these tests expose

- **UTF-16 versus bytes.** Every TS length limit counts UTF-16 code units
  (`.length`, `.slice`): field limits, the 32768 string cap, the 10240 base64
  gate, the 100-char last-resort threshold, and tag limits of 32 and 200. Only
  the 102400 budget and token counts use UTF-8 bytes. No test in scope uses
  non-ASCII text in a truncated string. Rust must pick a rule. For identical
  results, count UTF-16 units (`encode_utf16().count()`). TS can split a
  surrogate pair; Rust must cut on a char boundary.
- **Key order is asserted.** These tests assert order:
  - Input properties `["text","session_id","agent_id","context"]`.
  - `mcp_session` first in `structuredContent`, and its sub-keys `["session_id","agent_id","status"]`.
  - Breadth limits keep the first N keys in insertion order.
  - The tag cap keeps the first 50 valid entries.
  - Last-resort ties break by traversal order.

  This needs `serde_json` `preserve_order` (or `IndexMap`) wherever these
  values are built.
- **Number formatting.** `JSON.stringify(1.0)` gives `1`; `serde_json` gives
  `1.0` for an `f64`. This changes token counts and the size budget for
  float-valued arguments.
- **Absent, null and empty are distinct on the wire.**
  - `isError` is absent on success, not `false`.
  - Anonymous events carry `identifyActorData: {}`.
  - `sessionId: ""` publishes as `null`.
  - Empty `properties` on a custom event is dropped.
  - `parameters: null` stays `null`.

---

### src/tests/redaction.test.ts (33 tests)
Purpose: Tests the string redaction walk (`redactEvent`), the event-level hook (`applyEventRedaction`), and both hooks end to end on a tracked v1 server.
Harness: The `redactEvent` and `applyEventRedaction` suites call the module functions directly on hand-built `UnredactedEvent` objects. They use the mock redactor `vi.fn(async t => "[REDACTED-"+t.length+"]")`, which is cleared before each test. The two integration suites build the v1 todo fixture per test (`resetTodos()` + `setupTestServerAndClient()`, `cleanup()` after). They call `track(server, "test-project", {...})` and send `tools/call` through `client.request(..., CallToolResultSchema)`. They wait 100 ms, then read `EventCapture.getEvents()`. The drop test replaces `eventQueue.sendEvent` with a recorder, so it sees only events that reach the send step. `applyEventRedaction` uses a `baseEvent()` with: id `"evt_original"`, sessionId `"ses_123000000000000000000000000"`, projectId `"proj_789"`, eventType `"mcp:tools/call"`, timestamp `new Date("2024-01-01")`, resourceName `"add_todo"`, parameters `{text:"sensitive text"}`, response `{content:"sensitive response"}`, userIntent `"sensitive intent"`.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | redactEvent > should redact basic string fields | `{sessionId:"ses_123000000000000000000000000", userIntent:"sensitive user intent", timestamp:Date}` → `userIntent` `"[REDACTED-21]"`. The fn is called with `"sensitive user intent"`. | port |
| 2 | redactEvent > terminates on circular structures, preserving the cycle in the clone | Input: `parameters.request = req`, where `req = {url:"/checkout", token:"secret"}` and `req.res = {statusText:"oops", req}`. Output: `request.url` `"[REDACTED-9]"`, `request.res.statusText` `"[REDACTED-4]"`, and `request.res.req` is the same object as the cloned `request`. | N/A: `serde_json::Value` is a tree and cannot hold cycles |
| 3 | redactEvent > redacts a shared (non-circular) reference once and reuses the clone | `parameters {a: shared, b: shared}` with `shared.note "shared secret"` → `a.note` `"[REDACTED-13]"`, and `a` and `b` are the same object. | N/A: `Value` has no shared references |
| 4 | redactEvent > should not redact protected fields | These keep their values: sessionId, `id "evt_456"`, `projectId "proj_789"`, `server "my-server"`, `serverName "my-mcp-server"`, `serverVersion "1.2.3"`, `clientName "my-mcp-client"`, `clientVersion "4.5.6"`, `identifyActorGivenId "actor_123"`, `identifyActorName "John Doe"`, `resourceName "my-resource"`, `eventType "mcp:tools/call"`, `actorId "actor_456"`. The fn is never called. | port (the protected set must name the Rust event's fields) |
| 5 | redactEvent > should redact nested string fields | `parameters.query "sensitive query"` → `"[REDACTED-15]"`. `options.apiKey "secret-key-123"` → `"[REDACTED-14]"`. `options.timeout` stays `5000`. `response.data "sensitive response data"` → `"[REDACTED-23]"`. `response.metadata.source "sensitive source"` → `"[REDACTED-16]"`. | port |
| 6 | redactEvent > should handle arrays of strings | `tags ["sensitive1","sensitive2","sensitive3"]` (inside `parameters`) → `["[REDACTED-10]","[REDACTED-10]","[REDACTED-10]"]`. `numbers [1,2,3]` is unchanged. | port |
| 7 | redactEvent > should handle mixed nested structures | `users[0] {name "John", age 30, email "john@example.com"}` → `"[REDACTED-4]"`, `30`, `"[REDACTED-16]"`. `users[1] {name "Jane", age 25, email "jane@example.com"}` → `"[REDACTED-4]"`, `25`, `"[REDACTED-16]"`. | port |
| 8 | redactEvent > should preserve null and undefined values | `nullValue: null` stays null. `undefinedValue: undefined` stays undefined. `emptyString: ""` → `"[REDACTED-0]"` (the fn is called on the empty string). | port |
| 9 | redactEvent > should preserve Date objects | `timestamp` equals `2024-01-01T12:00:00Z`. `parameters.createdAt` is still a `Date` equal to `2024-01-02T12:00:00Z`. | adapt: the Rust timestamp is typed and never redacted; `Value` has no Date type |
| 10 | redactEvent > should skip functions in objects | `parameters.callback` (a function) → undefined. `data "sensitive data"` → `"[REDACTED-14]"`. | N/A: no function values in Rust data |
| 11 | redactEvent > should handle complex identifyData object without redacting it | Under `identifyData`, `email "user@example.com"`, `preferences.theme "dark"`, `notifications true`, `metadata.source "webapp"` and `version "1.0.0"` are unchanged. The fn is not called with those strings. | adapt: decide which Rust identity field is protected (see quirk 1) |
| 12 | redactEvent > should handle error objects | `isError` stays `true`. `error.message "Sensitive error message"` → `"[REDACTED-23]"`. `code "ERR_001"` → `"[REDACTED-7]"`. `stack "Error: Sensitive error message\n    at functionName (file.js:10:5)"` (65 chars) → `"[REDACTED-65]"`. | port |
| 13 | redactEvent > should handle deeply nested protected fields correctly | The top-level sessionId is unchanged. `parameters.nested.deeply.sessionId "This should be redacted"` → `"[REDACTED-23]"`. `data` → `"[REDACTED-14]"`. | port |
| 14 | redactEvent > should handle custom redaction function errors gracefully | The fn throws `Error("Redaction failed")` → `redactEvent` rejects with `"Redaction failed"`. | adapt: the redactor returns `Err` or panics (`catch_unwind`); the queue drops the event and logs `Failed to redact event: ...` |
| 15 | redactEvent > should not redact nested fields within any protected field | `identifyData.nested.deeply.sensitive "this should NOT be redacted"` and `data ["array","of","strings"]` are unchanged. `otherData.nested.deeply.sensitive "this SHOULD be redacted"` → `"[REDACTED-23]"`. The fn is called only for that string. | port |
| 16 | redactEvent > should create a new object without modifying the original | The original deep-equals its JSON copy. In the output, `userIntent` and `parameters.query` differ from the input. | port |
| 17 | redactEvent > should handle events with all possible fields | Kept: `id "evt_123"`, `projectId "proj_123"`, sessionId, `actorId "actor_123"`, `eventType`, `resourceName "resource_123"`, `identifyActorGivenId "given_id_123"`, `identifyActorName "Actor Name"`, `server "server_name"`, `identifyData.key "value"`. Redacted: `eventId "custom_evt_123"` → `"[REDACTED-14]"`, `userIntent "user intent text"` → `"[REDACTED-16]"`, `parameters.param1 "value1"` → `"[REDACTED-6]"`, `response.result "success"` → `"[REDACTED-7]"`, `error.message "error message"` → `"[REDACTED-13]"`. `isError false`, `duration 1000` and the Date `timestamp` are unchanged. | port |
| 18 | applyEventRedaction > should apply a synchronous hook in place | The hook returns `{...event, parameters:{text:"[SCRUBBED]"}}` → the call returns `true`. `event.parameters` is `{text:"[SCRUBBED]"}`. `response` is still `{content:"sensitive response"}`. | port |
| 19 | applyEventRedaction > should apply an asynchronous hook in place | An async hook sets `userIntent "[SCRUBBED]"` → the call returns `true` and `event.userIntent` is `"[SCRUBBED]"`. | adapt: depends on the Rust hook signature (03 proposes sync closures) |
| 20 | applyEventRedaction > should report a drop and leave the event untouched when the hook returns null | The hook returns `null` → the call returns `false` and the event deep-equals `baseEvent()`. | port |
| 21 | applyEventRedaction > should report a drop when the hook returns undefined | The hook returns `undefined` → the call returns `false`. | N/A: Rust has one `None`, which row 20 covers |
| 22 | applyEventRedaction > should honor field deletions instead of resurrecting them | The hook deletes `response` and `userIntent` → the call returns `true`. Both properties are gone. `parameters` is still `{text:"sensitive text"}`. | port |
| 23 | applyEventRedaction > should restore system-managed fields if the hook changes or removes them | The hook sets `id "evt_forged"`, `sessionId "ses_forged000000000000000000000"`, `projectId "proj_forged"`, `eventType "forged:event"`, and deletes `timestamp`. After: `id "evt_original"`, sessionId `"ses_123000000000000000000000000"`, `projectId "proj_789"`, `eventType "mcp:tools/call"`, `timestamp` = `Date("2024-01-01")`. | port |
| 24 | applyEventRedaction > should hide internal function fields from the hook but preserve the string-redaction fn | The hook input has no `redactionFn` or `eventRedactionFn`. After the call, `event.redactionFn` is the same function and `event.eventRedactionFn` is undefined, even though the hook returned one. | N/A: the Rust event struct holds no hook closures |
| 25 | applyEventRedaction > should keep the same object identity so pipeline observers see the changes | A second reference to the same event sees `parameters {text:"[SCRUBBED]"}`. | N/A: Rust ownership; the pipeline passes the event by value |
| 26 | applyEventRedaction > should propagate hook errors to the caller | The hook throws `"Hook failure"` → the call rejects with `"Hook failure"`. | adapt: the queue drops the event and logs `Failed to redact event (event-level hook): ...`; catch panics |
| 27 | redactEvent integration tests > should properly redact sensitive data in published events | `track(server,"test-project",{enableTracing:true, redactSensitiveInformation, identify})`. Redactor: contains `@` → `"[REDACTED-EMAIL]"`; lowercase contains `secret` or `key` → `"[REDACTED-SECRET]"`; contains `password` or `ssn` → `"[REDACTED-SENSITIVE]"`; else unchanged. `identify` → `{userId:"test-user-123", userName:"John Doe", userData:{email:"user@example.com", apiKey:"secret-api-key-123"}}`. Call `add_todo {text:"Send email to admin@company.com with password reset", context:"Adding a todo item for reset task"}`. Then: `parameters.request.params.arguments.text` is `"[REDACTED-EMAIL]"` (the `@` rule runs first); `context` is unchanged; no event of type `agentcat:identify`; `sessionId` matches `/^ses_/`; `projectId "test-project"`; `resourceName "add_todo"`; `eventType "mcp:tools/call"`; `identifyActorGivenId "test-user-123"`; `identifyActorName "John Doe"`. | port |
| 28 | redactEvent integration tests > should handle complex nested structures with redaction | Redactor: `/\d{4}[\s-]?\d{4}[\s-]?\d{4}[\s-]?\d{4}/` → `"[REDACTED-CC]"`. Text `"Process payment for card 1234 5678 9012 3456"` → `"[REDACTED-CC]"`. Context `"Processing payment for order"` is unchanged. | port |
| 29 | redactEvent integration tests > should not redact protected fields even if they contain sensitive patterns | Redactor: lowercase contains `id` → `"[REDACTED-ID]"`. `identify` → `{userId:"user-with-id-123", userName:"David Smith", userData:{internalId:"internal-id-456"}}`. Text `"Task with id reference custom-id-789"` and context `"Adding task with id reference"` both become `"[REDACTED-ID]"`. `sessionId` matches `/^ses_/`. `actorId` is undefined. `identifyActorGivenId "user-with-id-123"`. `identifyActorName "David Smith"`. | port |
| 30 | redactEvent hook integration tests > should let the hook inspect event metadata and replace event fields | `redactEvent` records `resourceName`. For `"add_todo"` it returns `{...event, parameters:{replaced:true}}`. Call add_todo `"Buy milk"` (context `"Adding a todo item"`). Then the recorded names contain `"add_todo"`; `parameters` equals `{replaced:true}`; `sessionId` matches `/^ses_/`; `projectId "test-project"`; `eventType "mcp:tools/call"`. | port |
| 31 | redactEvent hook integration tests > should drop only the events the hook rejects, while the others still publish | The hook returns `null` for `add_todo`. Calls: add_todo, then `list_todos {context:"Listing todos"}`. At the send step: 0 events with `resourceName "add_todo"` and 1 with `"list_todos"`. | port |
| 32 | redactEvent hook integration tests > should run the event hook on raw values before string redaction | The hook records `arguments.text`. Redactor: contains `secret` → `"[REDACTED-SECRET]"`. Call add_todo `"this contains a secret value"`. The hook saw `"this contains a secret value"`. The published text is `"[REDACTED-SECRET]"`. | port |
| 33 | redactEvent hook integration tests > should apply the hook to custom events published on a tracked server | When `eventType === "agentcat:custom"`, the hook returns `{...event, response:{scrubbed:true}}`. `publishCustomEvent(server,"test-project",{resourceName:"custom_action", response:{secret:"raw value"}})` → the custom event's `response` equals `{scrubbed:true}`. | port |

Quirks and likely bugs:
1. `PROTECTED_FIELDS` lists `identifyData`, but events store identity data in `identifyActorData`. As a result, the string redactor DOES rewrite actor data. In row 27, `identifyActorData` ships as `{email:"[REDACTED-EMAIL]", apiKey:"[REDACTED-SECRET]"}`; no test asserts this. Rows 11 and 15 test a key the pipeline never fills. This was checked by running the module.
2. The string redactor also rewrites SDK metadata and protocol fields: `sdkLanguage`, `agentcatVersion`, `ipAddress`, `parameters.request.method`, and each content block's `type`. A redactor that rewrites every string therefore makes sanitization turn every text block into `[unsupported content type "<redacted>" redacted - not supported by AgentCat]`. The comment in `token-estimates.test.ts` notes this.
3. Row 14 is titled "gracefully", but a throwing redactor drops every event, and only a log line records it.
4. The comment in row 27 says "Context should not be redacted". Context gets no special treatment; it only escapes because it does not match the patterns. Row 29 shows context being redacted.
5. Row 31 cannot use `EventCapture.flush()`: a dropped event never gets an `id`, so `flush` would wait 5 s and then throw.

### src/tests/report-missing.test.ts (12 tests)
Purpose: Tests the `get_more_tools` (report-missing) tool: its listing, the canned answer, the event it publishes, and session continuity across it.
Harness: Each test builds the v1 todo fixture (`resetTodos()` + `setupTestServerAndClient()` in `beforeEach`, `cleanup()` in `afterEach`). It calls `track(server, ...)` and uses `client.request` for `tools/list` (`ListToolsResultSchema`) and `tools/call` (`CallToolResultSchema`). It waits 50 ms and reads `EventCapture`. Test 11 builds two extra fixtures.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | Report Missing Tool > 1. Tool Injection Tests > should add report_missing to tools list when enableReportMissing is true | With `{enableReportMissing:true, enableTracing:true}`, the listing has `get_more_tools`. Its `description` contains `"Check for additional tools"`. `inputSchema.required` contains `"context"`. `annotations` equals `{title:"Get More Tools", readOnlyHint:true, destructiveHint:false, idempotentHint:true, openWorldHint:false}`. | port |
| 2 | ... > 1. Tool Injection Tests > should NOT add get_more_tools when enableReportMissing is false | With `enableReportMissing:false`, no listed tool is named `get_more_tools`. | port |
| 3 | ... > 1. Tool Injection Tests > should add report_missing WITHOUT context injection even when enableToolCallContext is true | `get_more_tools.inputSchema.required` contains `"context"` (its own parameter). `add_todo.inputSchema.properties.context` equals `{type:"string", description: DEFAULT_CONTEXT_PARAMETER_DESCRIPTION}`. `add_todo.required` contains `"context"`. | port |
| 4 | ... > 2. Tool Execution Tests > should successfully call get_more_tools with only context | Call with `{context:"Need a database query tool for SQL operations"}`. `result.content[1].text` contains `"Unfortunately"` (`content[0]` is the mint-back block). One `mcp:tools/call` event with `resourceName "get_more_tools"`, whose `parameters.request.params.arguments.context` is the same string. | port |
| 5 | ... > 2. Tool Execution Tests > should successfully call get_more_tools with description and optional context | Same as row 4 with `"User wants to fetch data from external REST APIs"`. | port |
| 6 | ... > 2. Tool Execution Tests > should handle missing description parameter gracefully | Call with `{context:"Some context without description"}` → `content[1].text` contains `"Unfortunately"`. | port |
| 7 | ... > 3. Event Tracking Tests > should track report_missing events with proper structure | `track(server,"proj_abc123xyz")` with defaults. Context `"User wants to monitor file changes in real-time"`. The event is defined. `sessionId` is defined and not `""`. `resourceName "get_more_tools"`. `parameters.request.params.name "get_more_tools"`. `arguments` equals exactly `{context}`. `userIntent` is defined. | port |
| 8 | ... > 4. Integration Tests > should maintain task continuity when calling report_missing between other tools | `session_id = sid("report_missing_continuity")` = `"ses_reportmissingcontinuity0000"`. Calls in order: `add_todo {text:"First todo", context:"Adding first todo to test task continuity", session_id}`, `get_more_tools {context:"Need a bulk todo import tool", session_id}`, `list_todos {context:"Listing todos after reporting missing tool", session_id}`. Result: exactly 3 tool events, all with that sessionId, in the order add_todo, get_more_tools, list_todos. | port |
| 9 | ... > 4. Integration Tests > should work correctly with user identification | `identify` → `{userId:"report-user-<uuid>", userData:{name:"Report Test User <uuid>", role:"Developer"}}`. Call `get_more_tools {context:"Need GraphQL query builder"}`. No `agentcat:identify` event. `identifyActorGivenId` equals the userId. `identifyActorData` equals the userData. `Object.keys(getServerTrackingData(server.server)).sort()` equals `["options","projectId"]`. | adapt: the last check reads a TS WeakMap; Rust proves "no identity cache" by behavior (`v2/hooks` row 3) |
| 10 | ... > 6. Analytics Value Tests > should provide actionable insights for missing tool reports | Three calls with contexts `"Want to query PostgreSQL database"`, `"Need to construct complex queries"`, `"Managing schema changes"` → 3 events. Each `arguments.context` matches, in order, and each has `sessionId` and `timestamp`. | port |
| 11 | ... > 6. Analytics Value Tests > should track get_more_tools across different sessions | Two new fixtures, each tracked. Contexts `"Need OAuth integration tool"` and `"Need OAuth2 authentication tool"` → 2 events with 2 distinct sessionIds (each minted). Both contexts contain `"OAuth"`. | port |
| 12 | ... > 6. Analytics Value Tests > should handle multiple missing tool reports in the same task | `sid("report_missing_workflow")` = `"ses_reportmissingworkflow000000"`. Contexts `"Importing data from spreadsheets"`, `"Need to validate imported data"`, `"Process large datasets"`, each with that session_id → 3 events, all on that sid. | port |

Quirks and likely bugs:
1. Row 3: the comment says "Verify context is NOT required", but the test asserts that it IS required. The title says "WITHOUT context injection", which is true only in the sense that the default context description is not injected.
2. Row 7: the comment says the event "should NOT have userIntent", but the test asserts `toBeDefined()`. The intercept sets `userIntent = arguments.context` for `get_more_tools` even when `enableToolCallContext` is off (`src/engine/callWrap.ts:335`). Other tools need that flag.
3. Row 6 is left over from an old schema: no `description` parameter exists any more.
4. The describe numbering skips "5.".

### src/tests/sanitization.test.ts (19 tests)
Purpose: Tests `sanitizeEvent`: how it replaces non-text response blocks and large base64 strings.
Harness: Pure unit tests. Each calls `sanitizeEvent(makeEvent(overrides))`, with `makeEvent`, `makeLargeBase64(n=12000)` and `makeLargeNonBase64(n=12000)` as described in the shared fixtures (item 9). No server, no queue.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | sanitizeEvent - response content blocks > should replace image and audio blocks but keep text blocks | Input `content [ {type:"text",text:"Hello world"}, {type:"image",data:"base64imagedata...",mimeType:"image/png"}, {type:"audio",data:"base64audiodata...",mimeType:"audio/wav"} ]` → length 3. `[0]` is unchanged. `[1]` = `{type:"text", text:"[image content redacted - not supported by AgentCat]"}`. `[2]` = `{type:"text", text:"[audio content redacted - not supported by AgentCat]"}`. | port |
| 2 | ... > should leave text-only content unchanged | `[{type:"text",text:"First"},{type:"text",text:"Second"}]` is unchanged. | port |
| 3 | ... > should redact EmbeddedResource with blob | `{type:"resource", resource:{uri:"file:///data.bin", blob:"base64blobdata...", mimeType:"application/octet-stream"}}` → `{type:"text", text:"[binary resource content redacted - not supported by AgentCat]"}`. | port |
| 4 | ... > should pass through EmbeddedResource with text | `{type:"resource", resource:{uri:"file:///readme.txt", text:"This is a text resource", mimeType:"text/plain"}}` is unchanged (deep equal). | port |
| 5 | ... > should redact unknown content types with type name in message | `{type:"video", data:"somestuff", mimeType:"video/mp4"}` → `{type:"text", text:"[unsupported content type \"video\" redacted - not supported by AgentCat]"}`. | port (the event must hold the response as `Value`; typed rmcp `Content` has no `video`) |
| 6 | ... > should pass through resource_link unchanged | `{type:"resource_link", uri:"file:///some/resource", name:"My Resource"}` is unchanged. | port |
| 7 | ... > should redact large base64 inside structuredContent | `structuredContent {data: makeLargeBase64() (12000 chars), label:"some label"}` → `data` is `"[binary data redacted - not supported by AgentCat]"` and `label` is unchanged. | port |
| 8 | ... > should handle response without content array | `{result:"success"}` is unchanged. | port |
| 9 | ... > should handle null and undefined response without error | `response: null` stays `null`. `response: undefined` stays undefined. | port (`Some(Value::Null)` and `None`) |
| 10 | sanitizeEvent - parameter scanning > should leave small strings unchanged | `{name:"hello", query:"some query text"}` is unchanged. | port |
| 11 | ... > should redact large base64 strings (>10KB) | `imageData` (12000 chars) → `"[binary data redacted - not supported by AgentCat]"`. | port |
| 12 | ... > should leave large non-base64 strings unchanged | `longText` = `makeLargeNonBase64()` (12002 chars with spaces, commas and `!`) is unchanged. | port |
| 13 | ... > should find and redact deeply nested large base64 | `level1.level2.level3.data` → the binary marker. | port |
| 14 | ... > should only redact large base64 in mixed-type parameters | `count 42`, `active true`, `name "short string"` and `tags ["a","b","c"]` are unchanged. `binaryData` → the binary marker. | port |
| 15 | ... > should redact large base64 inside an array | `items ["small string", <base64>, "another small"]` → `[1]` is the marker; the others are unchanged. | port |
| 16 | ... > should redact base64 string at exactly SIZE_GATE boundary (10240 chars) | `atGate` = `"A"×10239 + "="` (10240 chars) → the marker. `belowGate` = `"A"×10238 + "="` (10239 chars) is unchanged. | port |
| 17 | ... > should handle null and undefined parameters without error | `parameters: null` stays `null`. `undefined` stays undefined. | port |
| 18 | sanitizeEvent - integration > should sanitize both parameters and response in a full event | `projectId "proj_1"`, `resourceName "my-tool"`, `parameters {imageData:<base64>, query:"hello"}`, `response.content [ {text "Result text"}, {type:"image", data:"base64img", mimeType:"image/png"} ]` → `imageData` is the marker; `query` stays `"hello"`; `content[0]` is unchanged; `content[1]` is the image marker. `id "evt_1"`, `sessionId "ses_1"`, `projectId "proj_1"` and `resourceName "my-tool"` are unchanged. | port |
| 19 | ... > should not mutate the original event | After the call, the original `imageData` is still the base64 string, and the original `content[0].type` is `"image"` with `data "base64img"`. The result has `imageData` = the marker and `content[0].type` = `"text"`. | port |

Quirks and likely bugs:
1. `BASE64_PATTERN` (`/^[A-Za-z0-9+/\n\r]+=*$/`) needs no padding and has no length-mod-4 check. Any run of 10240 or more letters or digits counts as base64. In `truncation.test.ts` row 49, the plain parameter `"y"×40000` becomes `"[binary data redacted - not supported by AgentCat]"`. Long hex digests and similar values are affected too.
2. A content block without a `type` becomes `[unsupported content type "undefined" redacted - not supported by AgentCat]`.
3. Only `response.content` and `response.structuredContent` are scanned. A 30000-char base64 string in another `response` key, in `error`, or in `userIntent` is kept.
4. `resource.blob: null` counts as binary, because the check is `!== undefined`.

### src/tests/setup.ts (0 tests)
Purpose: The Vitest `setupFiles` entry for the `v1`, `v2` and `e2e-http` projects.
Harness: It sets `process.env.DISABLE_DIAGNOSTICS = "true"` before any test module loads, so no run sends OTLP diagnostics to the live collector. Diagnostics tests opt back in with `"false"` and a mocked `fetch`. The SDK also turns diagnostics off on its own under `VITEST`, `JEST_WORKER_ID` or `NODE_ENV=test`.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| - | (no tests) | - | adapt: Rust tests must turn off SDK diagnostics, for example by an env var or a config switch in a shared test init |

Quirks and likely bugs: none.

### src/tests/signal-shutdown.test.ts (4 tests)
Purpose: Tests `runSignalShutdown`: on SIGINT or SIGTERM it drains the queue and flushes diagnostics, then re-raises the signal so Node's default exit still happens.
Harness: A unit test with the test seam `deps = {destroy: vi.fn(async()=>{}), flush: vi.fn(async()=>{}), listenerCount: vi.fn(()=>0), reRaise: vi.fn()}`, with per-test overrides. No server is involved. (In the real module, `process.once("SIGINT"|"SIGTERM"|"beforeExit")` handlers are installed when `src/modules/eventQueue.ts` is imported.)

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | runSignalShutdown > re-raises the signal after the flush settles so default termination proceeds | `"SIGTERM"` → `destroy` and `flush` are called. `reRaise` is called with `"SIGTERM"`, and its call order is after `destroy`. | adapt: Rust installs no signal handler; the customer calls `shutdown().await`, which drains the queue (5 s budget) and flushes, and the customer owns exit |
| 2 | runSignalShutdown > does not re-raise when the customer has their own listener for the signal | With `listenerCount → 1` and `"SIGINT"`, `reRaise` is not called. | N/A: Node signal-listener semantics |
| 3 | runSignalShutdown > re-raises even when the flush itself fails | `destroy` throws `"drain boom"` → `reRaise("SIGTERM")` is still called. | adapt: `shutdown()` must finish even when draining fails |
| 4 | runSignalShutdown > never throws even when re-raise fails | `reRaise` throws `"kill boom"` → the call resolves to `undefined`. | adapt: `shutdown()` never panics and never returns an error to the caller |

Quirks and likely bugs:
1. Importing the queue module installs process signal handlers as a side effect. `eventQueue.destroy()` (called on `beforeExit` too) permanently replaces `add` with a function that drops events, so any event after `beforeExit` is lost. The Rust design must not copy this.

### src/tests/structured-mintback.test.ts (9 tests)
Purpose: Tests the `mcp_session` mirror in `structuredContent` and its `outputSchema` declaration, on the v1 high-level and low-level server paths.
Harness: `setupTestHooks()` deletes `./agentcat.log` before and after each test. The high-level suite uses the v1 todo fixture with `track(server,"proj_test",{enableAgentTracking:true})` and the typed `client.listTools()`/`client.callTool()`; the v1 client checks `structuredContent` against the listed `outputSchema` with ajv. The low-level suite uses `setupLowLevelStructured(opts)`:
- A v1 `Server({name:"structured test server", version:"1.0"}, {capabilities:{tools:{}}})`.
- Tool `stats`: `outputSchema {type:"object", properties:{count:{type:"number"}}, required:["count"], additionalProperties:false}`.
- Tool `poly`: `outputSchema {type:"object", oneOf:[{type:"object", properties:{a:{type:"string"}}, required:["a"], additionalProperties:false}]}`.
- The call handler returns `{content:[{type:"text",text:"poly"}], structuredContent:{a:"x"}}` for `poly`, and `{content:[{type:"text",text:"42 things"}], structuredContent:{count:42}}` otherwise.
- `track(server,"proj_test",{enableAgentTracking:true, ...opts})`.
- A v1 Client `{name:"test client", version:"1.0"}` over an in-memory pair.

Each test has a new `EventCapture`.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | structured mint-back: high-level (V2) path > structuredContent-only consumer discovers minted handles | listTools, then `get_stats {context:"checking stats"}` → `sc.mcp_session.session_id` matches `/^ses_/`, has no `agent_id`, and `status` is `"issued"`. `sc.count` is `0`. `Object.keys(sc)[0]` is `"mcp_session"`. | port (key order: preserve_order) |
| 2 | ... > tools/list declares mcp_session on get_stats; schema-less tools untouched | `get_stats.outputSchema.properties.mcp_session.type` is `"object"`. `Object.keys(mcp_session.properties)` equals `["session_id","agent_id","status"]`. `add_todo.outputSchema` is undefined. | port |
| 3 | ... > steady state: supplied handles are re-echoed on every response; text block stays mint-only | `session_id sid("fixed")` = `"ses_fixed0000000000000000000000"` and `agent_id "opus-4.80-1m\|claude-code\|k3n9x"` → `mcp_session` = `{session_id: sid, agent_id: same, status:"active"}`. No content block starts with `"[session_id"`. | port |
| 4 | ... > wire-only: published event.response never contains mcp_session | `JSON.stringify(event.response)` contains neither `"mcp_session"` nor `"[session_id"`. `event.response.structuredContent` equals `{count:0}`. | port |
| 5 | ... > content-only tools are unaffected: block leads content, no structuredContent conjured | `add_todo {text:"buy milk", context:"testing"}` → `structuredContent` is undefined. `content[0].type` is `"text"` and its `text` matches `/^\[session_id issued/`. | port |
| 6 | structured mint-back: low-level (V1) path > mints into structuredContent on the V1 path too | `stats {}` → `mcp_session.session_id` matches `/^ses_/`, has no `agent_id`, `status "issued"`, `count 42`, and `mcp_session` is the first key. `event.response` JSON has no `"mcp_session"`. | port |
| 7 | ... > composed outputSchema: schema untouched, no mirror, text block still delivers | The listed `poly.outputSchema` JSON has no `"mcp_session"`. The call gives `structuredContent.mcp_session` undefined, `a` = `"x"`, and `content[0].text` matching `/^\[session_id issued/`. | port |
| 8 | ... > hook mode: mirrored payload carries agent_id but never session_id or status | With `resolveSessionId: () => "corr-1"`, `stats {agent_id:"opus-4.80-1m\|claude-code\|k3n9x"}` → `mcp_session` has no `session_id` and no `status`, and its `agent_id` echoes the value. | port |
| 9 | ... > no tools/list ever (no registry): mirror applies — a list-less client cannot be validating | Calls `_requestHandlers.get("tools/call")({method:"tools/call", params:{name:"stats", arguments:{}}}, {})` directly → `structuredContent.mcp_session.session_id` matches `/^ses_/`. | adapt: call `Tracked::call_tool` with a synthetic context and no earlier `list_tools` |

Quirks and likely bugs:
1. Row 9's title is out of date. `ensureRegistries` (`src/engine/callWrap.ts:86-128`) rebuilds the registry from the stored list handler before the call, so a registry exists and contains `stats`. The "no registry, mirror anyway" branch (`callWrap.ts:303`) is never reached.
2. The describe names say "high-level (V2)" and "low-level (V1)", but both suites use the v1 MCP SDK (`@modelcontextprotocol/sdk`). The labels do not refer to the MCP SDK major version.

### src/tests/test-utils.ts (0 tests)
Purpose: Shared helpers for v1 and v2 tests: `LOG_FILE`, `sid()`, `cleanupLogFile()`, `setupTestHooks()` and `EventCapture`.
Harness: It defines the harness that the other files use. See shared helpers items 1 and 5. `LOG_FILE = "agentcat.log"` is relative to the working directory. `setupTestHooks()` deletes that file in `beforeEach` and `afterEach`.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| - | (no tests) | - | adapt: build a Rust `EventCapture` (a post-pipeline sink with `flush`, `clear`, `find_by_type`, `find_by_resource`) and `sid()` |

Quirks and likely bugs:
1. `setupTestHooks()` deletes `./agentcat.log`, but `writeToLog` writes to `~/agentcat.log` (`src/modules/logging.ts:50-56`). The cleanup does nothing to the real log.
2. `EventCapture` records the pre-pipeline object and depends on in-place mutation. A test that reads pending-hook fields (identity, customer tags, hook-mode sessionId) without `flush()` races with the pipeline.

### src/tests/test-utils/client-server-factory.ts (0 tests)
Purpose: Builds the v1 todo `McpServer`, plus a sampling-capable `Client` connected over `InMemoryTransport`.
Harness: See shared fixture item 3 for the exact tool names, descriptions, schemas, texts and client handler. Tools are registered with both `server.tool(...)` (`add_todo`, `list_todos`) and `server.registerTool(...)` (`complete_todo`, `get_stats`). The server connects with `server.server.connect(serverTransport)`.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| - | (no tests) | - | adapt: write the same four tools with `#[tool_router]` (`get_stats` with a `Json<T>` output for `outputSchema`), and use per-server state |

Quirks and likely bugs:
1. Todo state is shared at module level across every server the factory builds, so tests must call `resetTodos()`.

### src/tests/token-estimates.test.ts (6 tests)
Purpose: Tests `inputTokens` and `outputTokens` on real tool-call events: raw-argument counting, structured fallback, error results, and independence from redaction and truncation.
Harness: Each test builds the v1 todo fixture and starts `EventCapture`. It calls `await track(server,"test-tokens",{enableToolCallContext:false, enableTracing:true, ...})`. It then registers extra tools with `server.tool(name, desc, {text: z.string()}, handler)`, after `track`. It calls `client.request(tools/call)` and `await capture.flush()`. `toolEvent(capture, name)` finds the `mcp:tools/call` event by `resourceName`.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | token estimates on tool-call events > counts the raw arguments and the content text | `token_probe` returns `{content:[{type:"text",text:"hello world"}], structuredContent:{padding:"y"×1000}}`. Arguments `{text:"hi there"}` → `inputTokens 6` (`{"text":"hi there"}` is 19 bytes) and `outputTokens 4` (`"hello world"` is 11 bytes; `structuredContent` is ignored). | port |
| 2 | ... > counts the structured content when the content list is empty | `structured_only` returns `{content:[], structuredContent:{result:"ok"}}` → `outputTokens 5` (`{"result":"ok"}` is 15 bytes). | port |
| 3 | ... > counts an error result's content, including a throw the SDK turns into one | `error_result` returns `{isError:true, content:[{type:"text",text:"boom!"}]}` → `isError true`, `inputTokens 6`, `outputTokens 2` (5 bytes). `throws` throws `Error("no")`; the client receives a result with `isError true`. Its event has `isError true`, `inputTokens 6`, and `outputTokens 1` (the converted text `"no"` is 2 bytes). | adapt: in rmcp, a tool that returns `Err(E: IntoContents)` becomes an `is_error` result and is counted; `Err(McpError)` is a JSON-RPC error with no result, so `outputTokens` is absent |
| 4 | ... > keeps the raw counts when redaction rewrites the payloads | Redactor: contains `secret` → `"[REDACTED]"`. `secret_echo` echoes `args.text`. Arguments `{text:"secret-value-123456789"}` → `response.content[0].text` is `"[REDACTED]"`, `parameters.request.params.arguments.text` is `"[REDACTED]"`, `inputTokens 10` (33 bytes) and `outputTokens 7` (22 bytes). | port |
| 5 | ... > keeps the raw counts when truncation shortens the response | `big` returns text `"x"×40000` → `response.content[0].text.length` is `32771` (32768 + `"..."`) and `outputTokens` is `11429` (= ceil(40000/3.5)). | port |
| 6 | ... > never breaks the tool call when the estimator itself throws | `vi.spyOn` makes `estimateInputTokens` and `estimateOutputTokens` throw `"estimator bug"`. The tool returns `"hello world"`. Then `result.isError` is falsy, `content[0]` matches `{type:"text", text:"hello world"}`, and the input spy was called. | adapt: no module spying in Rust; add a test seam that makes the estimator panic; assert the call result is untouched |

Quirks and likely bugs:
1. Row 6 passes only because the whole tracing setup is abandoned. `estimateInputTokens` is called inside the setup `try` (`src/engine/callWrap.ts:181-270`), so a throw there skips everything: no event, no mint-back, no stripping of injected arguments. That is also why `content[0]` is the tool text and not a mint-back block. If only the output estimator threw, `finish()` would catch it and return the result with no event. Either way the whole event is lost, not "at most this field" as the module comment says.
2. `inputTokens` counts the injected `session_id`, `agent_id` and `context` arguments. These tests turn context off and send no handles, so they never show that.

### src/tests/tokenEstimate.test.ts (39 tests)
Purpose: Golden vectors for the token estimator. These are shared with the Python and Go SDKs.
Harness: Pure unit tests of `TOKEN_ESTIMATE_BYTES_PER_TOKEN`, `estimateTokens`, `utf8ByteLength`, `estimateInputTokens` and `estimateOutputTokens`. Four `it.each` tables; each row is its own test. `text(t) = {type:"text", text:t}`.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | estimateTokens > pins the divisor | `TOKEN_ESTIMATE_BYTES_PER_TOKEN === 3.5`. | port |
| 2 | estimateTokens > bytes 0 -> 0 tokens | `estimateTokens(0) = 0`. | port |
| 3 | estimateTokens > bytes 1 -> 1 tokens | `1 → 1`. | port |
| 4 | estimateTokens > bytes 7 -> 2 tokens | `7 → 2`. | port |
| 5 | estimateTokens > bytes 13 -> 4 tokens | `13 → 4`. | port |
| 6 | estimateTokens > bytes 4096 -> 1171 tokens | `4096 → 1171`. | port |
| 7 | estimateTokens > bytes 7516192765 -> 2147483647 tokens | `7516192765 → 2147483647`. Unclamped, ceil would give 2147483648. | port (take `u64` bytes; clamp to `i32::MAX`) |
| 8 | utf8ByteLength > "" -> 0 bytes | `"" → 0`, and it equals `TextEncoder` length. | port (`str::len`) |
| 9 | utf8ByteLength > "hello" -> 5 bytes | `"hello" → 5`. | port |
| 10 | utf8ByteLength > "café" -> 5 bytes | `"café" → 5`. | port |
| 11 | utf8ByteLength > "你好" -> 6 bytes | `"你好" → 6`. | port |
| 12 | utf8ByteLength > "😀" -> 4 bytes | `"😀" → 4` (a surrogate pair is one 4-byte code point). | port |
| 13 | utf8ByteLength > "\ud83d" -> 3 bytes | A lone high surrogate → 3 (encoded as U+FFFD). | N/A: a Rust `String` cannot hold a lone surrogate |
| 14 | estimateInputTokens (raw arguments) > {"q":"hello"} -> 4 | 13 bytes → 4. | port |
| 15 | estimateInputTokens (raw arguments) > {} -> 1 | `{}` is 2 bytes → 1. | port |
| 16 | estimateInputTokens (raw arguments) > {"name":"café"} -> 5 | 16 bytes (non-ASCII is not escaped) → 5. | port |
| 17 | estimateInputTokens (raw arguments) > {"html":"<a>&</a>"} -> 6 | 19 bytes (no HTML escaping) → 6. | port |
| 18 | estimateInputTokens (raw arguments) > {"t":"你好"} -> 4 | 14 bytes → 4. | port |
| 19 | estimateInputTokens (raw arguments) > {"ids":[1,2,3],"opts":{"deep":true,"n":null}} -> 13 | 45 bytes → 13. | port (integers must print without `.0`) |
| 20 | estimateInputTokens (raw arguments) > omits the field for absent arguments | `undefined → undefined` and `null → undefined`. | port (`None` and `Value::Null` → `None`) |
| 21 | estimateInputTokens (raw arguments) > omits the field when the arguments cannot be serialized | A circular object → `undefined`. | N/A: a `Value` cannot be circular |
| 22 | estimateInputTokens (raw arguments) > never throws: a toJSON that throws yields an omitted field | An object whose `toJSON()` throws `"boom"` → no throw, `undefined`. | N/A: no `toJSON` hook on wire `Value` |
| 23 | estimateOutputTokens (content blocks) > {"content":[{"type":"text","text":"hello world"}]} -> 4 | 11 bytes → 4. | port |
| 24 | ... > {"content":[{"type":"text","text":"abcd"},{"type":"text","text":"e"}]} -> 2 | 4 + 1 = 5 bytes → 2. | port |
| 25 | ... > {"content":[{"type":"text","text":""}]} -> 0 | 0 bytes → 0. | port |
| 26 | ... > {"content":[{"type":"image","data":"QUJD","mimeType":"image/png"}]} -> 0 | An image counts 0. | port |
| 27 | ... > {"content":[{"type":"resource","resource":{"uri":"file:///a","text":"resource body"}}]} -> 4 | `resource.text` is 13 bytes → 4. | port |
| 28 | ... > {"content":[{"type":"resource","resource":{"uri":"file:///a","blob":"QUJD"}}]} -> 0 | A blob resource counts 0. | port |
| 29 | ... > {"content":[{"type":"text","text":"hi"}],"structuredContent":{"big":"yyyy…"}} -> 1 | `structuredContent {big:"y"×1000}` is ignored when content is non-empty; `"hi"` → 1. | port |
| 30 | ... > {"content":[{"type":"text","text":"xxxx…"}]} -> 1171 | `"x"×4096` → 1171. | port |
| 31 | ... > {"content":[{"type":"text","text":"hi"}],"isError":true} -> 1 | `isError` is ignored → 1. | port |
| 32 | ... > {"content":[],"structuredContent":{"result":"ok"}} -> 5 | Empty content falls back to `{"result":"ok"}` (15 bytes) → 5. | port |
| 33 | ... > {"content":[]} -> 0 | → 0. | port |
| 34 | ... > {"content":[],"structuredContent":null} -> 0 | → 0. | port |
| 35 | ... > {"content":[{"type":"image",...}],"structuredContent":{"result":"ok"}} -> 0 | A non-empty, all-image content list → 0; there is no structured fallback. | port |
| 36 | estimateOutputTokens (content blocks) > falls back to the whole response when there is no content list | `{result:"ok"}` → 5. | port |
| 37 | ... > omits the field for an absent response | `undefined` and `null` → `undefined`. | port |
| 38 | ... > ignores blocks whose text is not a string | `content [{type:"text", text:42}, null, "str"]` → 0. | port (on a `Value`-based estimator) |
| 39 | ... > never throws: a content getter that throws yields an omitted field | A `content` getter that throws `"boom"` → no throw, `undefined`. | N/A: no JS getters |

Quirks and likely bugs:
1. Row 35: a result with only image blocks and a `structuredContent` gets 0 output tokens, because the fallback runs only when `content` is empty. This is pinned behavior, not a bug.
2. The `%j` test names print long strings in full; the "…" above shortens them.

### src/tests/tracing-initialization.test.ts (2 tests)
Purpose: Tests that calling `track()` several times does not double-wrap, and that a plain successful call publishes one event.
Harness: v1 todo fixture. `EventCapture` starts in `beforeEach`. Tests call `await track(server, {projectId:"test-project", enableTracing:true})`; note that this passes an OBJECT as `projectId`. They wait 100 ms after each call.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | Tracing Initialization Tests > should not create duplicate events when track() is called multiple times | `track` is called 3 times. Then `add_todo {text:"Test todo for double-wrapping", context:"Setup for double-wrapping test"}`, `clear()`, then `complete_todo {id:"1", context:"Testing double-wrapping protection"}` → `result.isError` is not `true`, there is exactly 1 event, its `resourceName` is `"complete_todo"`, and `isError` is undefined. | adapt: in Rust a double wrap is a type error or a marker pass-through (03 §4.3); test that wrapping twice gives one event and one mint-back |
| 2 | ... > should publish events for successful tool calls with handler-level architecture | `add_todo {text:"Test successful handler wrapping", context:"Testing handler-level event publishing"}` → 1 event with `resourceName "add_todo"`, `isError` undefined, `userIntent "Testing handler-level event publishing"`, and `eventType` and `timestamp` present. | port |

Quirks and likely bugs:
1. Both tests call `track(server, {projectId, enableTracing})`, which does not match the `track(server, projectId, options)` signature. The object becomes `data.projectId`, the options stay at their defaults, and events carry an object as `projectId`. The tests still pass because `EventCapture` stubs out sending. With real sending, the request would carry an object as `projectId`.

### src/tests/truncation.test.ts (49 tests)
Purpose: Tests `normalize` (markers, depth, breadth, cycles) and `truncateEvent` (field limits, frames, content text, the 100 KB budget, no mutation, and the sanitize-then-truncate order).
Harness: Pure unit tests. `normalize(value, depth?, breadth?, maxString?)` and `truncateEvent(makeEvent(overrides))`, with `makeEvent` as in shared fixture item 9 (fixed timestamp `2025-01-15T12:00:00Z`). Sizes are measured as `new TextEncoder().encode(JSON.stringify(result)).length`. The last test runs `sanitizeEvent` and then `truncateEvent`. "Actual" values come from running the module.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | normalize - string truncation > should leave short strings unchanged | `"hello" → "hello"`. | port |
| 2 | ... > should truncate strings exceeding maxStringLength with '...' | `"a"×33000` → length `32771` (32768 + 3), ends with `"..."`, starts with `"a"×100`. | port |
| 3 | ... > should leave strings at exactly maxStringLength unchanged | `"a"×32768` is unchanged. | port |
| 4 | normalize - non-serializable values > should convert functions to descriptive string | `function myFunc(){}` → `"[Function: myFunc]"`. | N/A: JS value type |
| 5 | ... > should convert anonymous functions | `() => {}` → `"[Function: <anonymous>]"`. | N/A: JS value type |
| 6 | ... > should convert symbols | `Symbol("test")` → `"[Symbol(test)]"`. `Symbol()` → `"[Symbol()]"`. | N/A: JS value type |
| 7 | ... > should convert undefined to string marker | `undefined → "[undefined]"`. | N/A: JS value type |
| 8 | ... > should convert BigInt to string marker | `BigInt(123) → "[BigInt: 123]"`. | N/A: JS value type |
| 9 | ... > should convert NaN to string marker | `NaN → "[NaN]"`. | N/A: `serde_json` numbers are finite |
| 10 | ... > should convert Infinity to string marker | `Infinity → "[Infinity]"`. `-Infinity → "[-Infinity]"`. | N/A: `serde_json` numbers are finite |
| 11 | ... > should convert Date to ISO string | `Date("2025-01-15T12:00:00Z") → "2025-01-15T12:00:00.000Z"`. | N/A: no Date in `Value` (but see hazards: the timestamp must serialize in this 24-char form for the size budget) |
| 12 | ... > should handle Invalid Date gracefully | `Date("not-a-date") → "[Invalid Date]"`. | N/A: JS value type |
| 13 | ... > should pass through numbers unchanged | `42`, `0` and `-3.14` are unchanged. | port |
| 14 | ... > should pass through booleans unchanged | `true` and `false` are unchanged. | port |
| 15 | ... > should pass through null as null | `null → null`. | port |
| 16 | normalize - depth limiting > should collapse objects beyond max depth to '[Object]' | `{value:"deep"}` wrapped 15 times in `{nested: ...}`. `normalize(obj, 5)` → `result.nested.nested.nested.nested.nested === "[Object]"`. | port |
| 17 | ... > should collapse arrays beyond max depth to '[Array]' | `["leaf"]` wrapped 15 times in `[ ... ]`. `normalize(arr, 3)` → `result[0][0][0] === "[Array]"`. | port |
| 18 | ... > should handle depth=0 by collapsing top-level objects | `normalize({a:1}, 0) → "[Object]"`. `normalize([1,2], 0) → "[Array]"`. | port |
| 19 | ... > should not collapse primitives regardless of depth | `normalize("hello",0) → "hello"`. `normalize(42,0) → 42`. | port |
| 20 | normalize - breadth limiting > should limit object properties to maxBreadth | 150 keys `key0..key149`. `normalize(wide, 10, 5)` → 6 keys (5 kept + `"..."`), and `result["..."]` is `"[MaxProperties ~]"`. | port (insertion order must keep `key0..key4`) |
| 21 | ... > should limit array elements to maxBreadth | `[0..149]` with `normalize(wide, 10, 5)` → length 6, and `[5]` is `"[MaxProperties ~]"`. | port |
| 22 | ... > should leave objects within breadth limit unchanged | `{a:1,b:2,c:3}` with breadth 100 is unchanged. | port |
| 23 | normalize - circular reference detection > should detect circular object references | `obj.self = obj` → `a` stays `1`, and `self` becomes `"[Circular ~]"`. | N/A: a `Value` cannot be circular |
| 24 | ... > should detect circular array references | `arr = [1,2,arr]` → `[1, 2, "[Circular ~]"]`. | N/A: a `Value` cannot be circular |
| 25 | ... > should allow same object in different branches (not a cycle) | `{a: shared, b: shared}` → both equal `{value:"shared"}`. | N/A: a `Value` has no shared references |
| 26 | ... > should detect deeply nested circular references | `level1.level2.level3.backToRoot = obj` → `"[Circular ~]"`. | N/A: a `Value` cannot be circular |
| 27 | normalize - undefined property handling > should omit undefined object properties (matching JSON.stringify behavior) | `{a:1, b:undefined, c:"hello"}` → `{a:1, c:"hello"}`, and `"b"` is not a key. | N/A: no `undefined` in `Value` |
| 28 | ... > should still convert top-level undefined to marker | `undefined → "[undefined]"`. | N/A: no `undefined` in `Value` |
| 29 | truncateEvent - field-level string limits > should truncate userIntent exceeding 2048 chars | `"x"×3000` → length `2051`, ends with `"..."`. | port |
| 30 | ... > should truncate resourceName exceeding 256 chars | `"t"×300` → `259`, ends with `"..."`. | port |
| 31 | ... > should truncate serverName, serverVersion, clientName, clientVersion exceeding 256 chars | `"s"×300`, `"v"×300`, `"c"×300` and `"cv"×200` (400 chars) → each has length `259`. | port |
| 32 | ... > should leave short field values unchanged | `userIntent "Get weather"`, `resourceName "fetch_weather"` and `serverName "my-server"` are unchanged. | port |
| 33 | ... > should handle undefined/null fields gracefully | Undefined `userIntent`, `resourceName` and `serverName` stay undefined. | port (`None`) |
| 34 | truncateEvent - error field limits > should truncate error.message exceeding 2048 chars | `error {message:"e"×3000, type:"Error"}` → message length `2051`, ends with `"..."`. | port |
| 35 | ... > should limit error.frames to 50 (first 25 + last 25) | 80 frames `{filename:"file<i>.ts", function:"func<i>", lineno:i+1, in_app:true}` → 50 frames. `[0]` is `"file0.ts"`, `[24]` is `"file24.ts"`, `[25]` is `"file55.ts"`, `[49]` is `"file79.ts"`. | port |
| 36 | ... > should leave frames at exactly 50 unchanged | 50 frames → still 50. | port |
| 37 | ... > should handle error without frames | `error {message:"test"}` → message `"test"`, and `frames` is undefined (the output error is exactly `{"message":"test"}`). | port |
| 38 | truncateEvent - response content text truncation > should truncate text content blocks exceeding 32KB | `content [text "x"×40000, text "short"]` → `[0].text.length` is `32771` and ends with `"..."`; `[1].text` is `"short"`. | port |
| 39 | truncateEvent - size targeting > should leave small events unchanged | `parameters {query:"hello"}`, `response {content:[{type:"text",text:"world"}]}` → size < 102400 and `query` is `"hello"`. Actual: unchanged, 189 bytes. | port |
| 40 | ... > should reduce depth progressively for events exceeding 100KB | Input: 8 nested levels, each `{data:"x"×15000, next:{...}}`, with leaf `data:"leaf"` (120283 bytes) → size <= 102400. Actual: depth 6 is the first that fits. `parameters` keeps 6 levels of `data` (15000 chars each, not cut), the 6th `next` becomes `"[Object]"`, and the output is 90240 bytes. | port |
| 41 | ... > should truncate largest string fields as last resort | `parameters {data:"x"×120000}` (120127 bytes) → size <= 102400. Actual: the layer-2 `normalize` already cuts `data` to 32771 chars (output 32898 bytes), so the last-resort step never runs. | port |
| 42 | ... > should guarantee 100KB max for pathological payloads | 50 keys `key0..key49`, each `"v"×3000`, plus `response {data:"r"×30000}` (180680 bytes) → size <= 102400. Actual: the payload is flat, so lower depth does not help, and the last resort runs once with remaining = 78480. `response.data` → 15003 chars (`"r"×15000+"..."`). `key0..key41` → 1503 chars each. `key42` → 2523. `key43..key49` stay at 3000. Output: 102332 bytes. | port |
| 43 | ... > should preserve timestamp as Date when last-resort truncation runs | `parameters {a:"x"×60000, b:"y"×60000, c:"z"×60000, d:"w"×60000}` → `timestamp instanceof Date`, its ISO string is `"2025-01-15T12:00:00.000Z"`, and size <= 102400. Actual: `a` → 16389 chars, `b` → 20130, `c` → 32771, `d` → 32771; output 102206 bytes. | adapt: the Rust timestamp is a typed field, so only the size and string results port |
| 44 | truncateEvent - non-mutation > should not mutate the original event object | Input: `userIntent "x"×3000`, `parameters.deep.nested.data "y"×40000`, and `error {message:"e"×3000, frames: 80}`. After the call, the original's `JSON.stringify` is unchanged. (Actual result: intent 2051, data 32771, message 2051, 50 frames, 40000 bytes.) | port |
| 45 | truncateEvent - edge cases > should handle empty event with minimal fields | `makeEvent({})` → `id "evt_1"` and `sessionId "ses_1"`. | port |
| 46 | ... > should pass through already-small events unchanged | `userIntent "Get weather"`, `resourceName "fetch_weather"`, `parameters.location "SF"` and `response` text `"65F"` are unchanged. | port |
| 47 | ... > should handle event with null parameters and response | `parameters: null`, `response: null` and `error: null` → `parameters` and `response` are still `null`. | port |
| 48 | ... > should preserve SDK-controlled fields exactly | These come back unchanged: `id "evt_abc123"`, `sessionId "ses_xyz789000000000000000000000"`, `projectId "proj_test"`, `eventType "mcp:tools/call"`, `timestamp` (the same `Date` object, by `toBe`), `duration 342`, `sdkLanguage "typescript"`, `agentcatVersion "0.1.12"`, `ipAddress "192.168.1.1"` and `isError false`. | port |
| 49 | truncateEvent - integration with sanitization pipeline > should work correctly after sanitizeEvent in the pipeline | Input: `userIntent "x"×3000`; `parameters {imageData:"A"×12000+"=", query:"hello", nested:{deep:{value:"y"×40000}}}`; `response.content [text "z"×40000, image "base64img"]`. Asserted: `imageData` is `"[binary data redacted - not supported by AgentCat]"`; `content[1]` is the image marker; `userIntent.length` is 2051; `content[0].text.length` <= 32771; `query` is `"hello"`. Actual: `content[0].text` is 32771 chars; `nested.deep.value` ALSO becomes the binary marker; output 35246 bytes. | port |

Quirks and likely bugs:
1. Row 41 does not test what its title says: the 32 KB string cap fires first, so `truncateLargestFields` is never reached.
2. Row 49 hides a sanitizer false positive: `"y"×40000` matches the base64 pattern and is replaced (see sanitization quirk 1).
3. The last-resort pass collects strings longer than 100 chars from the WHOLE event, not only the four payload fields. Long `userIntent`, `identifyActorName` or tag values (up to 200 chars) can also be cut, even though tags are "protected" elsewhere.
4. The 102400 budget is measured on the internal camelCase event (`JSON.stringify(event)`), not on the `PublishEventRequest` that is actually sent. The two sizes differ.
5. `truncateEvent` sets explicit `undefined` keys (`userIntent`, `resourceName`, `serverName`, `serverVersion`, `clientName`, `clientVersion`, `response`). JSON does not show them.
6. The last resort uses `structuredClone`, so `timestamp` becomes a copy of the `Date`. Row 43 checks the type and value, not identity; row 48 checks identity, but on the path with no clone.
7. The breadth check runs before the skip of `undefined` values. An object with exactly N defined keys plus one more key whose value is `undefined` still gets the `"..."` sentinel. A real key named `"..."` within the first N keys is overwritten by the sentinel.
8. All limits count UTF-16 code units, and `slice` can split a surrogate pair (see hazards).

### src/tests/validation.test.ts (11 tests)
Purpose: Tests `validateTags`: the key and value rules, the 50-entry cap, and the log messages.
Harness: `vi.mock("../modules/logging.js", () => ({writeToLog: vi.fn()}))`, with mocks cleared before each test. Pure unit tests call `validateTags(tags)`. The rules (`src/modules/validation.ts`): key regex `/^[a-zA-Z0-9$_.:\- ]+$/`, key length <= 32, value must be a string, value length <= 200, value must not contain `\n`, at most 50 entries. If nothing is valid, the result is `null`. Log formats: `Dropping invalid tag: "<key>" — key contains invalid characters or is empty`, `... — key exceeds max length of 32`, `... — non-string value (got <typeof>)`, `... — value exceeds max length of 200`, `... — value contains newline character`, and `Dropping <n> tag(s) — exceeds maximum of 50 entries per event` (the dash is U+2014).

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | validateTags > should pass through valid tags unchanged | `{env:"production", trace_id:"abc-123", region:"us-east-1"}` is unchanged. | port |
| 2 | validateTags > should return null for empty object | `{}` → `null`. | port |
| 3 | validateTags > should drop keys with invalid characters | `{valid_key:"value", "invalid!key":"value", "good.key":"value"}` → `{valid_key:"value", "good.key":"value"}`. The log contains `"invalid!key"`. | port |
| 4 | validateTags > should drop keys longer than 32 characters | `{["a"×33]:"value", short:"value"}` → `{short:"value"}`. The log contains `"exceeds max length"`. | port |
| 5 | validateTags > should drop values longer than 200 characters | `{key1:"a"×201, key2:"short"}` → `{key2:"short"}`. The log contains `"exceeds max length"`. | port |
| 6 | validateTags > should drop values containing newlines | `{key1:"has\nnewline", key2:"clean"}` → `{key2:"clean"}`. The log contains `"newline"`. | port |
| 7 | validateTags > should drop non-string values at runtime | `{key1:123, key2:"valid"}` → `{key2:"valid"}`. The log contains `"non-string"`. | adapt: applies only if Rust accepts tags as `serde_json::Value`; a typed `String` map cannot hold `123` |
| 8 | validateTags > should drop empty string keys | `{"":"empty-key-value", valid:"value"}` → `{valid:"value"}`. | port |
| 9 | validateTags > should keep only first 50 entries when exceeding limit | 60 tags `key000..key059 → value<i>` → 50 keys. The log contains `"Dropping 10"`. | port (insertion order decides which 50 survive) |
| 10 | validateTags > should return null when all entries are invalid | `{"!!!":"value", "###":"value"}` → `null`. | port |
| 11 | validateTags > should allow keys with periods, colons, dashes, spaces, and dollar signs | `{"my.tag", "my:tag", "my-tag", "my tag", "$ai_trace_id":"trace-1", "my$key"}` are unchanged. | port |

Quirks and likely bugs:
1. Only `\n` is rejected; `\r` passes.
2. Lengths count UTF-16 code units.
3. The cap counts VALID entries and keeps the first 50 in insertion order. A Rust `HashMap` would keep a random 50.

### src/tests/v2/agent-tracking.test.ts (5 tests)
Purpose: Tests `enableAgentTracking`: injection of `agent_id`, tolerance when it is left out, echoing, subagent flow, and the default off state, on the v2 high-level server.
Harness: `setupHighLevel(opts)` builds a v2 `McpServer({name:"v2-agent-tracking", version:"1.0.0"}, {capabilities:{tools:{}}})` with tool `echo` (`inputSchema z.object({msg: z.string()})`, `outputSchema z.object({echoed: z.string()})`), which returns `{content:[{type:"text", text:"echo: <msg>"}], structuredContent:{echoed:<msg>}}`. It calls `track(mcp,"proj_test",opts)`, then `connectClient`, then `listTools` once. Each test has a new `EventCapture`. Events are read without `flush`; the tags read here are set on the request path.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | v2 agent tracking: enableAgentTracking true > advertises both handles as required on every tool including get_more_tools | `echo.inputSchema.properties` has `agent_id` and `session_id`, and `required` contains both. `get_more_tools` has `agent_id` in its properties, and its `required` contains `agent_id` and `session_id`. | port |
| 2 | ... > tolerates omission: task-only mint-back, event without agent identity | `echo {msg:"hi", context:"agent omission"}` → the mint-back block contains `"[session_id issued"` and not `"agent_id"`. `structuredContent.mcp_session.session_id` equals `handleFrom(block,"session_id")`, and the mirror has no `agent_id`. The event tags have neither `agentcat_agent_id` nor `agentcat_agent_id_source`. | port |
| 3 | ... > echoes a self-chosen agent_id verbatim with source tag supplied | `{msg:"hi", context:"agent echo", session_id: sid("fixed"), agent_id:"opus-4.80-1m\|claude-code\|k3n9x"}` → no mint-back. The mirror is `{session_id: sid("fixed"), agent_id: the same value, status:"active"}`. `event.sessionId` is `sid("fixed")`. The tags include `{agentcat_agent_id:"opus-4.80-1m\|claude-code\|k3n9x", agentcat_agent_id_source:"supplied"}`. | port |
| 4 | ... > subagent flow: inherited task + own generated agent_id, no mint-back | `session_id sid("parent")` = `"ses_parent000000000000000000000"` and `agent_id "haiku-4.5\|claude-code\|q7w2e"` → no mint-back. `event.sessionId` is `sid("parent")`. The tags include the agent id with source `"supplied"`. | port |
| 5 | v2 agent tracking: OFF by default > injects no agent_id param, tags nothing, and mints task-only | With default options, no listed tool has `agent_id`. `echo {msg:"hi", context:"defaults"}` → the block contains `"[session_id issued"` and not `"agent_id"`. There are no agent tags. | port |

Quirks and likely bugs: none.

### src/tests/v2/concurrency.test.ts (1 test)
Purpose: Tests that concurrent calls keep their own session, agent and actor attribution.
Harness: A v2 `McpServer "v2-concurrent"` with tool `slow_echo` (`z.object({text: z.string(), delay_ms: z.number()})`), which sleeps `delay_ms` and returns `text`. `track(mcp,"proj_test",{enableAgentTracking:true, identify: req => ({userId: "actor-" + req.params.arguments.text})})`. Then `connectClient`, `listTools`, and three `callTool` calls under `Promise.all`.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | v2 concurrent tool calls do not clobber each other's attribution > each event keeps its own session_id, agent_id, and actor under interleaving | Calls (each with context `"concurrency regression test"`): A = `{text:"A", delay_ms:120, session_id:"ses_sessionA0000000000000000000", agent_id:"agt_A"}`, B = `{text:"B", delay_ms:40, session_id:"ses_sessionB0000000000000000000", agent_id:"agt_B"}`, C = `{text:"C", delay_ms:80, session_id:"ses_sessionC0000000000000000000", agent_id:"agt_C"}`. Result: 3 `mcp:tools/call` events. For each, matched by `arguments.text`: `sessionId` is its own sid; tags include `{agentcat_session_id_source:"supplied", agentcat_agent_id:<its agt_X>}`; `identifyActorGivenId` is `"actor-A"`, `"actor-B"` or `"actor-C"`. | port (`tokio::join!` over a multi-thread runtime) |

Quirks and likely bugs:
1. The test reads `identifyActorGivenId`, which stage 0 fills in the background, without `capture.flush()`. It passes on timing alone and can be flaky.

### src/tests/v2/detect.test.ts (9 tests)
Purpose: Tests detection of the TS MCP SDK major version and server flavor (`detectServer`, `describeSignals`, `fingerprintServerShape`).
Harness: It builds v1 and v2 `Server`/`McpServer` objects, or plain object shapes, and calls the detection functions. No transport is used.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | detectServer > detects a v1 high-level McpServer | Result matches `{major:1, flavor:"high"}`; `lowLevel` is `s.server`; `highLevel` is `s`. | N/A: TS SDK shape detection |
| 2 | detectServer > detects a v1 low-level Server | `{major:1, flavor:"low"}`; `lowLevel` is `s`. | N/A: TS SDK shape detection |
| 3 | detectServer > detects a v2 high-level McpServer (no .tool method) | `{major:2, flavor:"high"}`; `s.tool` is undefined; `lowLevel` is `s.server`. | N/A: TS SDK shape detection |
| 4 | detectServer > detects a v2 low-level Server | `{major:2, flavor:"low"}`. | N/A: TS SDK shape detection |
| 5 | detectServer > returns null for non-server shapes | `null`, `"nope"`, `{}`, `{server:{}}` and `{setRequestHandler(){}, _requestHandlers:{}}` (not a Map) → `null`. | N/A: TS SDK shape detection |
| 6 | detectServer > returns null for a high-level shell passing low-level gates but with neither .tool nor registerTool | `{server:{setRequestHandler(){}, _requestHandlers:new Map(), getClientVersion(){}, _serverInfo:{name:"x"}}}` → `null`. | N/A: TS SDK shape detection |
| 7 | detectServer > describeSignals lists only true signals | `{a:true, b:false, c:true}` → `"a,c"`. | N/A: TS SDK shape detection |
| 8 | fingerprintServerShape > reports the same signals detection computes for a v2 McpServer | The split list contains `"hasServerProp"` and `"hasRegisterTool"`, and not `"hasTool"`. | N/A: TS SDK shape detection |
| 9 | fingerprintServerShape > returns the empty string for null and non-objects | `null`, `undefined`, `"nope"`, `42` → `""`. | N/A: TS SDK shape detection |

Quirks and likely bugs: none.

### src/tests/v2/era-2026.test.ts (2 tests)
Purpose: Tests the 2026-07-28 protocol era through `createMcpHandler`, where each request builds and tracks a new server instance.
Harness: `buildHandler()` resets `factoryRuns` and returns `createMcpHandler(() => { factoryRuns++; ... })`. Each factory run builds a `McpServer "era-2026"` with `echo` (returns `msg`) and returns `track(mcp,"proj_test")`. `connect2026(handler)` makes a `Client({name:"agentcat-2026-client", version:"3.2.1"}, {versionNegotiation:{mode:"auto"}})` over `StreamableHTTPClientTransport(new URL("http://test.local/mcp"), {fetch: (url, init) => handler.fetch(new Request(url, init))})`. Everything runs in process, with no socket.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | 2026-07-28 era via createMcpHandler > tracks per-request instances and captures envelope identity | `echo {msg:"one"}`, then `{msg:"two"}` → `factoryRuns >= 2`; exactly 2 events with `resourceName "echo"`. `[0].clientName` is `"agentcat-2026-client"`, `[0].clientVersion` is `"3.2.1"`, and `[0].tags.agentcat_protocol_version` is `"2026-07-28"`. | adapt: rmcp stateless streamable HTTP with a per-request service factory; client info and protocol version come from the request `_meta` (`context.meta`) |
| 2 | ... > rebuild-on-demand serves calls on instances that never listed | `callTool echo {msg:"solo"}` with no `listTools` → `result.content[1].text` contains `"solo"` (`content[0]` is the mint-back). `event.sessionId` matches `/^ses_/`. | adapt: same Rust topology; the wrapper rebuilds its injection registry by calling the inner `list_tools` (5000 ms timeout) |

Quirks and likely bugs: none.

### src/tests/v2/get-more-tools.test.ts (5 tests)
Purpose: Tests `get_more_tools` on a v2 high-level server: the listing, the answer, the event, the mint-back, continuity and identity.
Harness: `setupHighLevel(opts)` builds a v2 `McpServer "v2-gmt"` with `echo` (returns `"echo: <msg>"`), calls `track(mcp,"proj_test",opts)`, then `connectClient` and `listTools`. Each test has a new `EventCapture`.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | v2 get_more_tools on a tracked high-level server > advertises the tool with required bespoke context, handles, and honest annotations | With `enableAgentTracking`: the `description` contains `"Check for additional tools"`; `required` contains `"context"`, `"session_id"` and `"agent_id"`; the properties include `session_id` and `agent_id`; `annotations` equals `{title:"Get More Tools", readOnlyHint:true, destructiveHint:false, idempotentHint:true, openWorldHint:false}`. | port |
| 2 | ... > answers with the canned text and publishes an event carrying userIntent | `{context:"Need a database query tool for SQL operations"}` → `content[1].text` contains `"Unfortunately"` and `"we have shown you the full tool list"`. The event has `eventType "mcp:tools/call"`, `resourceName "get_more_tools"`, and `userIntent` equal to the context. `arguments.context` is kept. `sessionId` matches `/^ses_/`. | port |
| 3 | ... > applies the mint-back to its result; the published event keeps only the answer | With tracking on: `{context:"need a search tool", agent_id:"opus-4.80-1m\|claude-code\|k3n9x"}` → the block contains `"[session_id issued"` and not `"agent_id"`, and `handleFrom(block,"session_id")` matches `/^ses_/`. `event.sessionId` equals that handle. The tags include the agent id with source `"supplied"`. `event.response` JSON lacks `"[session_id"` and contains `"we have shown you the full tool list"`. | port |
| 4 | ... > keeps task continuity when a supplied session_id spans customer tools and get_more_tools | `sid("gmt_continuity")` = `"ses_gmtcontinuity00000000000000"`. Calls: `echo {msg:"one", context:"before reporting"}`, `get_more_tools {context:"need a bulk import tool"}` (no mint-back), `echo {msg:"two", context:"after reporting"}`, all with that session_id → 3 events, all on the sid, with names `["echo","get_more_tools","echo"]`. | port |
| 5 | ... > stamps identity resolved for a get_more_tools call onto its event | `identify` → `{userId:"report-user", userData:{role:"Developer"}}`. Call `{context:"Need GraphQL query builder"}`, then `flush()` → no `agentcat:identify` event. The event has `resourceName "get_more_tools"`, `identifyActorGivenId "report-user"` and `identifyActorData {role:"Developer"}`. | port |

Quirks and likely bugs: none in addition to report-missing quirk 2.

### src/tests/v2/harness.ts (0 tests)
Purpose: Provides `connectClient`, `mintBackOf` and `handleFrom` for v2 tests.
Harness: It defines the harness. See shared helpers item 4. Both halves of the linked pair come from `@modelcontextprotocol/client`, because each v2 package bundles its own copy of `InMemoryTransport`.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| - | (no tests) | - | adapt: a `tokio::io::duplex` client/server pair with `ClientInfo {name:"agentcat-test-client", version:"1.0.0"}`; `mint_back_of` and `handle_from` helpers |

Quirks and likely bugs: none.

### src/tests/v2/hooks.test.ts (20 tests)
Purpose: Tests the customer hooks `identify`, `eventTags` and `eventProperties`, the `(request, extra)` arguments they receive, and client identity taken from the real connection, on v2.
Harness: `setupHighLevel(opts)` builds a v2 `McpServer({name:"v2-hooks", version:"1.0.0"})` with `add_note` (`z.object({text: z.string()})`), which returns `"Added note: <text>"`. It calls `track(mcp,"test-project",opts)`, then `connectClient` and `listTools`. `toolCallEventsOf(capture)` filters events of type `mcp:tools/call`. `customerTags` and `sdkTags` split the tags (shared helpers item 7). Results put the mint-back at `content[0]`, so tool text is at `content[1]`.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | v2 hooks: identify > runs identify on the tool call and stamps the identity onto that event | `identify(request, extra)` gets both arguments defined and returns `{userId:"user-<uuid>", userData:{name:"Test User <uuid>", email:"test-<uuid>@example.com"}}`. Call `{text:"hello", context:"identification test"}` → `content[1].text` contains `"Added note"`; the hook was called; no `agentcat:identify` event. The event has `resourceName "add_note"`, `identifyActorGivenId` equal to the userId, and `identifyActorData` equal to the userData. The tracking-data keys are `["options","projectId"]`. | adapt: the last check reads a TS WeakMap; the rest ports |
| 2 | ... > runs identify on every tool call and stamps every event, with no dedup | Three calls (`"first"`, `"second"`, `"third"`, context `"no-dedup test"`) → the hook ran 3 times. 3 events, each with `identifyActorGivenId` = the userId and `identifyActorData {name:"Another User"}`. | port |
| 3 | ... > re-resolves identity per request so a changed identity lands on the next event | Identities in order: `{userId:"event-user-a", userName:"First", userData:{seat:"a"}}`, then `{userId:"event-user-b", userName:"Second", userData:{seat:"b"}}`. Calls `"First call"` and `"Second call"` (context `"identity churn test"`), then `flush` → event 0 is a / `"First"` / `{seat:"a"}`; event 1 is b / `"Second"` / `{seat:"b"}`. | port |
| 4 | ... > publishes anonymously when identify returns null | The call succeeds. `identifyActorGivenId` and `identifyActorName` are undefined. `identifyActorData` equals `{}`. | port (anonymous events carry `{}`) |
| 5 | ... > degrades to anonymous when identify throws; the tool call still succeeds | The hook throws `"Failed to identify user"` → the call succeeds, with the same anonymous fields and `{}`. | port (`Err` or panic → anonymous) |
| 6 | ... > handles identify returning invalid data by leaving actor fields empty | The hook returns `{invalidField:"invalid"}` → the call succeeds, with given id and name undefined and data `{}`. | adapt: the Rust identity type is typed; test an identity with no user id |
| 7 | ... > returns the call while async identify is still in flight; identity lands in the background | The hook sleeps 100 ms, then returns `{userId:"async-user", userData:{source:"async-lookup"}}`. The call returns with `"Added note"` while the hook is still running. After `flush`, the event has `identifyActorGivenId "async-user"` and `identifyActorData {source:"async-lookup"}`, and `duration < 95`. | adapt: 03 proposes sync hooks that run before dispatch; if Rust defers hooks, port as is; otherwise assert that `duration` excludes hook time |
| 8 | ... > does not run identify or publish an event for tools/list | `listTools` → the hook is not called, and there are 0 events. | port |
| 9 | v2 hooks: eventTags > attaches customer tags; SDK handle tags ride alongside | With `enableAgentTracking` and `eventTags → {env:"test", trace_id:"abc-123"}`, a call with `agent_id "opus-4.80-1m\|claude-code\|k3n9x"`, then `flush` → `customerTags` equals `{env:"test", trace_id:"abc-123"}`, `agentcat_session_id_source` is `"minted"`, `agentcat_agent_id_source` is `"supplied"`, and `agentcat_agent_id` is the agent id. | port |
| 10 | ... > SDK tag wins when a customer tag collides with an SDK tag name | `eventTags → {agentcat_session_id_source:"customer-value", env:"test"}`. A call with `session_id sid("supplied")` (= `"ses_supplied0000000000000000000"`), then `flush` → the tag is `"supplied"` and `customerTags` is `{env:"test"}`. | port |
| 11 | ... > does not block tool calls when the callback throws; only SDK tags remain | The callback throws `"Tags callback failed"` → the call succeeds, `customerTags` is `{}`, and the SDK tags include `agentcat_session_id_source`. | port |
| 12 | ... > handles null return from the callback | `eventTags → null` → `customerTags` is `{}`, and the session-source tag is present. | port |
| 13 | ... > validates tags from the callback, dropping invalid keys | `{valid:"value", "invalid!":"value"}`, then `flush` → `customerTags` is `{valid:"value"}`. | port |
| 14 | v2 hooks: eventProperties > attaches properties to tool call events | `{device:"desktop", feature_flags:["dark_mode"], nested:{key:"value"}}`, then `flush` → `properties` equals the same object. | port |
| 15 | ... > does not block tool calls when the callback throws | The callback throws `"Properties callback failed"` → `content[1].text` contains `"Added note"`. | port |
| 16 | ... > omits properties when the callback returns null | `properties` is undefined. | port |
| 17 | ... > attaches both tags and properties to the same event | Tags `{env:"test"}` and properties `{device:"mobile"}`, then `flush` → both are present. | port |
| 18 | ... > adds neither tags nor properties when no callbacks are configured | `customerTags` is `{}`, the session-source tag is present, and `properties` is undefined. | port |
| 19 | v2 hooks: (request, extra) arguments carry the v2 ServerContext > passes the raw request and the v2 ServerContext to every hook | For `identify`, `eventTags` and `eventProperties`: `request.params.name` is `"add_note"`, `arguments.text` is `"ctx"`, and `arguments.context` is `"extra shape test"` (the hooks see the raw arguments). `extra`, `extra.mcpReq` and `extra.mcpReq.id` are all defined. | adapt: Rust hooks get a `ToolCall` view (name, raw arguments with injected params, `&RequestContext` with the request id) |
| 20 | v2 real-wire client identity > stamps the connecting client's own identity onto captured events | `clientName` is `"agentcat-test-client"` and `clientVersion` is `"1.0.0"`. | port (`peer_info().client_info`) |

Quirks and likely bugs:
1. Rows 4, 5, 6, 11, 12, 16 and 18 read without `flush`. The anonymous defaults and SDK-only tags are already true before stage 0 runs, so these tests would pass even if the hook result were applied wrongly later.
2. When a customer tag uses an SDK key, it is overwritten with no log line.

### src/tests/v2/http-context-capture.test.ts (3 tests)
Purpose: Tests that the HTTP request (method, url, headers) appears in `event.parameters.extra` after the whole pipeline.
Harness: `setupLowLevel()` builds a v2 low-level `Server "v2-http-context"`. Its list handler returns `echo` (`text` required); its call handler returns `"echo: <text>"`. It calls `track(server,"proj_test",{})`, then `connectClient` and `listTools`, and gets `handler = _requestHandlers.get("tools/call")`. Each test calls `handler(callEcho, extra)` with `callEcho = {method:"tools/call", params:{name:"echo", arguments:{text:"hi", context:"testing"}}}` and a synthetic `ServerContext` as `extra`, then waits 100 ms.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | v2: HTTP request context lands in published event parameters > captures headers, method, and url from the web Request | `extra = {sessionId:"transport-1", mcpReq:{id:7, requestState:()=>undefined, envelope:{}}, http:{req: new Request("http://127.0.0.1:4105/mcp", {method:"POST", headers:{"x-agentcat-health-run":"run:ts_v2"}})}}` → `parameters.extra.http.req.method` is `"POST"`, `.url` is `"http://127.0.0.1:4105/mcp"`, and `.headers["x-agentcat-health-run"]` is `"run:ts_v2"`. The `parameters` JSON contains `"run:ts_v2"`. | adapt: project `http::request::Parts` from `context.extensions` into `{http:{req:{method,url,headers}}}` |
| 2 | ... > leaves stdio-shaped extra (no http) unchanged | `extra = {mcpReq:{id:8, ...}}` → `parameters.extra.mcpReq` is defined and `parameters.extra.http` is undefined. | adapt: over stdio there are no `Parts`, so there is no `http` key; the rest of the Rust `extra` shape is new |
| 3 | ... > does not throw on a malformed req and still publishes the event | `http:{req:{headers:42}}` → the handler result has at least 1 content block, and `parameters.extra.http.req.headers` is `42`. | N/A: typed `http::request::Parts` cannot be malformed |

Quirks and likely bugs:
1. All headers are copied as they are, including `authorization` and `cookie`. Only a customer redactor removes them. Header names come out in lowercase (WHATWG `Headers`).

### src/tests/v2/late-registration.test.ts (4 tests)
Purpose: Tests tools registered AFTER `track()` on a v2 `McpServer`: the listing, argument stripping, error capture and identity.
Harness: Each test builds a v2 `McpServer "v2-late"` and calls `track()` BEFORE `registerTool`, so no tools exist at track time. Then `connectClient` and `listTools`. A module-level variable `lastSeenArgs` records what the handler received.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | v2 late registration: registerTool AFTER track() > advertises a tool registered after track() with injected params | The tool names include `"late_tool"` and `"get_more_tools"`. `late_tool.inputSchema.properties` has `session_id` and `context`. | adapt: the Rust wrapper decorates whatever the inner `list_tools` returns at request time; test with a tool added to a dynamic router |
| 2 | ... > strips injected args that a loose schema passes through (inner tap on the late executor) | `late_loose` (`z.looseObject({message})`) is called with `{message:"hi", context:"late strip test", session_id: sid("late")}` (= `"ses_late00000000000000000000000"`) → the handler args have no `session_id` and no `context`, and `message` is `"hi"`. `event.sessionId` is `sid("late")`. `userIntent` is `"late strip test"`. The raw `arguments.session_id` stays on the event. | port (Rust strips before forwarding; the inner tap does not apply) |
| 3 | ... > captures full error stacks from a late-registered tool via the executor tap | `late_explode` throws `Error("late kaboom with stack")`. With `session_id sid("late_err")` (= `"ses_lateerr00000000000000000000"`), `result.isError` is `true`. The event has `sessionId` = that sid, `isError true`, an `error.message` containing `"late kaboom with stack"`, and a truthy `error.stack`. | adapt: a Rust tool error is a value (`Err`) or a panic, with no JS stack; assert the message, and a backtrace only if one is captured |
| 4 | ... > stamps resolved identity onto events from late-registered tools | `identify` → `{userId:"post-track-user", userData:{name:"Post Track"}}`. `post_track_tool {message:"post-track identification", context:"verifying identify on dynamically added tools"}` → `content[1].text` contains `"post-track identification"`, and the hook was called. The event has `resourceName "post_track_tool"`, `userIntent` = the context, `identifyActorGivenId "post-track-user"`, and `identifyActorData {name:"Post Track"}`. | port |

Quirks and likely bugs:
1. Row 4 reads identity without `flush` and relies on timing.

### src/tests/v2/lowlevel-handles.test.ts (11 tests)
Purpose: Tests explicit handles on a v2 low-level `Server`: canonical injection order, minting, stripping, the error paths, invalid ids, and client-info sources.
Harness: `setupLowLevel(opts)` builds a v2 `Server "v2-low-handles"`. Its list handler returns `echo` (`properties {text: string}`, `required ["text"]`). Its call handler records the arguments in `receivedArgs`, and:
- for `text "explode"`, returns `{isError:true, content:[{type:"text", text:"boom"}]}`;
- for `"throw"`, throws `Error("low-level handler exploded")`;
- otherwise returns `"echo: <text>"`.

It calls `track(server,"proj_test",{enableAgentTracking:true, ...opts})`, then `connectClient`. Rows 10 and 11 call the wrapped `tools/call` handler directly.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | v2 low-level server: explicit handles > injects required handles into listed tool schemas in canonical order | `Object.keys(echo.inputSchema.properties)` equals `["text","session_id","agent_id","context"]`. `required` equals `["text","session_id","agent_id","context"]`. `get_more_tools` has `session_id` and `agent_id`. | port (needs `preserve_order`) |
| 2 | ... > publishes no tools/list or initialize events | After a list and one call, the event types do not contain `"mcp:tools/list"` or `"mcp:initialize"`, and do contain `"mcp:tools/call"`. | port |
| 3 | ... > first call sends start: mints session_id, prepends the mint-back, tags the source | `{text:"hi", context:"testing intent", session_id:"start"}` → the block contains `"[session_id issued"` and not `"agent_id"`. The handle matches `/^ses_/`. `event.sessionId` equals the handle. The tags include `{agentcat_session_id_source:"minted"}`, and there are no agent tags. | port |
| 4 | ... > strips injected params before the customer handler; event keeps them | `{text:"hi", context:"why", session_id: sid("x"), agent_id:"opus-4.80-1m\|claude-code\|k3n9x"}` (`sid("x")` = `"ses_x00000000000000000000000000"`) → the handler received exactly `{text:"hi"}`. The event's raw `arguments.session_id` is `sid("x")`. | port |
| 5 | ... > appends the mint-back on isError results; event.response and event.error stay clean | `text "explode"` → `result.isError` is `true`, and the block contains `"[session_id issued"` and not `"agent_id"`. The event has `isError true`; its `response` JSON lacks `"[session_id"`; its `error.message` contains `"boom"` and not `"[session_id"`. | port |
| 6 | ... > publishes the event and rethrows when the low-level handler throws | `{text:"throw", context:"testing", session_id: sid("boom")}` (= `"ses_boom00000000000000000000000"`) → `callTool` rejects. The event has `sessionId sid("boom")`, `isError true`, an `error.message` containing `"low-level handler exploded"`, and a truthy `error.stack`. | adapt: the handler returns `Err(McpError)`, the client gets a JSON-RPC error, and the event has `isError` and the message; Rust has no JS stack |
| 7 | ... > minting call: block on the wire, absent from event.response | `{text:"hi", context:"testing intent"}` → the block is present. `event.response` JSON lacks `"[session_id"`. `response.content` equals `[{type:"text", text:"echo: hi"}]`. | port |
| 8 | ... > echoed handles: event.response is exactly the handler result | With `sid("fixed")` and an agent id, `event.response` equals `{content:[{type:"text", text:"echo: hi"}]}`. | port (serialize `CallToolResult` without `isError`/`structuredContent` keys when unset) |
| 9 | ... > an invalid session_id publishes sessionless and tells the agent to re-send | `session_id "not-a-real-id"` → the block contains `"[session_id unrecognized"`. `event.sessionId` is `""`. The tags include `{agentcat_session_id_source:"invalid"}`. | port |
| 10 | ... > stamps _meta clientInfo and protocol version when present | A direct call with `params._meta {"io.modelcontextprotocol/clientInfo":{name:"modern-client", version:"2.0"}, "io.modelcontextprotocol/protocolVersion":"2026-07-28"}` and `extra {mcpReq:{id:7, requestState, envelope:{}}}` → `clientName` is `"modern-client"`, and the tags include `{agentcat_protocol_version:"2026-07-28"}`. | port (from `context.meta`) |
| 11 | ... > prefers the v2 envelope for clientInfo and protocol version | No `_meta`. `extra.mcpReq.envelope = {"io.modelcontextprotocol/clientInfo":{name:"envelope-client", version:"3.0"}, "io.modelcontextprotocol/protocolVersion":"2026-07-28"}` → `clientName` is `"envelope-client"`, `clientVersion` is `"3.0"`, and the protocol tag is `"2026-07-28"`. | adapt: rmcp has one `meta` source and no separate envelope; test the order meta → `peer_info()` instead |

Quirks and likely bugs:
1. Row 10 asserts only `clientName`, not `clientVersion "2.0"`.

### src/tests/v2/options.test.ts (10 tests)
Purpose: Tests the track options `enableTracing:false`, `enableReportMissing:false`, `enableToolCallContext:false` and `customContextDescription`, and the edge paths of `track()`.
Harness: `setupHighLevel(opts)` builds a v2 `McpServer "v2-options"` with `echo` (returns `"echo: <msg>"`), calls `track(mcp,"proj_test",opts)`, then `connectClient`. It does not list tools; each test does that itself.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | v2 options: enableTracing false > injects no handle params into the listing | `echo` has no `session_id` and no `agent_id`. | port |
| 2 | ... > passes tool calls through unwrapped-behavior: no event, no mint-back | `echo {msg:"hi"}` → `content[0].text` is `"echo: hi"`, there is no mint-back, and there are 0 events. | port |
| 3 | ... > still answers get_more_tools directly, publishing nothing | `get_more_tools` is listed. A call with `{context:"need a search tool"}` → `content[0].text` contains `"we have shown you the full tool list"`, there is no mint-back, and there are 0 events. | port |
| 4 | v2 options: enableReportMissing false > omits get_more_tools; handle injection is unaffected | There is no `get_more_tools`. `echo` has `session_id` and `context`. | port |
| 5 | v2 options: enableToolCallContext false > injects no context param; events carry no userIntent | `echo` has no `context` but has `session_id`. A call with `{msg:"hi", session_id: sid("ctxoff")}` (= `"ses_ctxoff000000000000000000000"`) → `event.sessionId` is the sid and `userIntent` is undefined. | port |
| 6 | v2 options: customContextDescription > lands the custom text in the advertised context param description | With `"Explain your reasoning for this action"`, `echo.properties.context.description` equals it and is not the default. The `get_more_tools` context description is not the custom text. | port |
| 7 | ... > captures userIntent from calls made with the custom description configured | Custom `"Tell me what you're trying to accomplish"`. A call with context `"I need to add a task to my list"` → the event has `resourceName "echo"` and `userIntent` equal to the context. | port |
| 8 | ... > uses the default description when customContextDescription is not provided | `context.description` equals `DEFAULT_CONTEXT_PARAMETER_DESCRIPTION`. | port |
| 9 | v2 options: track() edge paths > second track() on the same server is a no-op: no double wrap, one event per call | `track(mcp,"proj_test")` again → exactly 1 `get_more_tools`. `echo {msg:"hi", context:"double-track"}` → exactly 1 content block that starts with `"[session_id"`, and exactly 1 event. | adapt: a Rust double wrap is a type error or a marker pass-through |
| 10 | ... > returns an incompatible object untouched instead of throwing | `track({some:"object"}, "proj_test")` returns the same object. | N/A: Rust types prevent tracking a value that is not a server |

Quirks and likely bugs:
1. With `enableTracing:false`, context injection still happens, because `buildInjectedList` checks only `enableToolCallContext`. The listing then advertises a REQUIRED `context` parameter, and `callWrap` forwards the request unstripped. A low-level handler, or a strict schema, receives `context`. Row 1 checks only the handle params.
2. An empty `customContextDescription` (`""`) silently falls back to the default (`||`).

### src/tests/v2/pipeline.test.ts (9 tests)
Purpose: Tests the queue pipeline (redaction and sanitization) on v2-captured events, and `publishCustomEvent` with tracked servers and with session strings.
Harness: The tool tests build a v2 `McpServer "v2-pipeline"`, call `track`, then `connectClient` and `listTools`, then call the tool and wait 100 ms. The custom-event tests use `trackedServer(opts)`: a `McpServer "v2-custom-events"` with a `noop` tool, tracked with `"proj_test"`. They call `agentcat.publishCustomEvent(...)` and read `EventCapture` at once; capture happens at enqueue time.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | v2 pipeline: redactSensitiveInformation on captured events > applies the string-redaction hook to a v2-captured event | `leaky` returns `"result for <query>: secret-sauce"`. The redactor is `text.replace(/secret-sauce/g,"[REDACTED]")`. A call with `{query:"find the secret-sauce", context:"pipeline redaction test", session_id: sid("redact")}` (= `"ses_redact000000000000000000000"`) → the response text contains `"[REDACTED]"` and not `"secret-sauce"` (exactly `"result for find the [REDACTED]: [REDACTED]"`). `arguments.query` is `"find the [REDACTED]"`. `sessionId` is the sid. `resourceName` is `"leaky"`. | port |
| 2 | ... > sanitizes image content blocks in v2 tool responses | `get_attachment` returns `[{type:"text", text:"Here is the attachment:"}, {type:"image", data:"iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJ", mimeType:"image/png"}]`. A call with `{id:"att_1", context:"sanitization test"}` → `content` has length 2; `[0]` is unchanged; `[1]` is `{type:"text", text:"[image content redacted - not supported by AgentCat]"}`. | port |
| 3 | v2 pipeline: publishCustomEvent > queues a custom event on a v2-tracked server with sessionId attribution | `publishCustomEvent(mcp,"proj_test",{sessionId:"ses_custom000000000000000000000", resourceName:"custom-action", parameters:{action:"test"}, message:"Testing custom event"})` → 1 event with `eventType "agentcat:custom"`, that `sessionId`, `projectId "proj_test"`, `resourceName "custom-action"`, `parameters {action:"test"}`, and `userIntent "Testing custom event"` (`message` maps to `userIntent`). | port |
| 4 | ... > validates tags and normalizes empty properties on direct custom events | `tags {valid:"value", "bad!key":"value"}` and `properties {}` → `tags` is `{valid:"value"}` and `properties` is undefined. | port |
| 5 | ... > publishes without a session when a tracked server gets no sessionId | `{resourceName:"sessionless-action"}` → `sessionId` is `""` (null on the wire). | port |
| 6 | ... > uses a session ID string verbatim as the session, carrying full event data | `publishCustomEvent("user-session-12345","proj_test",{resourceName:"full-test", parameters:{key:"value"}, response:{result:"success"}, message:"Complete test", duration:1500, properties:{device:"mobile", feature_flags:["beta"]}, tags:{env:"production"}})` → `eventType "agentcat:custom"`, `sessionId "user-session-12345"` (as given, never derived), and every field carried over: `userIntent "Complete test"`, `duration 1500`, `tags {env:"production"}`, and the properties. | port |
| 7 | ... > lets eventData.sessionId take precedence over the session ID string | `("ignored-string","proj_test",{sessionId:"ses_wins00000000000000000000000"})` → `sessionId "ses_wins00000000000000000000000"`. | port |
| 8 | ... > throws for an untracked v2 server | An untracked `McpServer "v2-untracked"` → rejects with `"Server is not tracked"`, and there are 0 events. | adapt: the Rust API likely takes a `Tracked` handle or a session string, so an untracked server is a type error |
| 9 | ... > validates its required parameters | `("session-id","")` rejects with `"projectId is required"`. `(123,"proj_test")` and `(null,"proj_test")` reject with `"First parameter must be either an MCP server object or a session ID string"`. There are 0 events. | adapt: an empty project id returns `Err`; Rust types enforce the first-argument checks |

Quirks and likely bugs:
1. When `publishCustomEvent` gets a TRACKED server, it ignores its own `projectId` argument: `publishEvent` uses `data.projectId` (`src/modules/eventQueue.ts:390`). The tests use the same id twice, so they cannot see this.
2. A session string is published as given, even when it does not match `ses_` (`"user-session-12345"`). The server may drop such an event later (see audit 02 §2).
3. Events published with a bare session string skip both redaction hooks and carry no `sdkLanguage`, `serverName` or similar fields, because they go to `eventQueue.add` directly.

### src/tests/v2/registration-patch.test.ts (3 tests)
Purpose: Tests the `setRequestHandler` patch that re-arms the wraps when `tools/*` handlers are registered on a v2 `Server`.
Harness: A v2 low-level `Server`. `patchSetRequestHandler(server, cb)` is called, then `server.setRequestHandler(...)`.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | patchSetRequestHandler on v2 > fires the re-arm callback for string tools/call registrations | `setRequestHandler("tools/call", ...)` → the callback fires once, and `_requestHandlers` has `"tools/call"`. | N/A: monkey-patching `setRequestHandler` |
| 2 | ... > forwards 3-arg custom registrations intact (regression: dropped handler) | `setRequestHandler("acme/search", {params: z.object({q})}, handler)` does not throw, and the map has `"acme/search"`. | N/A: monkey-patching `setRequestHandler` |
| 3 | ... > does not fire for non-tool methods | `setRequestHandler("resources/list", ...)` → the callback count stays 0. | N/A: monkey-patching `setRequestHandler` |

Quirks and likely bugs: none.

### src/tests/v2/schema-edges.test.ts (8 tests)
Purpose: Tests handle and `mcp_session` injection on unusual input and output schemas, as a real client sees the listing.
Harness: `listWithTools(tools, opts)` builds a v2 low-level `Server "v2-schema-edges"` whose list handler returns `tools` and whose call handler returns `"ok"`. It calls `track(server,"proj_test",{enableAgentTracking:true, ...opts})`, then `connectClient`, then `listTools`, closes the client, and returns the listed tools.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | v2 schema edges: input injection > skips handle injection entirely on composed (oneOf) input schemas | `inputSchema {type:"object", oneOf:[{type:"object", properties:{a:{type:"string"}}}, {type:"object", properties:{b:{type:"string"}}}]}` → `oneOf` is still there. The schema JSON contains no `"session_id"`, no `"agent_id"` and no `"\"context\""`. | port |
| 2 | ... > builds a default object schema for a tool listed without one | `{name:"bare", description:"no schema"}` → `type` is `"object"`. `properties.session_id` is defined, with `pattern "^(start\|ses_[0-9A-Za-z]{27})$"`. `agent_id` and `context` are defined. `required` contains `session_id` and `agent_id`. | adapt: an rmcp `Tool` always has an `input_schema`; test with an empty `{}` schema |
| 3 | ... > preserves a customer-declared session_id/agent_id instead of overwriting | Customer `properties {session_id:{type:"string", description:"customer session id"}, agent_id:{type:"string", description:"customer agent id"}}` → both descriptions are kept, `session_id` has no `pattern`, and `required` (or `[]`) contains neither key. | port |
| 4 | ... > drops additionalProperties:false so optional handles can ride along | `{type:"object", properties:{text:{type:"string"}}, additionalProperties:false}` → `additionalProperties` is undefined and `session_id` is defined. | port |
| 5 | v2 schema edges: outputSchema injection > declares mcp_session in a listed plain-object outputSchema | `outputSchema {type:"object", properties:{count:{type:"number"}}, required:["count"], additionalProperties:false}` → `properties.mcp_session.type` is `"object"`, and its keys are `["session_id","agent_id","status"]`. | port |
| 6 | ... > omits agent_id from mcp_session when agent tracking is off | With `enableAgentTracking:false`, the `mcp_session` keys are `["session_id","status"]`. | port |
| 7 | ... > leaves composed (anyOf) outputSchemas untouched | `outputSchema {anyOf:[{type:"object", properties:{ok:{type:"boolean"}}}, {type:"object", properties:{err:{type:"string"}}}]}` → the JSON contains `"\"anyOf\""` and no `"mcp_session"`. (The TS v2 codec nests the schema under `result`; that detail does not port.) | port |
| 8 | ... > never overwrites a customer-declared mcp_session output field | `outputSchema.properties.mcp_session {type:"string", description:"customer-owned field"}` is kept. | port |

Quirks and likely bugs:
1. Row 3: a tool that declares its own `session_id` becomes `foreign`. Its calls publish with `sessionId ""` and get no session mint-back. Row 3 checks only the listing.

### src/tests/v2/session-modes.test.ts (8 tests)
Purpose: Tests the session modes: hook mode (`resolveSessionId`: derived id, silent mint on null, no `session_id` param), and prompted mode (supplied, minted on omission, the `start` sentinel).
Harness: `setupHighLevel(opts)` builds a v2 `McpServer "v2-session-modes"` with `echo` (returns `"echo: <msg>"`), calls `track(mcp,"proj_test",opts)`, then `connectClient` and `listTools`. Each test has a new `EventCapture`.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | v2 session modes: resolveSessionId hook > advertises no session_id param; agent_id stays available | `resolveSessionId → "customer-correlation-7"` with tracking on → `echo` has no `session_id` and has `agent_id` (required); `required` (or `[]`) does not contain `session_id`. `get_more_tools` follows the same rule. | port |
| 2 | ... > derives a deterministic ses_ id: same hook value + project → same id across calls and instances | Two calls (context `"hook determinism"`) → both `sessionId`s match `/^ses_/` and are equal, and the tags include `{agentcat_session_id_source:"hook"}`. A new server with the same value gives the same id. `"customer-correlation-8"` gives a different `ses_` id. (Golden: `"customer-correlation-7"` + `"proj_test"` → `ses_2cVoolHakk4pa3LZQSgvJRkTwcK`.) | port (assert the golden ids for cross-SDK stability) |
| 3 | ... > emits no mint-back in hook mode; a supplied agent_id is tagged as supplied | With `agent_id "opus-4.80-1m\|claude-code\|k3n9x"`: no mint-back, and the tags include the agent id with source `"supplied"`. | port |
| 4 | ... > hook mode + tracking on + omitted agent_id: no mint-back at all, no agent tags | No mint-back. `result.structuredContent` is undefined. There are no agent tags. | port |
| 5 | ... > mints silently on hook null: fresh ses_ per call, minted source, no mint-back | `resolveSessionId → null`, two calls → no mint-back on either. After `flush`, 2 events, each `sessionId` matching `/^ses_/` with tag `"minted"`, and the two ids differ. | port |
| 6 | v2 session modes: supplied vs minted (no hook) > tags a supplied session_id as supplied and reuses it verbatim, no mint-back | `session_id sid("fixed")` → no mint-back; `sessionId` is the sid; the tag is `"supplied"`. | port |
| 7 | ... > mints on omission, announces the id, and tags the source as minted | No `session_id` → the block handle matches `/^ses_/`, `event.sessionId` equals it, and the tag is `"minted"`. | port |
| 8 | ... > start begins a new task exactly like omission, case-insensitively | `session_id "start"`, then `" START "` → two block handles, both matching `/^ses_/` and different from each other. The events match them in order, and both have the tag `"minted"`. | port |

Quirks and likely bugs:
1. Row 2 reads `sessionId`, which stage 0 fills in hook mode, without `flush`. Until stage 0 runs, the event has `sessionId ""` with tag `"hook"`, so the test depends on timing.

### src/tests/v2/smoke.test.ts (1 test)
Purpose: A smoke test for the v2 harness itself; AgentCat is not involved.
Harness: A v2 `McpServer "smoke"` with `greet` (`z.object({name})`), which returns `"Hello <name>"`, and `connectClient`. There is no `track()`.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | v2 harness smoke > round-trips a tool call through a real v2 server | `callTool greet {name:"world"}` → `content[0].text` is `"Hello world"`. | adapt: a smoke test of the Rust duplex harness (rmcp client and server) |

Quirks and likely bugs: none.

### src/tests/v2/track-highlevel.test.ts (4 tests)
Purpose: Tests `track()` on a v2 `McpServer`: listing injection, the `outputSchema` mirror, stripping, the mint-back, errors, and re-wrapping after `update()`.
Harness: Each test builds a v2 `McpServer "v2-high"`, registers tools, calls `track(mcp,"proj_test")`, then `connectClient` and usually `listTools`. A module-level `lastSeenArgs` records what the handler received.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | track() on v2 McpServer > injects params + get_more_tools into tools/list and mirrors outputSchema | `echo` (`z.object({msg})`, output `z.object({echoed})`) → the names include `"echo"` and `"get_more_tools"`. `echo` has `session_id` and `context`. `echo.outputSchema.properties.mcp_session` is defined. | port |
| 2 | ... > captures events, strips injected args (loose schema), prepends mint-back, mirrors structuredContent | `echo` with `z.looseObject({msg})`. A call with `{msg:"hi", context:"test intent"}` → the handler args have no `session_id` and no `context`, and `msg` is `"hi"`. `content[0].text` contains `"[session_id issued"`. `structuredContent.mcp_session.session_id` matches `/^ses_/`. There is 1 event with `resourceName "echo"`, `sessionId` matching `/^ses_/`, and `userIntent "test intent"`. | port |
| 3 | ... > echoes a supplied session_id and captures error stacks via the executor tap | `explode` throws `Error("kaboom with stack")`. A call with `session_id sid("fixed123")` (= `"ses_fixed1230000000000000000000"`) → `result.isError` is `true`. The event has `sessionId` = the sid, `isError true`, an `error.message` containing `"kaboom with stack"`, and a truthy `error.stack`. | adapt: Rust tool errors are values or panics; assert the message (no JS stack) |
| 4 | ... > re-wraps the regenerated executor after update() on the next list | `registered.update({callback})`, then `listTools`, then a call with `{msg:"hi", context:"why", session_id: sid("x")}` → the new callback sees `msg "hi"` and no `session_id`. | N/A: `RegisteredTool.update()` regenerating the executor is TS v2 only |

Quirks and likely bugs: none.

### src/tests/v2/track-lowlevel.test.ts (2 tests)
Purpose: Tests `track()` on a v2 low-level `Server`: the wraps on handlers registered by method string, `nextCursor` passing through, and the registry rebuild on demand.
Harness: `buildLowLevel()` builds a v2 `Server "v2-low"`. Its list handler returns `[echo (msg required)]` plus `nextCursor:"page-2"`; its call handler returns `"echo:<msg>"`. It calls `track(server,"proj_test")`, then `connectClient`. Row 2 reads `~/agentcat.log` (`join(homedir(),"agentcat.log")`) from a byte offset taken before the call.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | track() on v2 low-level Server > wraps handlers registered via method strings; preserves nextCursor | `client.request({method:"tools/list", params:{}})` → `nextCursor` is `"page-2"`, the names include `"get_more_tools"`, and `tools[0].inputSchema.properties.session_id` is defined. `callTool echo {msg:"hi", session_id: sid("abc"), context:"why"}` (= `"ses_abc000000000000000000000000"`) → `content[0].text` is `"echo:hi"` (no mint-back for a supplied id), and `event.sessionId` is the sid. | port |
| 2 | ... > rebuilds registries on demand when tools/call arrives before any tools/list | A raw `tools/call` with no list first → `event.sessionId` is `sid("abc")`. The log text appended since the offset contains `"Rebuilt injection registries on demand"` (the full line is `Rebuilt injection registries on demand (tools/call before tools/list on this instance)`). | adapt: assert the rebuild through captured `tracing` output or a test hook, not a file in the home directory |

Quirks and likely bugs:
1. Row 2 depends on a shared file in the user's home directory being writable. Parallel runs can interleave writes to it.

### src/tests/workers/env.d.ts (0 tests)
Purpose: A type declaration for `cloudflare:test`: `Cloudflare.GlobalProps.mainModule` is typed as `./worker`.
Harness: None. It contains types only.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| - | (no tests) | - | N/A: Cloudflare Workers typing |

Quirks and likely bugs: none.

### src/tests/workers/worker.ts (0 tests)
Purpose: The Worker entry point for the Workers suite, with two `createMcpHandler` routes.
Harness:
- `/mcp`: `McpServer "agentcat-workers-test"` with `echo` (`z.object({text})`, returns `text`), tracked with `"proj_workers_test"` and `{apiBaseUrl:"https://ingest.agentcat.test", disableDiagnostics:true}`.
- `/mcp-hook*`: `McpServer "agentcat-workers-hook-test"`, the same `echo`, and the same options plus `resolveSessionId` (sleeps 100 ms, returns `"workers-hook-corr"`) and `identify` (sleeps 100 ms, returns `{userId:"workers-hook-user"}`).

It imports the BUILT package (`import * as agentcat from "agentcat"`, which resolves to `dist/index.workerd.mjs`).

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| - | (no tests) | - | N/A: Cloudflare Workers entry point |

Quirks and likely bugs: none of its own (see the next section).

### src/tests/workers/event-delivery.test.ts (3 tests)
Purpose: Tests that AgentCat ingestion and deferred hooks finish inside `waitUntil` after a Workers response returns, and that headers from a workerd `Request` are captured.
Harness: It runs in the Cloudflare Vitest pool (`vitest.workers.config.ts`, `wrangler.workers-test.jsonc`, compatibility date `2026-08-01`, `nodejs_compat`). `vi.stubGlobal("fetch", fetchMock)` accepts only the origin `https://ingest.agentcat.test`, returns `202`, and records the request bodies; it is restored in `afterEach`. The Client is `{name:"workers-test-client", version:"1.0.0"}` with `versionNegotiation {mode:"auto"}`, over `StreamableHTTPClientTransport("https://worker.test/mcp" or "/mcp-hook")`, with `fetch: (i, init) => SELF.fetch(new Request(i, init))`.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | Cloudflare Workers event delivery > keeps AgentCat ingestion alive after a v2 handler returns | The ingest fetch is held until the test releases it. `callTool echo {text:"hello from Workers"}` → the fetch mock is called once; the result's `content[0].text` is `"hello from Workers"` while ingestion is still held (`ingestionCompleted false`); after the release, ingestion completes. | N/A: Workers `waitUntil` lifecycle (the Rust analog is the dedicated publisher thread plus `shutdown().await`) |
| 2 | ... > resolves deferred hooks inside waitUntil after the response has returned | On `/mcp-hook`, `callTool echo {text:"deferred hooks"}` → `content[0].text` is `"deferred hooks"`, with 0 ingest bodies at that moment. Later, some body contains `"agentcat_session_id_source":"hook"`, `workers-hook-user` and `ses_`. | N/A: Workers runtime (the body check is a useful pattern for a Rust mock-ingest test) |
| 3 | ... > captures HTTP headers from a genuine workerd Request into event parameters | The transport sends the header `x-agentcat-health-run: workers-sentinel` → some ingest body contains `"workers-sentinel"`. | N/A: workerd `Request` realm (`v2/http-context-capture.test.ts` row 1 covers the Rust analog) |

Quirks and likely bugs:
1. Row 1 is probably out of date and would fail against the current code. Since commit `1c4f8b1` (#67, 2026-08-31), a minted session puts the `[session_id issued …]` block at `content[0]`, so `content[0].text` cannot be `"hello from Workers"`. This file was last changed in `0bb144e` (2026-08-04). The suite runs only through `pnpm test:workers`, not `pnpm test`, and no CI workflow in the repo runs it. Row 2 is consistent with the code only because hook mode has no mint-back.

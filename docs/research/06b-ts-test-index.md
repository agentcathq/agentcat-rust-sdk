# TS SDK test index, part B (18 files)

This file indexes every test in 18 test files of the TypeScript SDK. It says what each test asserts and whether the same behavior applies to a Rust `rmcp` server.

Source: `/home/ubuntu/agentcat-typescript-sdk`, commit `9b56e9e`, package `agentcat` `2.1.1-beta.1`. The `-beta.1` suffix is an uncommitted edit to `package.json` in that working tree (dated 2026-10-16); the commit itself says `2.1.1`. Tests that compare against `packageJson.version` read whichever value is on disk. Paths in headings are relative to the repo root. All 18 files run in the vitest project `v1` (`vitest.config.ts`). That project includes `src/tests/**/*.test.ts` without `v2/`, `e2e-http/`, and `workers/`, loads `src/tests/setup.ts`, and uses `pool: "forks"` with `singleFork: true`. As a result, the files run one after another in one process and share module singletons: the event queue, the log file target, and the cached log prefix.

Rust column legend:

- `port`: the same behavior applies to a Rust `rmcp` server. Copy the assertion.
- `adapt: <how>`: the behavior applies, but the Rust shape differs.
- `N/A: <reason>`: the test covers Node or TS-only behavior.

## Summary

Total: **369 test cases**. These are 355 plain `it`/`test` blocks plus 2 `it.each` tables in `handles.test.ts` that expand to 14 cases (5 + 9). Each expanded case has its own row.

Rust column totals: **236 port**, **99 adapt**, **34 N/A**.

| File | Tests | port | adapt | N/A |
| --- | ---: | ---: | ---: | ---: |
| `esm-consumer.test.ts` | 1 | 0 | 0 | 1 |
| `event-tags-properties.test.ts` | 18 | 13 | 5 | 0 |
| `eventQueue.test.ts` | 27 | 16 | 10 | 1 |
| `exceptions.test.ts` | 68 | 9 | 44 | 15 |
| `extra-projection.test.ts` | 8 | 1 | 2 | 5 |
| `handle-conflict-report.test.ts` | 9 | 5 | 3 | 1 |
| `handle-injection.test.ts` | 28 | 24 | 4 | 0 |
| `handler-property.test.ts` | 3 | 0 | 0 | 3 |
| `handles-integration.test.ts` | 28 | 26 | 2 | 0 |
| `handles.test.ts` | 78 | 72 | 6 | 0 |
| `identify.test.ts` | 12 | 10 | 1 | 1 |
| `ksuid.test.ts` | 25 | 17 | 7 | 1 |
| `logging-target.test.ts` | 5 | 0 | 3 | 2 |
| `logging-version-prefix.test.ts` | 4 | 1 | 2 | 1 |
| `mcp-version-compatibility.test.ts` | 1 | 0 | 0 | 1 |
| `posthog-exporter.test.ts` | 27 | 27 | 0 | 0 |
| `posthog-uuidv7.test.ts` | 12 | 12 | 0 | 0 |
| `publishCustomEvent.test.ts` | 15 | 3 | 10 | 2 |
| **Total** | **369** | **236** | **99** | **34** |

### Shared test helpers and fixtures the Rust suite must replicate

1. **Event capture sink** (`EventCapture`, `src/tests/test-utils.ts:36-110`).
   - `start()` replaces the queue singleton's `add` with a function that pushes the event object into a list and then calls the real `add`. Capture is synchronous at enqueue time, so a test can read an event right after `client.callTool` resolves.
   - The real pipeline still runs on the same object: stage 0 (deferred hooks), `redactEvent`, string redaction, sanitization, truncation, then `id = "evt_" + KSUID`. Each stage mutates the captured object in place (`Object.assign`).
   - `sendEvent` is replaced by a no-op. No HTTP request goes out and no telemetry exporter runs.
   - `flush(timeoutMs = 5000)` polls every 5 ms until every captured event has a truthy `id` (the last pipeline step). On timeout it throws `EventCapture.flush timed out waiting for captured events to finish processing`. It waits only for events this capture saw.
   - `getEvents()` returns a copy of the list. `clear()` empties it. `findEventByType(t)` returns the first event with `eventType === t`. `findEventsByResourceName(n)` filters by `resourceName`. `stop()` restores the original `add` and `sendEvent`.
   - Rust: build a test sink that records each event at enqueue time and again after the pipeline, plus a `flush()` that waits for the pipeline. Many TS tests wait with fixed sleeps (`setTimeout` 50 or 100 ms). The Rust suite must use `flush()` instead.

2. **`sid(label)`** (`test-utils.ts:11-17`): returns `"ses_" + (label with every char outside [0-9A-Za-z] removed, then 27 zeros appended, cut to 27 chars)`. Example: `sid("parent")` = `ses_parent000000000000000000000`. The result passes `isValidSessionId`.

3. **Todo server fixture** (`setupTestServerAndClient`, `src/tests/test-utils/client-server-factory.ts`).
   - Server: v1 `McpServer({name: "test server", version: "1.0"})`.
   - Tool `add_todo` (`server.tool`, description `Add a new todo item`, input `text: string` described `The text of the todo item`). It pushes `{id: String(nextId++), text, completed: false}` and returns text `` Added todo: "${text}" with ID ${id} ``.
   - Tool `list_todos` (`server.tool`, description `List all todo items`, no input). It returns the todos as lines `${id}: ${text} ✓` (completed) or `${id}: ${text} ○`, joined with `\n`, or `No todos found` when empty.
   - Tool `complete_todo` (`registerTool`, description `Mark a todo item as completed`, input `id: string` described `The ID of the todo to complete`). It throws `Error("Todo with ID ${id} not found")` for an unknown id, else returns `` Completed todo: "${text}" ``.
   - Tool `get_stats` (`registerTool`, description `Get todo stats`, empty input, `outputSchema {count: number}` described `Number of todos`). It returns content text `${n} todos` and `structuredContent {count: n}`.
   - Client: `Client({name: "test client", version: "1.0"}, {capabilities: {sampling: {}}})` with a `sampling/createMessage` handler that returns `{model: "test-model", role: "assistant", content: {type: "text", text: "This is a test response"}}`.
   - Transport: `InMemoryTransport.createLinkedPair()`. Both sides connect before the function returns, so tests call `track()` after `initialize` has already run.
   - `resetTodos()` empties the module-level list and sets `nextId = 1`. `cleanup()` closes both transports.
   - Rust: an rmcp server with a `#[tool_router]` that has the same four tools, connected to an rmcp client over `tokio::io::duplex`. Keep the exact texts, because tests assert `content[1].text` contains `Added todo` (the mint-back block is `content[0]`).

4. **Low-level echo server fixtures** (`handles-integration.test.ts:19-180`): `setupLowLevel(opts)`, `setupOwnSessionId()`, `setupComposedSchema()`, plus `mintBackOf(result)` (returns `content[0].text` only when it starts with `[session_id`) and `handleFrom(text, name)` (regex `` `${name}: (\S+)` ``). See that section for the exact schemas.

5. **Log capture.** Unit tests replace `writeToLog` with a spy (`vi.mock("../modules/logging")`) and match exact log strings. `logging-version-prefix.test.ts` uses the real diagnostics sink (`setDiagnosticsSink(fn)`), which receives each full log line. Rust needs an injectable log sink so tests can match the same strings.

6. **Ingestion API mock** (`eventQueue.test.ts`). `vi.mock("agentcat-api")` replaces `EventsApi` with an object whose `publishEvent` spy resolves `{}`. Tests read `mockPublishEvent.mock.calls[0][0].publishEventRequest`. That object is the camelCase model before the generated client converts it to snake_case JSON (`sessionId` becomes `session_id`). Rust: assert on the JSON body that a mock HTTP server receives.

7. **Queue control seams** (`eventQueue.test.ts`). Tests overwrite private fields of the singleton: `apiClient`, `maxRetries` (default `3`), `concurrency` (default `5`), `maxQueueSize` (default `10000`), `telemetryManager`. They also install a background-task registrar with `setBackgroundTaskRegistrar(fn)`, which receives one delivery promise per event that has a `projectId`. Rust needs equivalent test-only configuration on the queue.

8. **Fixed KSUID** (`eventQueue.test.ts`, `publishCustomEvent.test.ts`): `KSUID.withPrefix` is mocked to return `{random: async () => "evt_test123"}` (and `randomSync: () => "ses_test12300000000000000000000"` in `publishCustomEvent.test.ts`). Rust: inject an id generator.

9. **HTTP mock for exporters** (`posthog-exporter.test.ts`): `globalThis.fetch` is replaced by `vi.fn()` that resolves `{ok: true, status: 200, text: () => Promise.resolve("ok")}`. Tests parse `fetch.mock.calls[0][1].body` as JSON. Rust: a local mock HTTP server (for example `wiremock`) or an injectable HTTP client.

10. **Environment.** `src/tests/setup.ts` sets `process.env.DISABLE_DIAGNOSTICS = "true"`. `setupTestHooks()` deletes `./agentcat.log` (relative to the working directory) before and after each test. The SDK writes to `~/agentcat.log`, so that cleanup does not touch the real log file.

11. **Fixture builders**, each described in its section: `makeEvent()` (PostHog), `data()` and `ownSessionTool()` (conflict report), `makeTool()`, `customerSchema()`, `structuredTool()`, `objectOutputSchema()` (injection), `customerTags()`/`sdkTags()` (tags), `nodeModules(homedir)` (log target), `PREFIXED_LINE` (log prefix), constants `T`, `A`, `MINTED_BLOCK` (handles).

### Constants that tests compare byte for byte

From `src/modules/constants.ts` (the em dash `—` is part of the strings):

- `SESSION_ID_PARAM_PATTERN` = `^(start|ses_[0-9A-Za-z]{27})$`
- `SESSION_START_SENTINEL` = `start`
- `MINT_BACK_HEADER_ISSUED` = `[session_id issued — see this tool's session_id parameter description]`
- `MINT_BACK_ISSUED_BODY` = `This is the first-call issuance described in this tool's session_id parameter description.`
- `MINT_BACK_HEADER_UNRECOGNIZED` = `[session_id unrecognized — see this tool's session_id parameter description]`
- `MINT_BACK_UNRECOGNIZED_BODY` = `The value sent was not issued by this server. Re-send the session_id issued earlier for this task; if none was issued yet, send start and one will be issued.`
- `mintBackSessionLine(id)` = `session_id: ${id}`
- `MCP_SESSION_KEY` = `mcp_session`
- Tag keys: `agentcat_session_id_source`, `agentcat_agent_id`, `agentcat_agent_id_source`, `agentcat_protocol_version`, `agentcat_mrtr`.
- `_meta` keys: `io.modelcontextprotocol/clientInfo`, `io.modelcontextprotocol/protocolVersion`.
- `AGENTCAT_CUSTOM_EVENT_TYPE` = `agentcat:custom`. `AGENTCAT_SOURCE` = `agentcat`.
- Long descriptions (`SESSION_ID_PARAM_DESCRIPTION`, `AGENT_ID_PARAM_DESCRIPTION`, `MCP_SESSION_FIELD_DESCRIPTION`, `MCP_SESSION_FIELD_DESCRIPTION_HOOK_MODE`, `MCP_SESSION_SESSION_ID_DESCRIPTION`, `MCP_SESSION_AGENT_ID_DESCRIPTION`, `MCP_SESSION_STATUS_DESCRIPTION`): copy them from `constants.ts:20-73`. Tests compare them with `toBe`.
- `get_more_tools` canned answer (`src/modules/tools.ts:42`): `Unfortunately, we have shown you the full tool list. We have noted your feedback and will work to improve the tool list in the future.`

---

### src/tests/esm-consumer.test.ts (1 test)

Purpose: check that a real Node ESM project can import the packed SDK tarball.

Harness: `beforeAll` (timeout 120 s) runs `pnpm pack --pack-destination <os.tmpdir()>` in the repo root. It takes the last stdout line as the tarball path and throws `` pnpm pack did not produce a tarball, got: ${packOutput} `` when that line does not end in `.tgz`. It creates a temp dir `agentcat-esm-consume-*` with `package.json` `{"name":"agentcat-esm-consumer","private":true,"type":"module"}` and runs `pnpm add <tarball>`. `afterAll` deletes the dir and the tarball. No server, no events.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | ESM consumer smoke test > can import the SDK package from a real Node ESM process | Spawns `node --input-type=module -e "import * as m from '<pkgName>'; if (typeof m.track !== 'function') { console.error('track missing'); process.exit(2); } console.log('ok');"` in the temp dir. `pkgName` comes from `package.json` (`agentcat`). Asserts `stderr === ""`, `status === 0`, `stdout.trim() === "ok"`. | N/A: npm ESM/CJS packaging. An optional Rust analog is `cargo package` plus a downstream crate that compiles a call to `track`. |

Quirks: none. This test changes the repo state (it runs `pnpm pack`, which runs `prepack` → `tsup` build).

---

### src/tests/event-tags-properties.test.ts (18 tests)

Purpose: the customer `eventTags` and `eventProperties` hooks on tool-call events and on `publishCustomEvent`, and the absence of `initialize` and `tools/list` events.

Harness: `beforeEach` calls `resetTodos()`, `setupTestServerAndClient()` (connected v1 todo server), and `new EventCapture().start()`. `afterEach` stops the capture and closes the transports. Each test calls `track(server, "test-project", opts)` after connect, sends `client.request({method: "tools/call", params: {name, arguments}}, CallToolResultSchema)`, sleeps 50 ms (100 ms in rows 14 and 16), and reads captured events. Helpers: `customerTags(tags)` removes the four SDK keys `agentcat_session_id_source`, `agentcat_agent_id`, `agentcat_agent_id_source`, `agentcat_protocol_version`. `sdkTags(tags)` keeps only those keys. The tool-call event is found by `eventType === "mcp:tools/call"`.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | Event Tags & Properties > eventTags callback > should attach tags to tool call events | Options `{enableAgentTracking: true, eventTags: async () => ({env: "test", trace_id: "abc-123"})}`. Call `add_todo` with `{text: "Test todo", agent_id: "opus-4.80-1m\|claude-code\|k3n9x"}`. Customer tags equal `{env: "test", trace_id: "abc-123"}`. `tags.agentcat_session_id_source === "minted"`, `tags.agentcat_agent_id_source === "supplied"`, `tags.agentcat_agent_id === "opus-4.80-1m\|claude-code\|k3n9x"`. | port |
| 2 | … > eventTags callback > should overwrite a customer tag that collides with an SDK tag name | `eventTags` returns `{agentcat_session_id_source: "customer-value", env: "test"}`. Call with `{text: "Test todo", session_id: sid("supplied")}`. `tags.agentcat_session_id_source === "supplied"` (SDK tags merge last and win). Customer tags equal `{env: "test"}`. | port |
| 3 | … > eventTags callback > should not block tool calls when callback throws | `eventTags` throws `Error("Tags callback failed")`. `result.content[1].text` contains `Added todo` (`content[0]` is the mint-back block). Customer tags equal `{}`. `sdkTags` has `agentcat_session_id_source`. | port |
| 4 | … > eventTags callback > should handle null return from callback | `eventTags` returns `null`. Customer tags `{}`. SDK tag `agentcat_session_id_source` present. | port |
| 5 | … > eventTags callback > should validate tags from callback | `eventTags` returns `{valid: "value", "invalid!": "value"}`. Customer tags equal `{valid: "value"}` (key regex `^[a-zA-Z0-9$_.:\- ]+$` drops `invalid!`). | port |
| 6 | … > eventProperties callback > should attach properties to tool call events | `eventProperties` returns `{device: "desktop", feature_flags: ["dark_mode"], nested: {key: "value"}}`. `event.properties` equals that object. | port |
| 7 | … > eventProperties callback > should not block tool calls when callback throws | `eventProperties` throws `Error("Properties callback failed")`. `result.content[1].text` contains `Added todo`. No event assertion. | port |
| 8 | … > eventProperties callback > should handle null return from callback | `eventProperties` returns `null`. `event.properties` is `undefined`. | port |
| 9 | … > both callbacks configured > should attach both tags and properties to the same event | Tags `{env: "test"}`, properties `{device: "mobile"}`. Customer tags equal `{env: "test"}`. Properties equal `{device: "mobile"}`. | port |
| 10 | … > publishCustomEvent > should include tags and properties passed directly | `track(server, "test-project", {})`, then `publishCustomEvent(server, "test-project", {resourceName: "custom-action", tags: {env: "production", trace_id: "xyz"}, properties: {device: "mobile", feature_flags: ["beta"]}})`. The event with `resourceName === "custom-action"` has tags exactly `{env: "production", trace_id: "xyz"}` (no SDK tags on custom events) and properties `{device: "mobile", feature_flags: ["beta"]}`. | adapt: same data, through the Rust `publish_custom_event` API. |
| 11 | … > publishCustomEvent > should validate tags passed directly | Tags `{valid: "value", "bad!key": "value"}`. Event tags equal `{valid: "value"}`. | adapt: Rust custom-event API. |
| 12 | … > publishCustomEvent > should normalize empty properties to omission | `properties: {}`. `event.properties` is `undefined`. | adapt: Rust custom-event API; an empty map is omitted. |
| 13 | … > no callbacks configured > should not add tags or properties when not configured | Options `{}`. Customer tags `{}`. SDK tag `agentcat_session_id_source` present. `event.properties` `undefined`. | port |
| 14 | … > initialize is no longer an event > publishes no initialize event even when tag/property callbacks are set | Fresh `McpServer({name: "test server", version: "1.0"})` with tool `test_tool` (description `A test tool`, input `{input: z.string()}`, returns `` Got: ${input} ``). `track` runs BEFORE connect with `eventTags → {env: "test", source: "init"}` and `eventProperties → {device: "desktop"}`. Client `{name: "test client", version: "1.0"}`, `capabilities: {}`. After connect and 100 ms, no event has `eventType === "mcp:initialize"`. | port |
| 15 | … > tools/list is no longer an event > publishes no tools/list event even when tag/property callbacks are set | Tags `{env: "test", action: "list"}`, properties `{source: "list-test"}`. A `tools/list` request publishes no event with `eventType === "mcp:tools/list"`. | port |
| 16 | … > redaction bypass > should not redact tags or properties when redactSensitiveInformation is configured | `redactSensitiveInformation: async () => "[REDACTED]"`. Tags `{env: "production", trace_id: "abc-123"}` and properties `{device: "desktop", app_version: "2.1.0", feature_flags: ["dark_mode", "beta_ui"]}` arrive unchanged. | port |
| 17 | … > callback receives correct arguments > should pass request and extra to eventTags callback | Call `add_todo` with `{text: "Callback args test"}`. The hook's `request.params.name === "add_todo"`, `request.params.arguments.text === "Callback args test"`, and `extra` is defined. | adapt: the Rust hook receives the `CallToolRequestParams` (raw arguments, injected handles included) and the rmcp `RequestContext` or its projection. |
| 18 | … > callback receives correct arguments > should pass request and extra to eventProperties callback | Call with `{text: "Props callback args test"}`. `request.params.name === "add_todo"`, `extra` defined. | adapt: same as row 17. |

Quirks:

- Row 7 asserts nothing about the event. It only checks that the tool call succeeds.
- The tests wait 50 ms instead of `capture.flush()`, although tags and properties land in pipeline stage 0. This can flake on a slow machine.
- `customerTags()` does not remove `agentcat_mrtr`. A multi-round-trip call would make the "exact customer tags" assertions fail.
- Row 10: custom events never get SDK tags and never get string redaction (audit 02 §8 item 7).

---

### src/tests/eventQueue.test.ts (27 tests)

Purpose: `publishEvent` field stamping, the wire mapping, background delivery lifecycle, the queue singleton, and the `redactEvent` hook order.

Harness:

- Mocks: `vi.mock("agentcat-api")` (automock). `EventsApi` returns `{publishEvent: mockPublishEvent}`, where `mockPublishEvent` resolves `{}`. `Configuration` returns `{}`.
- `vi.mock("../modules/logging.js")`: `writeToLog` is a spy.
- `vi.mock("../modules/internal.js")`: `getServerTrackingData` returns `{projectId: "test-project", options: {enableTracing: true}}`.
- `vi.mock("../modules/session.js")`: `buildSessionInfo` returns `{ipAddress: "127.0.0.1", sdkLanguage: "typescript", agentcatVersion: "1.0.0", serverName: "test-server", serverVersion: "1.0.0", clientName: "test-client", clientVersion: "1.0.0", identifyActorGivenId: undefined, identifyActorName: undefined, identifyActorData: {}}`.
- `vi.mock("../thirdparty/ksuid/index.js")`: `KSUID.withPrefix` returns `{random: async () => "evt_test123"}`.
- The module under test is imported after the mocks. Mock server: `{getClientVersion: () => undefined}`.
- Tests assign `(eventQueue as any).apiClient = mockApiClient` because the singleton built its client at import time. They wait with `setTimeout` 10 or 100 ms, or `vi.waitFor`.
- `setupTestHooks()` deletes `./agentcat.log`. `afterEach` calls `vi.resetAllMocks()`.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | EventQueue > publishEvent > should publish event with server tracking data and session info | Event `{sessionId: "test-session", tool: "test-tool", timestamp, arguments: {test: "value"}}`. After 10 ms, `getServerTrackingData` was called with the server and `buildSessionInfo` was called. | port |
| 2 | … > publishEvent > should not publish event when server tracking data is missing | `getServerTrackingData` returns `null`. `writeToLog` called with exactly `Warning: Server tracking data not found. Event will not be published.` | port |
| 3 | … > publishEvent > should not publish event when enableTracing is false | Tracking data `{projectId: "test-project", options: {enableTracing: false}}`. `buildSessionInfo` is not called. | port |
| 4 | … > publishEvent > should calculate duration when not provided | Timestamp is 1 s in the past. Asserts only that `buildSessionInfo` was called. The duration value is not checked. | port |
| 5 | … > publishEvent > should preserve existing duration | `duration: 500`. Asserts only that `buildSessionInfo` was called. | port |
| 6 | … > publishEvent > should publish an absent sessionId as empty string and send it as null | `publishEvent(server, {eventType: "mcp:tools/call", resourceName: "do_thing", timestamp})`. A spy on `eventQueue.add` sees `sessionId === ""`. After 100 ms, `mockPublishEvent` was called once and `publishEventRequest.sessionId` is `null`. | port |
| 7 | … > publishEvent > should send inputTokens and outputTokens on the wire request | `inputTokens: 6`, `outputTokens: 4` → `publishEventRequest.inputTokens === 6`, `.outputTokens === 4`. | port |
| 8 | … > publishEvent > should leave the token fields undefined when the event has none | No token fields → `inputTokens` and `outputTokens` are `undefined` (keys omitted from JSON). | port |
| 9 | … > background delivery lifecycle > keeps the registered task pending until AgentCat ingestion completes | `mockPublishEvent` returns a promise the test resolves by hand. Event `{sessionId: "test-session", eventType: "mcp:tools/call", resourceName: "slow_tool", timestamp}`. The registrar receives exactly 1 task. The task stays unsettled until `finishSend()` runs, then settles. | adapt: Rust has no `waitUntil`. Expose a per-event delivery future or a registrar callback (for example for workers-rs `ctx.wait_until`) and assert it completes only after the HTTP call. |
| 10 | … > background delivery lifecycle > settles fail-open when AgentCat ingestion exhausts retries | `mockPublishEvent` rejects `Error("ingestion unavailable")`, `maxRetries = 0`. The task resolves to `undefined`. `writeToLog` called with a string containing `Failed to deliver AgentCat event after retries`. | adapt: the delivery future resolves `Ok(())` and never yields an error; same log text. |
| 11 | … > background delivery lifecycle > does not register telemetry-only events | `eventQueue.add({sessionId: "telemetry-session", eventType: "mcp:tools/call", timestamp})` (no `projectId`). After `getStats().activeRequests === 0`, the registrar was never called. | adapt: an event without a project id creates no delivery handle. |
| 12 | … > background delivery lifecycle > does not hold AgentCat delivery open for custom exporters | `telemetryManager.export` returns a promise that never settles during the test. The registered task resolves while telemetry is still pending. | adapt: exporter work must not extend the delivery handle. |
| 13 | … > background delivery lifecycle > settles an event when queue overflow drops it | `concurrency = 0`, `maxQueueSize = 1`. Two `add` calls with `projectId: "test-project"` (`dropped-session`, `retained-session`) give 2 registered tasks. The first resolves. `writeToLog` called with exactly `Event queue full, dropping oldest event`. | adapt: drop-oldest is a port; settling the dropped event uses the Rust delivery handle. |
| 14 | … > background delivery lifecycle > continues delivery when runtime task registration throws | The registrar throws `Error("no active request context")`. `publishEvent` does not throw. `mockPublishEvent` is still called. Log contains `Failed to register AgentCat event delivery as a background task`. | adapt: a registrar that fails (or panics, caught) must not stop delivery. |
| 15 | … > EventQueue singleton > should be accessible and have required methods | `eventQueue`, `.add`, `.getStats`, `.destroy` are defined. | adapt: Rust names differ (for example `enqueue`, `stats`, `shutdown`). |
| 16 | … > EventQueue singleton > should track queue stats correctly | `getStats()` has `queueLength` (number), `activeRequests` (number), `isProcessing` (boolean). | adapt: a Rust stats struct with the same three fields. |
| 17 | … > EventQueue singleton > should be able to add events directly | `eventQueue.add(event)` does not throw. | port |
| 18 | … > EventQueue singleton > should handle destroy method without errors | `expect(async () => await eventQueue.destroy()).not.toThrow()`. This cannot fail (an async function never throws synchronously). | adapt: `shutdown(timeout).await` completes without error. |
| 19 | … > EventQueue singleton > should prevent adding events after destroy | After `await eventQueue.destroy()`, `add(event)` logs exactly `Queue is shutting down, event dropped`. | adapt: events after shutdown are dropped with the same log. Decide whether shutdown is permanent (it is in TS). |
| 20 | … > Integration tests > should process events end-to-end through publishEvent | After 100 ms, `getServerTrackingData` was called with the server and `buildSessionInfo` was called. | port |
| 21 | … > Integration tests > logs only event metadata on successful send, never the payload | Restores the prototype `add` (`delete eventQueue.add`). `SECRET = "TOP_SECRET_PAYLOAD_abc123"` sits in `parameters.request.params.arguments.apiKey`, `response.content[0].text`, and `userIntent`. After 100 ms some log line contains `Successfully sent event` and `session test-session`. No log line contains `Event details`. No log line contains SECRET. Line format: `` Successfully sent event ${id} \| ${eventType} \| session ${sessionId} \| ${projectId} \| ${duration} ms \| ${identifyActorGivenId \|\| "anonymous"} ``. | port |
| 22 | … > Integration tests > should handle multiple events | 5 events with `tool: "test-tool-${i}"`. After 100 ms both mocks were called. | port |
| 23 | … > event-level redaction hook (redactEvent) > should apply the redactEvent option from tracking data to published events | Tracking data `options: {enableTracing: true, redactEvent: hook}`, `hook = e => ({...e, parameters: {text: "[EVENT-REDACTED]"}})`. Event `{sessionId: "test-session", eventType: "mcp:tools/call", resourceName: "do_thing", parameters: {text: "raw sensitive value"}}`. Hook called once; its argument has `resourceName === "do_thing"` and `parameters` `{text: "raw sensitive value"}`. Sent `parameters` equal `{text: "[EVENT-REDACTED]"}`. | port |
| 24 | … > event-level redaction hook (redactEvent) > should drop the event when the hook returns null | `redactEvent: () => null`. `mockPublishEvent` not called. Log exactly `Event dropped by redactEvent hook`. | port |
| 25 | … > event-level redaction hook (redactEvent) > should drop the event when the hook throws | Hook throws `Error("hook exploded")`. Not published. Log contains `Failed to redact event (event-level hook)`. | port |
| 26 | … > event-level redaction hook (redactEvent) > should run the event hook before string redaction and compose both | Event hook records `event.userIntent` and returns `` `${userIntent} (reviewed)` ``. `redactionFn` replaces `secret` with `[REDACTED]`. Input `userIntent: "raw secret"`. The event hook saw `["raw secret"]`. Sent `userIntent === "raw [REDACTED] (reviewed)"`. | port |
| 27 | … > Process lifecycle handling > should handle SIGINT signal | Only `expect(process.once).toBeDefined()`. | N/A: Node `process.once` signal API. The Rust suite needs its own shutdown tests. |

Quirks:

- Rows 1, 4, 5, 20, 22 assert only that mocks were called. Row 4 does not check the computed duration. The `duration || fallback` code treats a real `0` ms as missing (audit 02 §8 item 10), and no test covers it.
- Rows 18 and 27 cannot fail.
- `destroy()` replaces `add` on the singleton for good. Later tests must `delete eventQueue.add` to recover. Shutdown is permanent in TS.
- The fixtures use fields `tool` and `arguments` that the `Event` type does not have, and several omit `eventType`, so `publishEvent` sends `eventType: ""`.
- The mocked `buildSessionInfo` returns `sdkLanguage: "typescript"`; the real code sends `"TypeScript"`.
- `maxRetries = 3` means 4 attempts with backoff 1 s, 2 s, 4 s, and every error is retried. Telemetry exporters run again on every retry (audit 02 §8 items 1 and 3).

---

### src/tests/exceptions.test.ts (68 tests)

Purpose: `captureException(error)`, which builds the `error` object of an event: message, type, platform, parsed V8 stack frames, `in_app`, `context_line`, the `Error.cause` chain, non-Error values, and filename normalization.

Harness: direct calls to `captureException` from `src/modules/exceptions.ts`. Most tests build `new Error("Test")` and overwrite `error.stack` with a literal V8 stack, so frame parsing is deterministic. Tests that keep the real stack depend on it in rows 1, 7, 8, 15, and 23 (vitest reports plain absolute paths, not `file://` URLs). No server, no mocks. Root describe: `captureException`; the rows omit it. In rows 35–68, each input is a one-frame stack `Error: Test\n    at <fn> (<path>:<line>:<col>)`; the row lists `<path>:<line>:<col>`.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | basic error capture > should capture Error with message and type | `new Error("Test error message")` → `message "Test error message"`, `type "Error"`, `stack` defined, `frames` defined with length > 0. | adapt: `message` = the error's `Display`; `type` = short type name; `stack`/`frames` only when a backtrace was captured (force capture in the test). |
| 2 | basic error capture > should capture TypeError with correct type | `new TypeError("Type error message")` → `message "Type error message"`, `type "TypeError"`. | adapt: another Rust error type gives its own short name (for example `ParseIntError`). |
| 3 | basic error capture > should capture ReferenceError with correct type | `new ReferenceError("Reference error message")` → `"Reference error message"`, `"ReferenceError"`. | adapt: same check with a third Rust error type. |
| 4 | basic error capture > should capture custom error class | `class CustomError extends Error` with `this.name = "CustomError"` → `message "Custom error message"`, `type "CustomError"`. | adapt: a crate-local `CustomError` type → `CustomError` (last segment of `std::any::type_name`). |
| 5 | basic error capture > should always set platform to 'javascript' | `platform === "javascript"`. | adapt: `"rust"`. The server applies JS grouping rules only to `javascript`, `node`, `js`. |
| 6 | basic error capture > should set platform to 'javascript' for non-Error objects | `captureException("string error").platform === "javascript"`. | adapt: `"rust"` for panics and result errors too. |
| 7 | stack trace parsing > should parse stack frames with function names | `frames[0]` has properties `filename`, `function`, `in_app`; `typeof in_app === "boolean"`. | port |
| 8 | stack trace parsing > should detect in_app correctly for user code | A real throw has at least one frame with `in_app: true`. | adapt: needs `Backtrace::force_capture()` and debug info in the test build; the test crate's frame must be in-app. |
| 9 | stack trace parsing > should detect library code in node_modules | Stack `Error: Test\n    at userFunction (/app/src/test.ts:10:5)\n    at libFunction (/app/node_modules/some-lib/index.js:42:10)\n    at internal (node:internal/process:123:45)` → 3 frames. `[0].in_app true`, filename contains `test.ts`. `[1].in_app false`, filename contains `node_modules`. `[2].in_app false`, filename contains `node:`. | adapt: Rust vector = user `src/…rs` (true), `~/.cargo/registry/src/<index>/<crate>-<ver>/…` (false), `/rustc/<hash>/library/std/…` (false). |
| 10 | stack trace parsing > should parse line and column numbers | `at testFunction (/app/src/file.ts:42:15)` → `lineno 42`, `colno 15`. | adapt: from debug info; omit `colno` when the symbolizer gives none. |
| 11 | stack trace parsing > should handle anonymous functions | `at /app/src/file.ts:10:5` → `function "<anonymous>"`, `lineno 10`, `colno 5`. | adapt: a frame without a symbol name gets a fixed placeholder; closure frames (`{{closure}}`) are normalized, not dropped. |
| 12 | stack trace parsing > should handle async functions | `at async asyncFunction (/app/src/file.ts:20:10)` → `function "async asyncFunction"` (prefix kept). | adapt: async fns show as `{{closure}}` or `{async_fn#0}` frames; normalize to a stable name. |
| 13 | stack trace parsing > should handle native code frames | `at Array.map (native)\n    at userFunction (/app/src/file.ts:10:5)` → `[0].function "Array.map"`, `[0].filename "native"`, `[0].in_app false`. | adapt: frames without file info (libc, unresolved) get a placeholder filename and `in_app: false`. |
| 14 | stack trace parsing > should limit stack frames to 50 | 100 lines `at function${i} (/app/src/file.ts:${i}:5)` → `frames.length <= 50` (actual: exactly the first 50). | port |
| 15 | stack trace parsing > should capture context_line for in_app frames | A real throw: some in-app frame has `context_line !== undefined`. | adapt: read the line from `abs_path` when the file exists (true under `cargo test`, usually false for a deployed binary). |
| 16 | stack trace parsing > should NOT capture context_line for library code (in_app: false) | Stack with only `/app/node_modules/some-lib/index.js:42:10` and `node:internal/process:123:45` frames → every frame `in_app false` and `context_line undefined`. | port |
| 17 | stack trace parsing > should handle missing files gracefully when extracting context_line | `at testFunction (/nonexistent/file/path.ts:10:5)` → 1 frame, `in_app true`, `context_line undefined`, no throw. | port |
| 18 | Error.cause chain > should unwrap single cause | `new Error("Wrapper error", {cause: new Error("Root cause")})` → `message "Wrapper error"`; `chained_errors.length 1`; `[0].message "Root cause"`, `[0].type "Error"`. | adapt: walk `std::error::Error::source()`. |
| 19 | Error.cause chain > should unwrap multiple causes | Top → `Middle cause` → `Root cause` → `chained_errors.length 2`; `[0].message "Middle cause"`, `[1].message "Root cause"` (outer to inner). | adapt: same order from `source()`. |
| 20 | Error.cause chain > should limit cause chain to 10 | `Root` wrapped 20 times (`Level 0` … `Level 19`) → `chained_errors.length <= 10` (actual: 10 entries, `Level 18` … `Level 9`). | port |
| 21 | Error.cause chain > should handle non-Error causes | `new Error("Wrapper", {cause: "string cause"})` → length 1, `[0].message "string cause"`, `[0].type undefined` (key absent in JSON). The chain stops there. | N/A: `source()` always returns `&dyn Error`; Rust has no non-error cause. |
| 22 | Error.cause chain > should detect circular cause references | `error2.cause = error1`, `error1.cause = error2`, capture `error2` → no crash, length `<= 2` (actual: messages `["Error 1", "Error 2"]`). | adapt: keep a visited set of pointers plus the cap of 10. A sound `source()` chain cannot cycle. |
| 23 | Error.cause chain > should capture stack traces for each cause | Top-level `stack` and `frames` defined; `chained_errors[0].stack` and `.frames` defined. | adapt: a Rust cause has a backtrace only if it carries one (for example an inner `anyhow::Error`). Send `stack`/`frames` per cause only then. |
| 24 | non-Error objects > should handle string errors | `captureException("string error")` → `message "string error"`; `type`, `stack`, `frames` undefined. | adapt: a panic payload `&str` or `String` → `message`. A Rust panic also has a location and can capture a backtrace, so `stack`/`frames` can be present. |
| 25 | non-Error objects > should handle number errors | `captureException(42)` → `message "42"`, `type undefined`. | N/A: Rust cannot throw a number. A non-string panic payload (`std::panic::panic_any(42)`) is an opaque `Box<dyn Any>`; send a fixed message. |
| 26 | non-Error objects > should handle boolean errors | `false` → `message "false"`, `type undefined`. | N/A: same as row 25. |
| 27 | non-Error objects > should handle null | `null` → `message "null"`, `type undefined`. | N/A: Rust has no thrown null. |
| 28 | non-Error objects > should handle undefined | `undefined` → `message "undefined"`, `type undefined`. | N/A: Rust has no thrown undefined. |
| 29 | non-Error objects > should handle object errors | `{code: 404, message: "Not found"}` → `message '{"code":404,"message":"Not found"}'` (`JSON.stringify`), `type undefined`. | adapt: the nearest Rust case is `Err(rmcp::ErrorData {code, message, data})`. Send its `message` and keep the code (for example in `type`), not raw JSON. |
| 30 | non-Error objects > should handle objects with circular references | `obj.self = obj` → `type undefined`, `typeof message === "string"` (actual: `"[object Object]"`). | N/A: Rust error values cannot be cyclic. |
| 31 | edge cases > should handle errors without stack traces | `delete error.stack` → `message "No stack"`, `type "Error"`, `stack` and `frames` undefined. | adapt: an error without a captured backtrace (backtraces disabled) → no `stack`, no `frames`. |
| 32 | edge cases > should handle errors with empty messages | `new Error("")` → `message ""`, `type "Error"`. | port |
| 33 | edge cases > should handle errors with only whitespace in stack | `stack = "   \n  \n  "` → `stack` kept as `"   \n  \n  "`, `frames` equal `[]`. | adapt: a backtrace with no usable frames. TS sends `frames: []`; decide whether Rust sends `[]` or omits the key. |
| 34 | edge cases > should handle malformed stack traces gracefully | Lines `at incomplete line without location`, `at /file:10:5`, `at functionWithNoParens /file:20:5` → `frames` defined, no throw. Actual frames: `{function: "incomplete line without location", filename: "<unknown>", in_app: false}`; `{function: "<anonymous>", filename: "file", abs_path: "/file", lineno: 10, colno: 5, in_app: true}`; `{function: "<anonymous>", filename: "functionWithNoParens /file", abs_path: "functionWithNoParens /file", lineno: 20, colno: 5, in_app: true}`. | adapt: unresolved Rust frames (no symbol, no file) must never panic the capture code. |
| 35 | path normalization > user home directory stripping > should strip macOS user home directories | `/Users/john/project/src/index.ts:10:5` → `filename "src/index.ts"`, `abs_path "/Users/john/project/src/index.ts"`. | adapt: same rule on Rust paths (`/Users/<u>/…/src/main.rs` → `src/main.rs`). Rust paths are build-machine paths from debug info. |
| 36 | … > user home directory stripping > should strip Linux user home directories | `/home/ubuntu/app/src/server.ts:20:8` → `"src/server.ts"`, abs `"/home/ubuntu/app/src/server.ts"`. | adapt: same rule on Rust paths. |
| 37 | … > user home directory stripping > should strip Windows user home directories | `C:\Users\Jane\projects\myapp\src\index.ts:15:10` → `"src/index.ts"`, abs `"C:\Users\Jane\projects\myapp\src\index.ts"`. | adapt: same rule on Windows Rust builds. |
| 38 | … > node_modules normalization > should normalize node_modules paths consistently | `/Users/john/project/node_modules/express/lib/router.js:42:10` → `"node_modules/express/lib/router.js"`, abs unchanged, `in_app false`. | adapt: the Rust analog is the Cargo registry: `…/.cargo/registry/src/<index>/<crate>-<ver>/src/x.rs` → for example `<crate>-<ver>/src/x.rs`, `in_app false`. The JS rules would give `src/x.rs` with `in_app true`. |
| 39 | … > node_modules normalization > should handle scoped packages in node_modules | `/app/node_modules/@scope/package/dist/index.js:15:20` → `"node_modules/@scope/package/dist/index.js"`, abs unchanged. | N/A: npm scopes. |
| 40 | … > node_modules normalization > should handle nested node_modules | `/app/node_modules/pkg1/node_modules/pkg2/index.js:10:5` → `"node_modules/pkg2/index.js"` (last occurrence). | N/A: npm nesting has no Cargo equivalent. |
| 41 | … > node_modules normalization > should handle Windows-style node_modules paths | `C:\projects\app\node_modules\lodash\index.js:100:20` → `"node_modules/lodash/index.js"`, abs unchanged. | adapt: Windows Cargo registry `C:\Users\<u>\.cargo\registry\src\…` → forward slashes, crate-relative. |
| 42 | … > deployment path stripping > should strip /var/www/ paths | `/var/www/myapp/src/api/users.ts:25:12` → `"src/api/users.ts"`, abs unchanged. | adapt: keep the prefix list; add Rust build prefixes (for example CI checkout dirs). |
| 43 | … > deployment path stripping > should strip /app/ paths (Docker, Heroku) | `/app/dist/server.js:50:8` → `"dist/server.js"`, abs `"/app/dist/server.js"`. | adapt: `/app/` is a common Rust Docker build dir (`/app/src/main.rs` → `src/main.rs`). |
| 44 | … > deployment path stripping > should strip AWS Lambda paths | `/var/task/src/handler.ts:30:15` → `"src/handler.ts"`, abs `"/var/task/src/handler.ts"`. | adapt: low value; Rust Lambda binaries are built elsewhere. |
| 45 | … > deployment path stripping > should strip Docker container paths | `/usr/src/app/src/index.ts:10:5` → `"src/index.ts"`, abs `"/usr/src/app/src/index.ts"`. | adapt: `/usr/src/app` is a usual Rust Docker work dir; keep it. |
| 46 | … > deployment path stripping > should strip /opt/ paths | `/opt/myservice/lib/main.ts:15:8` → `"lib/main.ts"`, abs `"/opt/myservice/lib/main.ts"`. | adapt: same prefix rule. |
| 47 | … > deployment path stripping > should strip /srv/ paths | `/srv/webapp/src/routes.ts:42:20` → `"src/routes.ts"`, abs `"/srv/webapp/src/routes.ts"`. | adapt: same prefix rule. |
| 48 | … > project boundary detection > should find /src/ boundary | `/some/long/path/to/project/src/components/Button.tsx:20:10` → `"src/components/Button.tsx"`. | adapt: `src/` is the Cargo convention too. Apply it only after the registry and toolchain checks. |
| 49 | … > project boundary detection > should find /lib/ boundary | `/random/path/myproject/lib/utils/helper.ts:35:5` → `"lib/utils/helper.ts"`. | adapt: same marker rule. |
| 50 | … > project boundary detection > should find /dist/ boundary | `/deployment/path/dist/server.js:100:15` → `"dist/server.js"`. | N/A: `dist/` is JS build output; Rust frames point at sources. |
| 51 | … > project boundary detection > should use the last occurrence of project markers | `/Users/john/src/project/src/index.ts:10:5` → `"src/index.ts"`. | adapt: same marker rule. |
| 52 | … > Node.js internal module normalization > should normalize node:internal/* to node:internal | `node:internal/modules/cjs/loader:1105:14` → `"node:internal"`, `in_app false`. | adapt: the Rust analog is toolchain source `/rustc/<commit>/library/<std\|core\|alloc>/src/…` → for example `std/src/…`, `in_app false`. |
| 53 | … > Node.js internal module normalization > should normalize node:fs/promises to node:fs | `node:fs/promises:50:10` → `"node:fs"`, `in_app false`. | N/A: Node built-in module specifiers. |
| 54 | … > Node.js internal module normalization > should preserve simple node: modules | `node:process:400:5` → `"node:process"`, `in_app false`. | N/A: Node built-in module specifiers. |
| 55 | … > Windows path handling > should normalize Windows backslashes to forward slashes | `C:\projects\myapp\src\utils\helper.ts:25:10` → filename contains no `\` and contains `/` (actual `"src/utils/helper.ts"`). | adapt: Windows Rust builds. |
| 56 | … > Windows path handling > should handle Windows paths with mixed separators | `C:\projects/myapp\src/index.ts:10:5` → `"src/index.ts"`, no `\`. | adapt: Windows Rust builds. |
| 57 | … > complex real-world scenarios > should handle mix of user code and library code | `/Users/dev/project/src/api/handler.ts:50:10` → `"src/api/handler.ts"`, `in_app true`; `/Users/dev/project/node_modules/express/lib/router.js:100:5` → `"node_modules/express/lib/router.js"`, `false`; `node:internal/modules/cjs/loader:1105:14` → `"node:internal"`, `false`. | adapt: Rust vector with one user frame, one registry frame, one toolchain frame. |
| 58 | … > complex real-world scenarios > should handle Docker deployment with node_modules | `/app/dist/api/users.ts:30:8` → `"dist/api/users.ts"`; `/app/node_modules/validator/lib/index.js:42:15` → `"node_modules/validator/lib/index.js"`. | adapt: Rust vector with `/app/src/…` and a registry frame. |
| 59 | … > complex real-world scenarios > should handle AWS Lambda with layers | `/var/task/src/lambda/handler.ts:20:5` → `"src/lambda/handler.ts"`; `/opt/nodejs/node_modules/aws-sdk/lib/service.js:150:10` → `"node_modules/aws-sdk/lib/service.js"`. | N/A: Node Lambda layers. |
| 60 | … > URL scheme normalization > should handle file:// URLs from ESM modules | `file:///Users/john/project/src/index.ts:10:5` → `"src/index.ts"`, abs `"file:///Users/john/project/src/index.ts"`. | N/A: ESM `file://` URLs; Rust debug info has plain paths. |
| 61 | … > URL scheme normalization > should handle file:// URLs with Windows paths | `file:///C:/Users/Jane/project/src/index.ts:15:8` → `"src/index.ts"`, abs `"file:///C:/Users/Jane/project/src/index.ts"`. | N/A: ESM `file://` URLs. |
| 62 | … > URL scheme normalization > should handle file:// URLs in node_modules | `file:///Users/john/project/node_modules/express/lib/router.js:50:5` → `"node_modules/express/lib/router.js"`, `in_app false`. | N/A: ESM `file://` URLs. |
| 63 | … > special cases and edge cases > should preserve already-relative paths | `src/index.ts:10:5` → `filename` and `abs_path` both `"src/index.ts"`. | adapt: Cargo records workspace paths as relative (`src/main.rs`, shown as `./src/main.rs`). Keep them relative and strip `./`. |
| 64 | … > special cases and edge cases > should preserve special paths like native | `at Array.map (native)` → `filename "native"`, `abs_path "native"`. | adapt: a fixed placeholder for frames without a file. |
| 65 | … > special cases and edge cases > should handle paths without common markers | `/random/path/without/markers/file.ts:10:5` → `"random/path/without/markers/file.ts"` (leading `/` removed). | adapt: same rule on Rust paths. |
| 66 | … > special cases and edge cases > should handle very short paths | `/index.ts:1:1` → `"index.ts"`. | port |
| 67 | … > backwards compatibility > should maintain abs_path field with original path | `/Users/john/long/path/to/project/src/index.ts:10:5` → `filename "src/index.ts"`, `abs_path` unchanged. | port |
| 68 | … > backwards compatibility > should maintain all other frame properties | `at myFunction (/app/src/test.ts:42:15)` → `function "myFunction"`, `filename` and `abs_path` defined, `lineno 42`, `colno 15`, `in_app` a boolean. | port |

#### Error capture: what the TS SDK puts in `error`, and what Rust must send

This subsection reads `src/modules/exceptions.ts`, `src/types.ts:255-279`, `src/engine/callWrap.ts:305-355`, and `src/engine/innerTap.ts:54-63`. Probe results come from running a copy of `exceptions.ts` in a scratch directory (Node 22).

**Wire shape.** The event key is `error`. JS drops `undefined` values when it serializes, so an absent field means "no key".

```text
ErrorData (key order as built):
  message: string            required. Error: error.message || "". Non-Error: stringified value.
  type?: string              Error: error.name || error.constructor.name. Absent for non-Error and isError results.
  platform?: string          always "javascript"
  stack?: string             raw Error.stack, verbatim (first line "Name: message", then "    at …" lines)
  frames?: StackFrame[]      parsed from stack, max 50, innermost first (V8 order)
  chained_errors?: ChainedErrorData[]   only when at least one cause exists; max 10; outer to inner

StackFrame (key order as built):
  function: string           "<anonymous>" for a bare location; the whole line text for an unparseable line
  filename: string           normalized relative path (see below); "native", "<unknown>"
  abs_path?: string          original location string, unchanged (absent for unparseable lines)
  lineno?: number
  colno?: number
  in_app: boolean
  context_line?: string      only for in-app frames whose source file can be read

ChainedErrorData (key order as built):
  message: string            cause.message || ""; non-Error cause: stringified value
  type?: string              cause.name || constructor name || "Error"; absent for a non-Error cause
  stack?: string
  frames?: StackFrame[]      max 50 per cause
  (no platform key)
```

**Three input paths in `callWrap`:**

1. The handler throws an `Error` and the MCP SDK turns it into an `isError` result. The inner tap (`innerTap.ts:54-63`) stores the original `Error` on `extra.__agentcat_error`. `finish()` captures that stored error, so the event keeps the real `type`, `stack`, and `frames`, and also records `response` (the error result) and `outputTokens`.
2. The handler returns `{isError: true, content: [...]}` itself (or the stash is missing). `captureException(result)` sees an object with `isError` and an array `content`. `message` = the `text` of every `{type: "text"}` block, joined with one space, then trimmed (inner spaces stay: `" boom "` + `"x"` → `"boom  x"`), or `"Unknown error"` when that is empty. Only `message` and `platform` are set.
3. The original handler rejects (the SDK did not convert the throw). The wrapper sets `isError`, `error = captureException(thrown)`, and `duration`, publishes, then rethrows. The event has no `response` and no `outputTokens`.

In all three paths `isError` is `true`. On success `isError` and `error` are absent, not `false`.

**Non-Error values** (`stringifyNonError`): `null` → `"null"`, `undefined` → `"undefined"`, a string → itself, a number or boolean → `String(v)`, anything else → `JSON.stringify(v)` with `String(v)` as the fallback (a cyclic object gives `"[object Object]"`). A thrown `Symbol` or function makes `JSON.stringify` return `undefined`, so `message` is missing (probe). The server then rejects the whole error, because `message` is required.

**Frame parsing** (`parseV8StackTrace`): only lines that start with `at ` after trimming are frames. Formats:

- `at fn (location)` → `function` = `fn` trimmed (keeps `async ` and `new ` prefixes).
- `at location` → `function "<anonymous>"`.
- Anything else → `{function: <text after "at ">, filename: "<unknown>", in_app: false}`.
- Location `native` → filename and abs_path `"native"`. `unknown location` → `"<unknown>"`. `eval at f (…), <anonymous>:l:c` → the innermost real file location (probe: `eval at foo (/app/src/a.ts:3:4), <anonymous>:1:2` → `src/a.ts`, line 3, col 4). Otherwise `^(.+):(\d+):(\d+)$`.
- Parsing stops after 50 frames.

**`in_app`** uses the raw `abs_path`: `false` when it contains `/node_modules/` or `\node_modules\`, starts with `node:`, or equals `native` or `<unknown>`. Everything else is `true`, including a nonexistent path.

**`context_line`**: only when `in_app` is true and `abs_path` and `lineno` are set. The SDK reads the whole file with `fs.readFileSync(abs_path, "utf8")` on every frame, splits on `\n`, and takes line `lineno - 1` verbatim (a CRLF file keeps the `\r`). Any read error is ignored. There is no `pre_context` or `post_context`. Edge runtimes skip this step.

**`filename` normalization** (`makeRelativePath`, in order; `abs_path` always keeps the original):

1. Strip `file://`, and add a leading `/` if the result does not start with `/` or a drive letter.
2. If the path does not start with `/` and does not match `^[A-Za-z]:\\`, return it as is (after `node:internal…` → `node:internal`, `node:x/y` → `node:x`). So `C:/projects/app/src/index.ts` comes back unchanged (probe).
3. Replace every `\` with `/`.
4. `node:` paths → the rule in step 2.
5. If the path contains `/node_modules/`, return the part from the LAST `node_modules/`.
6. Replace a leading `/Users/<u>/`, `/home/<u>/`, or `<drive>:/Users/<u>/` with `~/`.
7. Remove a leading `/var/www/<x>/`, `/var/task/`, `/usr/src/app/`, `/app/`, `/opt/<x>/`, `/srv/<x>/` (applied in that order).
8. If the path starts with `process.cwd()`, remove `cwd.length + 1` chars.
9. If the path still starts with `/` or a drive, cut it at the LAST of the primary markers `/src/`, `/lib/`, `/dist/`, `/build/` (checked in that order), else at the secondary markers `/app/`, `/components/`, `/pages/`, `/api/`, `/utils/`, `/services/`, `/modules/`. A `~/…` path gets the same cut, but keeps `~/` when no marker matches (`/home/bob/proj/thing.ts` → `~/proj/thing.ts`).
10. Remove one leading `/`.

**Cause chain** (`unwrapErrorCauses`): follows `error.cause` while it is truthy, up to 10 entries. A non-Error cause adds `{message: stringified, type: undefined}` and stops. A cause already seen stops the walk. The top-level error is not in the "seen" set, so a two-error cycle yields `["Error 1", "Error 2"]` for `captureException(error2)` (the top error appears again).

**How the server uses it** (audit 02 §5.7): grouping uses `type` and frame contributions, never `stack`, `lineno`, `colno`, or `abs_path`. The JS rules (only for `platform` `javascript`, `node`, `js`) use `context_line` when it is 120 chars or fewer, else the function name. Non-JS rules use `module|function` when `module` is set, else `basename(filename).lower()|function`. The server takes the issue source from the FIRST in-app frame in list order. One invalid frame (missing `filename` or `function`, `in_app: null`, float `lineno`) cancels grouping for the whole error.

**Rust mapping** (recommendation; the decisions belong to the port):

| TS source | Rust source | What to send |
| --- | --- | --- |
| Thrown `Error`, stashed by the inner tap | A tool fn returns `Err(E)` with `E: std::error::Error`. In rmcp 3.5.1, `Result<T, E>` with `E: IntoContents` becomes `CallToolResult` with `is_error: Some(true)` before `Tracked<H>::call_tool` sees it (`rmcp-3.5.1/src/handler/server/tool.rs:125-133`), so the original type is lost, as in TS. To keep it, offer an inner-tap analog (for example a helper that records the error in a task-local before conversion). | `message` = `Display`; `type` = last path segment of `std::any::type_name::<E>()` (no `::`); `stack` = backtrace `Display` if one was captured (`anyhow::Error::backtrace()`, a stored `std::backtrace::Backtrace`); `frames` from the resolved frames; `chained_errors` from `source()`. |
| `{isError: true}` result | `CallToolResult { is_error: Some(true), .. }` | `message` = text contents joined with one space and trimmed, else `"Unknown error"`; no `type`, `stack`, `frames`. Same as TS. |
| Handler rejects (rethrown) | `call_tool` returns `Err(rmcp::ErrorData)` (a JSON-RPC error) | `message` = `ErrorData.message`; a short `type` such as `McpError(-32602)`; no frames unless the SDK captured a backtrace. Publish, then return the same `Err`. |
| Uncaught throw | A panic in the handler future | Catch with `futures::FutureExt::catch_unwind` on `AssertUnwindSafe(fut)`. `message` = payload `&str`/`String`, else a fixed text; `type` for example `panic`; frames from a backtrace captured in a chained panic hook (`Backtrace::force_capture()`) plus the panic location. Publish, then `resume_unwind` so behavior does not change. |
| `Error.cause` | `source()` | Max 10, outer to inner, pointer-identity cycle guard. Per-cause `stack`/`frames` only when that cause carries a backtrace. |
| `platform: "javascript"` | — | `"rust"`. Never `javascript`, `node`, or `js`. |

Rust frame rules (recommendation, consistent with audit 02 §5.7.1):

- `function`: demangled name without the `::h<16 hex>` hash; normalize `{{closure}}` and async markers to stable text. A frame without a symbol gets a fixed placeholder.
- `module`: the crate and module path (for example `my_crate::tools`). The TS `StackFrame` has no `module` key, but the server schema accepts it, and the non-JS grouping rules prefer it.
- `filename`: relative path. Strip `./`. Map `…/.cargo/registry/src/<index>/<crate>-<ver>/…` and `…/.cargo/git/checkouts/…` to a crate-relative path. Map `/rustc/<commit>/library/<crate>/…` to `<crate>/…`. Then apply the home, deployment, and `src/` rules above.
- `abs_path`: the path from debug info, unchanged. `lineno`/`colno`: integers, or omitted.
- `in_app: false` for toolchain paths (`/rustc/`, `library/std|core|alloc`), registry and git dependency paths, `vendor/`, frames without a file, and frames of the panic and backtrace machinery (`std::panicking`, `core::panicking`, `rust_begin_unwind`, `std::backtrace`). Do NOT reuse the JS `isInApp`: a probe shows it marks `~/.cargo/registry/…/tokio-1.40.0/src/runtime/task.rs` and `/rustc/abc123/library/std/src/panicking.rs` as `in_app: true` with filename `src/…`.
- Order: innermost first, like V8. Drop the leading capture frames, because the server takes the first in-app frame as the issue source.
- `context_line`: read from `abs_path` only for in-app frames; skip silently when the file is missing. Avoid sync file reads on the request path (TS does them).

Quirks:

- `context_line` never appears in a real Node ESM process. V8 reports ESM frames as `file:///…` URLs, and `readFileSync` gets that URL string as a path and fails (probe: a real frame had `abs_path "file:///…/probe.mts"` and no `context_line`). Row 15 passes only because vitest reports plain paths.
- The top-level error is not in the cycle guard, so a cycle repeats the top error once (row 22).
- Chained `type` falls back to `"Error"`, while top-level `type` can be absent. Chained entries carry no `platform`.
- An unparseable line gives a frame without `abs_path` or `lineno`. `at functionWithNoParens /file:20:5` gives filename `functionWithNoParens /file` with `in_app: true`, and the SDK then tries to read a file by that name (row 34).
- Step 8 has no separator check: with `cwd = /x/exc`, the path `/x/exc2/deep/x.ts` becomes `deep/x.ts` (probe). It also never matches a project under `/home/<u>` or `/Users/<u>`, because step 6 already rewrote those to `~/`.
- A drive path with forward slashes (`C:/projects/app/src/index.ts`) is returned unchanged, but the mixed form in row 56 is normalized.
- A thrown `Symbol` or function produces no `message` key.
- `context_line` reads run synchronously on the tool-call path, up to 50 reads per error plus 50 per cause, with no cache.

---

### src/tests/extra-projection.test.ts (8 tests)

Purpose: `projectExtraForEvent(extra)`, which turns the request-handler context into plain JSON for `event.parameters.extra`.

Harness: direct calls. `vi.mock("../modules/logging")` replaces `writeToLog` with `vi.fn()`. Inputs are WHATWG `Request` objects, `URL` objects, and plain objects. Identity checks use `toBe`.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | projectExtraForEvent > v2: web Request under http.req > projects a real Request into plain { method, url, headers } | `req = new Request("http://localhost:4105/mcp", {method: "POST", headers: {"x-agentcat-health-run": "run:server"}})`, `extra = {sessionId: "s", mcpReq: {id: 7}, http: {authInfo: {token: "t"}, req}}`. `out.http.req` equals `{method: "POST", url: "http://localhost:4105/mcp", headers: objectContaining({"x-agentcat-health-run": "run:server"})}`. After `JSON.parse(JSON.stringify(out))`, the header is still `"run:server"`. `out.http.authInfo` and `out.mcpReq` are the same references; `out.sessionId === "s"`. | adapt: for streamable HTTP, read `http::request::Parts` from the rmcp `RequestContext` extensions and build `{http: {req: {method, url, headers}}}`. `Parts.uri` is usually origin-form (`/mcp`), so rebuild the absolute URL from the scheme and `Host` header to match. Duplicate headers: `Headers.entries()` joins values with `", "`. |
| 2 | … > v2: web Request under http.req > lowercases header names (Headers semantics) | Header `X-Mixed-Case: v` → `headers["x-mixed-case"] === "v"`, `headers["X-Mixed-Case"]` undefined. | port |
| 3 | … > v2: web Request under http.req > is idempotent: a projected req has no headers.entries and passes through | `second = project(first)` → `second` is the same object as `first`. | N/A: JS duck typing on `headers.entries`; Rust projects typed `Parts` once. |
| 4 | … > v2: web Request under http.req > returns the original extra when headers.entries throws | `headers.entries` throws `Error("boom")` → returns the original `extra` (same object). It logs `Warning: Failed to project request context into event parameters, publishing extra as-is - Error: boom` (not asserted). | N/A: a JS getter that throws; the Rust projection cannot fail this way (it still must never cost the event). |
| 5 | … > v1: URL instance under requestInfo.url > replaces the URL instance with its href, headers untouched by reference | `{requestInfo: {headers: {"x-corr": "1"}, url: new URL("http://localhost:4105/mcp?q=1")}}` → `requestInfo.url === "http://localhost:4105/mcp?q=1"`; `requestInfo.headers` is the same object. | N/A: TS v1 SDK `URL` object; row 1 covers the Rust URL string. |
| 6 | … > v1: URL instance under requestInfo.url > passes an already-string url through unchanged | `{requestInfo: {headers: {a: "b"}, url: "http://localhost/mcp"}}` → same object. | N/A: TS v1 SDK shape. |
| 7 | … > pass-through > returns non-rewritable values by identity | Same object back for: stdio `{sessionId: "s", mcpReq: {id: 1}}`, `null`, `undefined`, `"nope"`, `{http: {}}`, `{http: {req: {headers: {a: "b"}}}}`, `{http: {req: {headers: 42}}}`. | adapt: only the stdio case applies: a stdio request yields no `http` block. |
| 8 | … > snapshot semantics > later top-level mutations of the original do not appear in the projection | After projection, `extra.__agentcat_error = new Error("stashed mid-handler")` → `out !== extra` and `"__agentcat_error" in out === false`. | N/A: JS shared mutable `extra` and the inner-tap stash; a Rust projection is a new `serde_json::Value`. |

Quirks:

- Headers are published verbatim, including `authorization` and `cookie` (audit 02 §8 item 20). No test here covers a denylist.
- `Object.fromEntries(headers.entries())` keeps only the last `set-cookie` value, because `Headers.entries()` yields `set-cookie` entries separately.

---

### src/tests/handle-conflict-report.test.ts (9 tests)

Purpose: the log for a tool that declares its own `session_id`, the dedupe of that log, and the record of tools that own `session_id`.

Harness: `vi.mock("../modules/logging")` replaces `writeToLog` with a spy, cleared before each test. Tests call `addHandleParametersToTools` and `buildInjectedList` (`src/engine/listWrap.ts:34-83`) directly. `data(overrides)` returns `{projectId: "proj_test", options: {enableReportMissing: false, enableTracing: true, enableToolCallContext: false, enableAgentTracking: false, ...overrides}}`. `ownSessionTool()` returns `{name: "own_session", inputSchema: {type: "object", properties: {session_id: {type: "string"}}}}`. Log lines are filtered by tool name.

Full collision log text (`handle-injection.ts:151-157`): `` ERROR: Tool "own_session" already declares a 'session_id' parameter. AgentCat will not inject its own, and calls to this tool are published without a session, so they cannot be correlated. Your parameter is untouched and still reaches your handler. If you already manage sessions, pass a resolveSessionId hook to track() — AgentCat will derive its session from your identifier and stop injecting session_id entirely. ``

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | session_id collision report > reports a session_id collision once per tool, as an error with remediation | Three passes with the same `reportedConflicts` set and `{injectSessionId: true, injectAgentId: false}` → exactly 1 log line mentions `own_session`. It contains `ERROR:`, `resolveSessionId`, and `without a session`. | port |
| 2 | session_id collision report > reports two different colliding tools each once | Tools `tool_a` and `tool_b` (both declare `session_id`), two passes → 1 log line each. | port |
| 3 | session_id collision report > logs the error even without a dedupe set — once per pass, no dedupe | No `reportedConflicts` → no throw; two passes → 2 log lines, both contain `ERROR:`. | N/A: a defensive TS path (`?.`); Rust can make the dedupe set mandatory. |
| 4 | session_id collision report > getReportedConflicts returns a stable set per key, and independent sets for different keys | Same key → same `Set` object; different keys → different sets; adding `tool_x` to set A does not affect set B; `getReportedConflicts(keyA).has("tool_x") === true`. | adapt: per-server state on the tracked wrapper instead of a `WeakMap` keyed by object. |
| 5 | session_id collision report > buildInjectedList dedupes per AgentCatData object: repeat tools/list on the same server logs once, a second server logs its own once | Three `buildInjectedList(serverA, [ownSessionTool()])` → 1 log line. Two more on `serverB` → 2 log lines in total. | adapt: dedupe state lives on each tracked server. |
| 6 | customer-declared session_id tracking > records only the tools whose own schema declares session_id | Tools `own_session`, `union_tool` (`{type: "object", oneOf: [{properties: {kind: {const: "a"}}}, {properties: {kind: {const: "b"}}}]}`), `normal` (`{q: string}`) → declared set has `own_session`, not `union_tool`, not `normal`. | port |
| 7 | customer-declared session_id tracking > records a composed schema that also declares session_id at its root | `union_with_own_session` = the `oneOf` schema plus root `properties: {session_id: {type: "string"}}` → recorded. | port |
| 8 | customer-declared session_id tracking > keys per AgentCatData object, like reportedConflicts | After `buildInjectedList(serverA, …)`, A has `own_session`, B does not. | adapt: per-server state. |
| 9 | customer-declared session_id tracking > records nothing in hook mode, where session_id is never injected | `data({resolveSessionId: () => "customer-7"})` → declared set size `0`. | port |

Quirks:

- The declared set only grows until the process restarts. A tool that once declared `session_id` stays "foreign" even after its schema changes.

---

### src/tests/handle-injection.test.ts (28 tests)

Purpose: schema injection of `session_id`, `agent_id`, and `mcp_session`; argument stripping; registry storage.

Harness: pure function calls from `src/modules/handle-injection.ts` and `src/modules/context-parameters.ts`. Fixtures: `makeTool(name, schema)` = `{name, inputSchema: schema, callback: async () => ({content: []})}`; `customerSchema()` = `{type: "object", properties: {text: {type: "string"}}, required: ["text"]}`; `structuredTool(name, outputSchema)` = `customerSchema()` input plus that `outputSchema`; `objectOutputSchema()` = `{type: "object", properties: {count: {type: "number"}}, required: ["count"], additionalProperties: false}`. The input registry is `Map<toolName, Set<param>>`; the output registry is `Set<toolName>`.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | addHandleParametersToTools > injects session_id and agent_id after customer params, both required | `{injectSessionId: true, injectAgentId: true}` → property keys `["text", "session_id", "agent_id"]`; `required` `["text", "session_id", "agent_id"]`; `session_id.description === SESSION_ID_PARAM_DESCRIPTION`; `session_id.pattern === "^(start\|ses_[0-9A-Za-z]{27})$"`; `agent_id.description === AGENT_ID_PARAM_DESCRIPTION`; `agent_id.pattern` undefined; registry `add_todo` = `{session_id, agent_id}`. | port |
| 2 | addHandleParametersToTools > creates the required array when the schema has none | `{type: "object", properties: {}}` → `required` `["session_id", "agent_id"]`. | port |
| 3 | addHandleParametersToTools > does not duplicate session_id in an existing required array | `required: ["session_id"]` with no such property, session only → `required` stays `["session_id"]`. | port |
| 4 | addHandleParametersToTools > does not duplicate agent_id in an existing required array | `required: ["agent_id"]`, agent only → stays `["agent_id"]`. | port |
| 5 | addHandleParametersToTools > leaves required untouched when the customer declares agent_id | Customer `agent_id: {type: "string", description: "mine"}`, `required: []` → `required` `[]`; description stays `"mine"`. | port |
| 6 | addHandleParametersToTools > full order with context injector: customer, session_id, agent_id, context | Handles, then `addContextParameterToTools(tools, undefined, registry)` → keys and `required` both `["text", "session_id", "agent_id", "context"]`; registry `{session_id, agent_id, context}`. | port |
| 7 | addHandleParametersToTools > hook mode: no session_id, agent_id uses the same single description | `{injectSessionId: false, injectAgentId: true}` → no `session_id`; `agent_id.description === AGENT_ID_PARAM_DESCRIPTION`; `required` `["text", "agent_id"]`; registry `{agent_id}`. | port |
| 8 | addHandleParametersToTools > skips a param the tool already defines, and does not record it | Customer `session_id: {type: "string", description: "customer's own"}` stays unchanged with no `pattern`; `required` `["text", "agent_id"]`; registry `deploy` = `{agent_id}`. | port |
| 9 | addHandleParametersToTools > skips complex schemas entirely | `{oneOf: [{type: "object"}]}` → `properties` undefined, `required` undefined, registry has no `odd`. | port |
| 10 | addHandleParametersToTools > removes additionalProperties:false and handles missing schemas | `customerSchema()` + `additionalProperties: false` → `additionalProperties` removed. A tool with no `inputSchema` → keys `["session_id", "agent_id"]`, `required` the same. | adapt: rmcp `Tool.input_schema` is always present (`Arc<JsonObject>`), so the "missing schema" half becomes "empty object schema". |
| 11 | addHandleParametersToTools > does NOT skip get_more_tools (handles are injected; context stays bespoke) | `get_more_tools` with `properties: {context: {type: "string", description: "bespoke"}}`, `required: ["context"]`, then the context injector → keys `["context", "session_id", "agent_id"]`; `context.description === "bespoke"`; `required` `["context", "session_id", "agent_id"]`; registry `{session_id, agent_id}`. | port |
| 12 | addHandleParametersToTools > agent_id copy prescribes the self-chosen model\|harness\|nonce format | `AGENT_ID_PARAM_DESCRIPTION` contains `required on every call including your first`, `opus-4.80-1m\|claude-code\|k3n9x`, and `never inherited`. It does not contain `the server will issue` or `session_id`. | port |
| 13 | stripInjectedArguments > strips exactly what was injected | Registry `add_todo` = `{session_id, agent_id, context}`; args `{text: "x", session_id: "ses_1", agent_id: "agt_1", context: "why"}` → `{text: "x"}`. | port |
| 14 | stripInjectedArguments > preserves the customer's own session_id param | Registry `deploy` = `{agent_id, context}`; args `{session_id: "prod-42", agent_id: "agt_1", context: "why"}` → `{session_id: "prod-42"}`. | port |
| 15 | stripInjectedArguments > falls back to stripping all three when no registry entry exists | `unlisted_tool` with args `{text: "x", session_id: "a", agent_id: "b", context: "c"}` → `{text: "x"}`. With registry `undefined`, `{text: "x", session_id: "a"}` → `{text: "x"}`. | port |
| 16 | stripInjectedArguments > fallback never strips get_more_tools' own context | `get_more_tools`, no registry, `{context: "need a tool", session_id: "a", agent_id: "b"}` → `{context: "need a tool"}`. | port |
| 17 | stripInjectedArguments > handles non-object args | `undefined` args → `undefined`. | adapt: rmcp arguments are `Option<JsonObject>`; `None` passes through. |
| 18 | registry storage + request cloning > stores per server object | `setInjectedParamsRegistry(server, registry)` → `getInjectedParamsRegistry(server)` is the same object; another `{}` → `undefined`. | adapt: the registry lives on the `Tracked` wrapper. |
| 19 | registry storage + request cloning > clones the request with stripped arguments, leaving the original intact | Request `{params: {name: "add_todo", arguments: {text: "x", session_id: "ses_1", context: "why"}}}` → clone has `{text: "x"}`; the original still has `session_id: "ses_1"` (the event keeps the raw request). | port |
| 20 | outputSchema injection > injects mcp_session into a plain object outputSchema and registers the tool | `get_stats` with `objectOutputSchema()` → `outputSchema.properties.mcp_session`: `type "object"`, `description === MCP_SESSION_FIELD_DESCRIPTION`, sub-keys `["session_id", "agent_id", "status"]`, `status` equal `{type: "string", enum: ["issued", "active", "unrecognized"], description: MCP_SESSION_STATUS_DESCRIPTION}`. `required` stays `["count"]`, `additionalProperties` stays `false`, `count` stays `{type: "number"}`. Output registry has `get_stats`. | port |
| 21 | outputSchema injection > sub-properties track modes: hook mode drops session_id and status, tracking off drops agent_id | Hook mode (`injectSessionId: false`) → sub-keys `["agent_id"]`, description `MCP_SESSION_FIELD_DESCRIPTION_HOOK_MODE`. Agent tracking off → `["session_id", "status"]`, description `MCP_SESSION_FIELD_DESCRIPTION`. | port |
| 22 | outputSchema injection > does not mutate the customer's outputSchema object | The original schema has no `mcp_session` afterwards. | port |
| 23 | outputSchema injection > skips complex outputSchema (oneOf/allOf/anyOf) and does not register | `{oneOf: [{type: "object", properties: {a: {type: "string"}}}]}` → `outputSchema` equal to the input; output registry size `0`; input schema still gets `session_id`. | port |
| 24 | outputSchema injection > composed inputSchema: early-return skips output injection too, nothing registered | `inputSchema` `oneOf` → `outputSchema` equal to `objectOutputSchema()` with no `mcp_session`; size `0`. | port |
| 25 | outputSchema injection > collision: a customer-declared mcp_session is never clobbered | `outputSchema.properties.mcp_session = {type: "string"}` stays; size `0`. | port |
| 26 | outputSchema injection > no outputSchema: untouched, nothing registered | `outputSchema` undefined; size `0`. | port |
| 27 | outputSchema injection > no injection at all (neither handle) leaves outputSchema alone | Both flags `false` → no `mcp_session`; size `0`. | port |
| 28 | outputSchema injection > output registry accessors are keyed per server object | `getOutputInjectionRegistry(server)` undefined before set; same object after `setOutputInjectionRegistry`; another `{}` → undefined. | adapt: per-server state on the wrapper. |

Quirks:

- The fallback strip (no registry entry) removes `session_id`, `agent_id`, and `context` even from a tool that owns them. A `tools/call` that arrives before any `tools/list` on that instance loses the customer's own values. `callWrap.ensureRegistries` rebuilds the registry first, which hides this, but a rebuild timeout (5 s) or failure brings it back.
- Input schemas lose `additionalProperties: false`, which loosens the customer's validation. Output schemas keep it.
- Key order matters (rows 1, 6, 11, 20). Rust needs `serde_json` with `preserve_order`.

---

### src/tests/handler-property.test.ts (3 tests)

Purpose: in the TS v1 SDK, a registered tool stores its function under `handler` (SDK 1.24 and later) or `callback` (1.23 and earlier). `track()` must keep the same property name.

Harness: `new McpServer({name: "test", version: "1.0.0"})`, `server.tool(...)`, then reads `(server as any)._registeredTools`. Writes diagnostic lines with `console.log`.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | MCP SDK callback/handler compatibility > should have either callback or handler property (SDK version dependent) | `test_tool` (`{a: z.number()}`) has a function under `handler` or `callback`. | N/A: TS MCP SDK internals. |
| 2 | MCP SDK callback/handler compatibility > should preserve the original property name after track() is called | After `track(server, "test-project-id")`, the property name is the same and its value is a function. | N/A: monkey-patching `_registeredTools`. |
| 3 | MCP SDK callback/handler compatibility > should preserve property name for tools registered after track() | `late_tool` (`{b: z.string()}`) registered after `track()` uses the same property name as `initial_tool`. | N/A: monkey-patching `_registeredTools`. |

Quirks: none.

---

### src/tests/handles-integration.test.ts (28 tests)

Purpose: end-to-end handle behavior through a real client and server: schema injection, mint-back, stripping, foreign and composed schemas, hook mode, option switches, and event contents.

Harness:

- Real v1 SDK `Server` (low-level) or `McpServer` (high-level todo fixture) plus `Client({name: "test client", version: "1.0"})` over `InMemoryTransport.createLinkedPair()`. `setupTestHooks()` at file level.
- `setupLowLevel(opts)`: `Server({name: "low-level test server", version: "1.0"}, {capabilities: {tools: {}}})`. `tools/list` returns one tool `echo` (description `echoes`, `inputSchema {type: "object", properties: {text: {type: "string"}}, required: ["text"]}`). `tools/call` pushes `request.params.arguments` into `receivedArgs`; text `explode` returns `{isError: true, content: [{type: "text", text: "boom"}]}`, else `{content: [{type: "text", text: "echo: ${text}"}]}`. It calls `track(server, "proj_test", {enableAgentTracking: true, ...opts})` BEFORE connect.
- `setupOwnSessionId()`: `Server({name: "own session id server", version: "1.0"})`, tool `echo` (description `declares its own session_id`) with properties `text` and `session_id: {type: "string", description: "customer's own"}` (no `required`). It returns `ok`. `track(server, "proj_test", {enableAgentTracking: true})`.
- `setupComposedSchema()`: `Server({name: "composed schema server", version: "1.0"})`, tool `union_tool` (description `takes one of two shapes`) with `inputSchema {type: "object", oneOf: [{properties: {kind: {const: "a"}, a: {type: "string"}}, required: ["kind", "a"]}, {properties: {kind: {const: "b"}, b: {type: "number"}}, required: ["kind", "b"]}]}`. It returns `ok`. `track(server, "proj_test")` with defaults (agent tracking off).
- `mintBackOf(result)` returns `content[0].text` only when it starts with `[session_id`, so every use also checks that the block leads. `handleFrom(text, name)` uses `` new RegExp(`${name}: (\\S+)`) ``.
- An `EventCapture` starts in `beforeEach` of each describe. Events are read right after `callTool`, or after `capture.flush()` for hook-resolved fields.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | low-level server: explicit handles > injects required handles into listed tool schemas | `echo` keys and `required` both `["text", "session_id", "agent_id", "context"]`; `session_id.pattern === SESSION_ID_PARAM_PATTERN`; `get_more_tools` has `session_id` and `agent_id` properties and `annotations` equal `{title: "Get More Tools", readOnlyHint: true, destructiveHint: false, idempotentHint: true, openWorldHint: false}`. | port |
| 2 | low-level server: explicit handles > publishes no tools/list or initialize events | After `listTools()` and `callTool({name: "echo", arguments: {text: "hi", context: "testing"}})`, event types exclude `mcp:tools/list` and `mcp:initialize` and include `mcp:tools/call`. | port |
| 3 | low-level server: explicit handles > first call sends start: mints session_id, prepends the mint-back, tags the source | Args `{text: "hi", context: "testing intent", session_id: "start"}` → the mint-back block contains `[session_id issued` and not `agent_id`; `handleFrom(block, "session_id")` matches `^ses_`; `event.sessionId` equals it; tags include `agentcat_session_id_source: "minted"`; no `agentcat_agent_id` or `agentcat_agent_id_source`. | port |
| 4 | low-level server: explicit handles > echoed handles are reused verbatim with no mint-back | Args `{text: "hi", context: "testing", session_id: sid("fixed"), agent_id: "opus-4.80-1m\|claude-code\|k3n9x"}` → no mint-back; `event.sessionId === sid("fixed")`; tags include `agentcat_session_id_source: "supplied"`, `agentcat_agent_id: "opus-4.80-1m\|claude-code\|k3n9x"`, `agentcat_agent_id_source: "supplied"`. | port |
| 5 | low-level server: explicit handles > subagent flow: inherited task + omitted agent yields no mint-back, task tagged supplied | `session_id: sid("parent")`, no `agent_id` → no mint-back; source `supplied`; no agent tags. | port |
| 6 | low-level server: explicit handles > prepends the mint-back on isError results too | Text `explode` → `result.isError === true`; mint-back contains `[session_id issued` and not `agent_id`. | port |
| 7 | low-level server: explicit handles > strips injected params before the customer handler; event keeps them | Args `{text: "hi", context: "why", session_id: sid("x"), agent_id: "agt_x"}` → `receivedArgs[0]` equals `{text: "hi"}`; `event.parameters.request.params.arguments.session_id === sid("x")`. | port |
| 8 | low-level server: explicit handles > a customer's own session_id is never adopted and still reaches the handler | Listed `echo.session_id.pattern` undefined; `required` does not contain `session_id`. Args `{text: "hi", session_id: "CUSTOMER-VALUE-123"}` → handler receives `session_id: "CUSTOMER-VALUE-123"`; `event.sessionId === ""`; `tags.agentcat_session_id_source === "foreign"`. | port |
| 9 | low-level server: explicit handles > a customer's own session_id param: start is their value too, never a sentinel | Args `{text: "hi", session_id: "start"}` → handler receives `"start"`; `event.sessionId === ""`; source `foreign`. | port |
| 10 | low-level server: explicit handles > composed-schema tool: injection is skipped, yet the call still mints a session and announces it | Listed `union_tool.inputSchema.properties ?? {}` has no `session_id`; if the client still sees `oneOf`, it has length 2. Call `{kind: "a", a: "x"}` → mint-back contains `[session_id issued`; `event.sessionId` equals the issued id; source `minted`. | port |
| 11 | low-level server: explicit handles > composed-schema tool: an echoed session_id AgentCat issued is honored, not called foreign | Call `{kind: "b", b: 2, session_id: sid("union")}` → no mint-back; `event.sessionId === sid("union")`; source `supplied`. | port |
| 12 | low-level server: explicit handles > hook mode: no session_id param, hook-derived session id, no task mint-back | Options `{resolveSessionId: () => "customer-correlation-7"}` (plus agent tracking). `listTools` twice. `echo` has no `session_id`, has `agent_id`; `required` contains `agent_id` and not `session_id`. Call with `agent_id: "opus-4.80-1m\|claude-code\|k3n9x"` → no mint-back. After `flush()`, tags include `agentcat_session_id_source: "hook"`, `agentcat_agent_id: "opus-4.80-1m\|claude-code\|k3n9x"`, `agentcat_agent_id_source: "supplied"`; `event.sessionId` matches `^ses_` (it is `deriveSessionId("customer-correlation-7", "proj_test")`). | port |
| 13 | low-level server: explicit handles > enableTracing:false — no handle params injected, no mint-back, no events | `echo` has no `session_id` and no `agent_id`; call → no mint-back; no `mcp:tools/call` event. | port |
| 14 | low-level server: explicit handles > enableAgentTracking:false — no agent param, tag, or mint-back line | No `agent_id` property; mint-back contains `session_id: ` and not `agent_id`; no `agentcat_agent_id` tag. | port |
| 15 | low-level server: explicit handles > enableReportMissing:false — handles still inject, no get_more_tools | No `get_more_tools` tool; `echo` has `session_id` and `agent_id`, both in `required`. | port |
| 16 | low-level server: explicit handles > stamps _meta clientInfo and protocol version when present | Calls the wrapped handler from `server._requestHandlers.get("tools/call")` directly with `params._meta` `{"io.modelcontextprotocol/clientInfo": {name: "modern-client", version: "2.0"}, "io.modelcontextprotocol/protocolVersion": "2026-07-28"}` and `extra = {}` → `event.clientName === "modern-client"`; tags include `agentcat_protocol_version: "2026-07-28"`. | adapt: call `Tracked::call_tool` directly with request params whose `_meta` carries those keys, plus a test `RequestContext`. |
| 17 | low-level server: explicit handles > declares mcp_session in a listed plain-object outputSchema | Server `low-level structured server`, tool `stats` (description `returns stats`, `inputSchema {type: "object", properties: {}}`, `outputSchema {type: "object", properties: {count: {type: "number"}}, required: ["count"], additionalProperties: false}`), returns `{content: [{type: "text", text: "42"}], structuredContent: {count: 42}}`. Listed `outputSchema.properties.mcp_session.type === "object"`; sub-keys `["session_id", "agent_id", "status"]`. | port |
| 18 | high-level server: explicit handles > mints on first call, prepends exactly one mint-back block | Todo fixture, `track(server, "proj_test")`. `add_todo` `{text: "buy milk", context: "testing handles"}` → exactly one content block starts with `[session_id`, and it is `content[0]`; `event.sessionId` equals the id in that block. | port |
| 19 | high-level server: explicit handles > callback receives stripped args; supplied handles reused silently | Agent tracking on; args `{text: "buy milk", context: "testing", session_id: sid("hl"), agent_id: "agt_hl"}` → `content[0].text` contains `buy milk`; no mint-back; `event.sessionId === sid("hl")`; source `supplied`. | port |
| 20 | high-level server: explicit handles > agent tracking is OFF by default: no param, no tag, task-only mint-back | No listed tool has `agent_id`; `add_todo` `{text: "buy milk", context: "testing defaults"}` → mint-back contains `[session_id issued` and not `agent_id`; no agent tags. | port |
| 21 | high-level server: explicit handles > get_more_tools carries handles and the mint-back | Agent tracking on; `get_more_tools` `{context: "need a search tool"}` → mint-back issued, no `agent_id`; `event.sessionId` matches `^ses_`; `event.userIntent === "need a search tool"`. | port |
| 22 | high-level server: explicit handles > enableTracing:false — get_more_tools still answers, no mint-back, no event | `get_more_tools` is listed; the call returns `content[0].text` containing `we have shown you the full tool list`; no mint-back; no event. | port |
| 23 | high-level server: explicit handles > identify runs per call and stamps the event; no identify event published | `identify: async () => ({userId: "actor-7", userName: "Sam"})`. After `flush()`, no `agentcat:identify` event; the tool-call event has `identifyActorGivenId "actor-7"`, `identifyActorName "Sam"`. | port |
| 24 | published events exclude SDK-authored mint-back text > low-level: minting call — block on the wire, absent from event.response | `{text: "hi", context: "testing intent"}` → mint-back on the wire; `JSON.stringify(event.response)` has no `[session_id`; `event.response.content` equals `[{type: "text", text: "echo: hi"}]`. | port |
| 25 | published events exclude SDK-authored mint-back text > low-level: minting error call — event.response and event.error stay clean | Text `explode` → `isError` on the wire and mint-back present; `event.isError === true`; `event.response` has no `[session_id`; `event.error.message` contains `boom` and not `[session_id`. | port |
| 26 | published events exclude SDK-authored mint-back text > low-level: echoed handles — event.response is exactly the handler result | Args with `session_id: sid("fixed")`, `agent_id: "agt_fixed"` → `event.response` equals `{content: [{type: "text", text: "echo: hi"}]}`. | adapt: `event.response` is the serialized rmcp `CallToolResult`. In rmcp 3.5.1, `CallToolResult::success` sets `result_type: Some(complete)` and `is_error: Some(false)`, so compare with the Rust serialization (`resultType`, `isError: false`), not the TS literal. |
| 27 | published events exclude SDK-authored mint-back text > high-level: minting call — block on the wire, absent from event.response | Agent tracking on; `add_todo` `{text: "buy milk", context: "testing handles"}` → mint-back issued, no `agent_id`; `event.response` has no `[session_id` and contains `buy milk`. | port |
| 28 | published events exclude SDK-authored mint-back text > high-level: get_more_tools minting call — event.response keeps only the answer | `get_more_tools` `{context: "need a search tool"}` → mint-back issued; `event.response` has no `[session_id` and contains `we have shown you the full tool list`. | port |

Quirks:

- Row 10: a tool whose schema never advertised `session_id` still gets a mint-back that tells the agent to send `session_id`. It works only because the wrapper strips the argument before SDK validation.
- Row 22: `get_more_tools` still answers when tracing is off, and the call is not published.
- Row 12 calls `listTools()` twice for no reason. Row 16 drives the private `_requestHandlers` map.
- Row 10 tolerates SDK 1.22, which strips `oneOf` from the client's view of the schema.

---

### src/tests/handles.test.ts (78 tests)

Purpose: the pure handle functions in `src/modules/handles.ts`: session id minting and derivation, handle extraction, `resolveHandles`, the session hook, mint-back text and structured mint-back, handle tags, and `isValidSessionId`.

Harness: direct calls, no mocks. Constants: `T = "ses_2xF9kQm3rTvB8nLpYw7ZcHd4Ke1"`, `A = "opus-4.80-1m|claude-code|k3n9x"`, `MINTED_BLOCK(id) = "[session_id issued — see this tool's session_id parameter description]\n" + "session_id: " + id + "\n" + "This is the first-call issuance described in this tool's session_id parameter description."`. `req(args) = {params: {name: "add_todo", arguments: args}}`. Tests `await resolveHandles(...)` although it is synchronous. Rows 34–38 and 63–71 come from `it.each` tables.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | handle primitives > mints session ids with the ses_ prefix | `newSessionId()` matches `^ses_[0-9A-Za-z]{27}$`; a second call differs. | port |
| 2 | handle primitives > derives deterministic session ids from id+project | `deriveSessionId("customer-abc", "proj_1")` twice → equal, starts with `ses_`. | port |
| 3 | handle primitives > different project yields a different derived id | `("customer-abc", "proj_1")` ≠ `("customer-abc", "proj_2")`. | port |
| 4 | handle primitives > derives without a project id | `deriveSessionId("customer-abc")` is stable and differs from the `"proj_1"` variant. | port |
| 5 | handle primitives > pins derived session ids to their golden values | `deriveSessionId("customer-abc", "proj_1") === "ses_2cOHEO0LYGADMzRvWTXXVbbgxgm"`; `deriveSessionId("customer-abc") === "ses_2cZY3tvyI25O2AmL2CGVo2B1IIj"`. Construction: `sha256(projectId ? id + ":" + projectId : id)`; time = `2024-01-01T00:00:00Z` ms + `readUInt32BE(hash, 0) % 31536000000`; payload = hash bytes 4..20; KSUID `fromParts` (seconds, floored). | port |
| 6 | handle primitives > does not trim its input | `deriveSessionId(" x ", "p")` ≠ `deriveSessionId("x", "p")`; `" x ","p"` → `"ses_2c3yR5mYKQdLaXsJNgZH6erbfQK"`; `"x","p"` → `"ses_2bw285VY9apdgUgTPXKFnT6P4G0"`. | port |
| 7 | handle primitives > extractHandle returns trimmed non-empty strings only | `{session_id: " ses_x "}` → `"ses_x"`; `""` → undefined; `"   "` → undefined; `42` → undefined; args `undefined` → undefined; `null` (agent) → undefined; `"not-an-object"` → undefined. | adapt: arguments are `Option<JsonObject>`; the non-object inputs become `None`; the string and number cases port. |
| 8 | handle primitives > extractHandle returns any non-empty string; validation happens in resolveHandles | `{session_id: "my-own-correlation-id"}` → `"my-own-correlation-id"`. | port |
| 9 | buildMintBackText > task minted: task-only block, no agent mention even when agent supplied | `{sessionId: T, sessionSource: "minted", hookMode: false, agentId: A, agentSource: "supplied"}` → `MINTED_BLOCK(T)`. | port |
| 10 | buildMintBackText > task minted without agent tracking: same task-only block | `{sessionId: T, sessionSource: "minted", hookMode: false}` → `MINTED_BLOCK(T)`. | port |
| 11 | buildMintBackText > task supplied -> null (nothing to announce, agent never mints) | Source `supplied` (agent supplied) → `null`. | port |
| 12 | buildMintBackText > hook mode never emits task instructions, even when hook-null minted | `hookMode: true`, source `minted` → `null`. | port |
| 13 | appendMintBack > prepends a text block as the first content element | `({content: [{type: "text", text: "hi"}]}, "BLOCK")` → length 2; `[0]` `{type: "text", text: "BLOCK"}`; `[1]` `{type: "text", text: "hi"}`. | port |
| 14 | appendMintBack > prepends on isError results too | `{isError: true, content: [{type: "text", text: "boom"}]}` → length 2; `[0]` is the block. | port |
| 15 | appendMintBack > leaves non-array content untouched | `{structuredContent: {a: 1}}` → same object; `null` → `null`. | adapt: rmcp `CallToolResult.content` is always a `Vec`; only the "no content" case maps. |
| 16 | appendMintBack > does not mutate the original result | The original `content` keeps length 1. | port |
| 17 | buildHandleTags > stamps sources, agent id, and protocol version | Source `supplied`, agent `A` supplied, version `"2026-07-28"` → `{agentcat_session_id_source: "supplied", agentcat_agent_id: A, agentcat_agent_id_source: "supplied", agentcat_protocol_version: "2026-07-28"}`. | port |
| 18 | buildHandleTags > omits agent and protocol keys when absent | `{sessionSource: "hook", hookMode: true}` → `{agentcat_session_id_source: "hook"}`. | port |
| 19 | buildHandleTags > clamps a >200-char agent_id to exactly 200 chars in the tag only | `"a".repeat(500)` → tag length 200, equal to `"a".repeat(200)`; `res.agentId` unchanged. | port |
| 20 | buildHandleTags > strips newlines from the agent_id tag | `"line1\nline2\r\nline3"` → tag `"line1 line2  line3"` (each `\r` and `\n` becomes one space); no `\n`, no `\r`; `res.agentId` unchanged. | port |
| 21 | buildHandleTags > passes a normal agent_id through unchanged | `A` → `A`. | port |
| 22 | resolveHandles — prompted mode > mints the task but never the agent when nothing supplied (agent tracking on) | `({enableAgentTracking: true}, "proj_1", req({}))` → `hookMode false`, source `minted`, `sessionId` `^ses_`, `agentId` and `agentSource` undefined. | port |
| 23 | resolveHandles — prompted mode > subagent flow: supplied task + omitted agent leaves the agent unresolved | `session_id: sid("parent")` → source `supplied`, `sessionId === sid("parent")`, agent undefined. | port |
| 24 | resolveHandles — prompted mode > uses supplied handles verbatim | `session_id: sid("anythinggoes")`, `agent_id: " opus-4.80-1m\|claude-code\|k3n9x "` → source `supplied`, id `sid("anythinggoes")`, `agentSource "supplied"`, `agentId "opus-4.80-1m\|claude-code\|k3n9x"` (trimmed). | port |
| 25 | resolveHandles — prompted mode > enableAgentTracking:false resolves no agent id at all | `agent_id: "agt_ignored"` → agent undefined. | port |
| 26 | resolveHandles — prompted mode > agent tracking is OFF by default — omitted option resolves no agent id | Options `{}`, `agent_id: "agt_ignored"` → source `minted`, agent undefined. | port |
| 27 | resolveHandles — prompted mode > rejects a malformed supplied session_id: sessionless, source invalid | `session_id: "nope"` → source `invalid`, `sessionId ""`. | port |
| 28 | resolveHandles — prompted mode > never stores the raw rejected value | `session_id: "sk_live_secret_token"` → `sessionId ""`; `JSON.stringify(res)` has no `sk_live_secret_token`. | port |
| 29 | resolveHandles — prompted mode > accepts a well-formed supplied session_id verbatim | `sid("parent")` → `supplied`, same id. | port |
| 30 | resolveHandles — prompted mode > foreign param: sessionless regardless of what the agent sent | 5th arg `sessionParamIsOurs = false`: `session_id: "customer-value"` → `foreign`, `""`; no value → `foreign`, `""`. | port |
| 31 | resolveHandles — prompted mode > hook mode wins over foreign: arguments are never read | `{resolveSessionId: () => "corr-7"}`, foreign → `hookMode true`, source `hook`, `sessionId ""`. | port |
| 32 | resolveHandles — prompted mode > still mints when nothing is supplied on our own param | `req({})` → `minted`, `^ses_`. | port |
| 33 | resolveHandles — prompted mode > start sentinel resolves exactly like omission: fresh mint, source minted | `session_id: "start"` → `minted`, `^ses_`; a second call gives a different id. | port |
| 34 | resolveHandles — prompted mode > start sentinel is case-insensitive and whitespace-tolerant: %j mints (`it.each` case `"START"`) | `session_id: "START"` → `minted`, `^ses_`. | port |
| 35 | … same `it.each`, case `"Start"` | → `minted`, `^ses_`. | port |
| 36 | … same `it.each`, case `"sTaRt"` | → `minted`, `^ses_`. | port |
| 37 | … same `it.each`, case `"  start  "` | → `minted`, `^ses_` (trimmed first). | port |
| 38 | … same `it.each`, case `" START\t"` | → `minted`, `^ses_`. | port |
| 39 | resolveHandles — prompted mode > near-sentinel values stay on the invalid path | `"restart"`, `"start now"`, `"starts"`, `"star t"` → each `invalid`, `""`. | port |
| 40 | resolveHandles — prompted mode > foreign param: start is the customer's value, never a sentinel | `session_id: "start"`, foreign → `foreign`, `""`. | port |
| 41 | resolveHandles — hook mode > returns a provisional resolution: empty id, source 'hook', hookMode on | `{resolveSessionId: async () => "customer-42"}` → `hookMode true`, `hook`, `""`. | port |
| 42 | resolveHandles — hook mode > ignores agent-supplied session_id — the hook wins | `{resolveSessionId: () => "customer-42"}`, `session_id: "ses_agent_sent"` → source `hook`, `sessionId !== "ses_agent_sent"`. | port |
| 43 | resolveHandles — hook mode > hook mode: supplied agent_id resolves, omitted stays unresolved | `{resolveSessionId: () => "c", enableAgentTracking: true}`: with `agent_id: "opus-4.80-1m\|claude-code\|k3n9x"` → `supplied`, that id; without → undefined. | port |
| 44 | invokeSessionHook > resolves the hook value and forwards request/extra verbatim | `request = req({session_id: "ses_agent_sent"})`, `extra = {requestInfo: {headers: {"x-correlation-id": "corr-1"}}}`, hook returns `extra.requestInfo.headers["x-correlation-id"] ?? null` → called once with the same `request` and `extra` objects; value `"corr-1"`. | adapt: the Rust hook receives the request params and the rmcp `RequestContext` (headers from `http::request::Parts`). |
| 45 | invokeSessionHook > resolves null when the hook returns nullish | Hook returns `null` → `null`; returns `undefined` → `null`. | adapt: the Rust hook returns `Option<String>`; only `None`. |
| 46 | invokeSessionHook > contains a synchronous throw | Hook throws `Error("db down")` → `null`. | adapt: a panic in the hook, caught with `catch_unwind` → `None`. |
| 47 | invokeSessionHook > contains a rejection | Hook rejects `Error("db down")` → `null`. | adapt: the hook future returns `Err` (or panics) → `None`. |
| 48 | sessionFromHookValue > derives deterministically from the hook value + project | `("customer-42", "proj_1")` twice → source `hook`, same id, `^ses_`, equal to `deriveSessionId("customer-42", "proj_1")`. | port |
| 49 | sessionFromHookValue > trims the value before deriving | `"  corr-1  "` → `deriveSessionId("corr-1", "proj_1")`. | port |
| 50 | sessionFromHookValue > mints a fresh id per null — silent mint, never 'supplied' | `null` twice → both `minted`, `^ses_`, different ids. | port |
| 51 | sessionFromHookValue > treats a whitespace-only value as null | `"   "` → `minted`. | port |
| 52 | buildStructuredMintBack > task minted + agent supplied: both ids, status issued | → `session_id T`, `agent_id A`, `status "issued"`; keys `["session_id", "agent_id", "status"]`. | port |
| 53 | buildStructuredMintBack > steady state (both supplied): ids plus status active | Source `supplied` → `status "active"`, both ids. | port |
| 54 | buildStructuredMintBack > agent tracking off: task only, status active | No agent → `session_id T`, no `agent_id`, `status "active"`; keys `["session_id", "status"]`. | port |
| 55 | buildStructuredMintBack > hook mode never exposes session_id or status; supplied agent is echoed | `hookMode true`, source `minted`, agent `A` → keys `["agent_id"]`. | port |
| 56 | buildStructuredMintBack > hook mode with agent tracking off: nothing echoable -> null | `{sessionSource: "hook", hookMode: true}` → `null`. | port |
| 57 | mirrorStructuredMintBack > adds the field as the FIRST key of structuredContent without mutating | Mint `{session_id: T, status: "issued"}`; `{content: [], structuredContent: {a: 1}}` → `structuredContent.mcp_session` equals the mint; `a === 1`; keys `["mcp_session", "a"]`; the original stays `{a: 1}`; result is a new object. | port |
| 58 | mirrorStructuredMintBack > returns the input untouched when structuredContent is absent or not a plain object | `{content: []}`, `{structuredContent: [1, 2]}`, `{structuredContent: "nope"}`, `null` → each returns the same value. | port |
| 59 | mirrorStructuredMintBack > never clobbers an existing key (customer data wins) | `{structuredContent: {mcp_session: "customer-owned"}}` → same object. | port |
| 60 | mirrorStructuredMintBack > applies to isError results that carry structuredContent | `{isError: true, structuredContent: {msg: "boom"}}` → mirror added; keys `["mcp_session", "msg"]`. | port |
| 61 | isValidSessionId > accepts an ID this SDK actually mints | `newSessionId()` and `deriveSessionId("anything", "proj_1")` → `true`. | port |
| 62 | isValidSessionId > accepts the test helper's IDs | `sid("parent")` → `true`. | port |
| 63 | isValidSessionId > rejects %s (`it.each` case `wrong prefix`) | `"task_2xF9kQm3rTvB8nLpYw7ZcHd4Ke"` → `false`. | port |
| 64 | … same `it.each`, case `no prefix` | `"2xF9kQm3rTvB8nLpYw7ZcHd4Ke1"` → `false`. | port |
| 65 | … same `it.each`, case `too short` | `"ses_abc"` → `false`. | port |
| 66 | … same `it.each`, case `too long` | `"ses_" + "a".repeat(28)` → `false`. | port |
| 67 | … same `it.each`, case `empty` | `""` → `false`. | port |
| 68 | … same `it.each`, case `prefix only` | `"ses_"` → `false`. | port |
| 69 | … same `it.each`, case `customer value` | `"my-app-session-42"` → `false`. | port |
| 70 | … same `it.each`, case `non-base62 body` | `"ses_" + "-".repeat(27)` → `false`. | port |
| 71 | … same `it.each`, case `inner whitespace` | `"ses_ " + "a".repeat(26)` → `false`. | port |
| 72 | invalid and foreign mint-back > invalid: corrects the agent without issuing a replacement | `{sessionId: "", sessionSource: "invalid", hookMode: false}` → text exactly `"[session_id unrecognized — see this tool's session_id parameter description]\nThe value sent was not issued by this server. Re-send the session_id issued earlier for this task; if none was issued yet, send start and one will be issued."`; no match for `ses_[0-9A-Za-z]{27}`. | port |
| 73 | invalid and foreign mint-back > invalid: offers an escape to an agent that was never issued one | Text contains `if none was issued yet, send start and one will be issued.` | port |
| 74 | invalid and foreign mint-back > invalid: structured mirror carries status unrecognized but no session_id | → `status "unrecognized"`, no `session_id`, keys `["status"]`. | port |
| 75 | invalid and foreign mint-back > invalid + agent supplied: agent_id rides along with the unrecognized status | Agent `"opus\|cc\|k3n9x"` → `status "unrecognized"`, no `session_id`, `agent_id "opus\|cc\|k3n9x"`, keys `["agent_id", "status"]`. | port |
| 76 | invalid and foreign mint-back > foreign: says nothing when agent_id is not in play either | `{sessionId: "", sessionSource: "foreign", hookMode: false}` → text `null`, structured `null`. | port |
| 77 | invalid and foreign mint-back > foreign: never echoes session state, which is the customer's | Foreign + agent → no `session_id`, no `status`. | port |
| 78 | invalid and foreign mint-back > foreign: still echoes agent_id, which AgentCat did inject | Foreign + agent `"opus\|cc\|k3n9x"` → `agent_id "opus\|cc\|k3n9x"`, keys `["agent_id"]`. | port |


Quirks:

- `deriveSessionId` comments promise an offset of "max 1 year", but `readUInt32BE` is at most 4,294,967,295 ms, which is below `31536000000`, so the modulo never applies. The offset covers about 49.7 days, and every derived id has a timestamp between 2024-01-01 and 2024-02-19. The golden vectors depend on this, so Rust must copy it exactly.
- `buildHandleTags` cuts with `.slice(0, 200)`, which counts UTF-16 units and can split a surrogate pair.
- `resolveHandles` is synchronous, but the tests `await` it.
- Row 42 uses `ses_agent_sent`, which is not a valid id anyway, so it does not prove that a valid id is ignored in hook mode.

---

### src/tests/identify.test.ts (12 tests)

Purpose: the `identify` hook runs on every tool call, in the background, and its result is stamped on that call's event. No `agentcat:identify` event exists.

Harness: `beforeEach` runs `resetTodos()` and `setupTestServerAndClient()`; `afterEach` runs `cleanup()`. Each test starts its own `EventCapture`, calls `track(server, "test-project", {enableTracing: true, identify, ...})`, sends `client.request({method: "tools/call", ...}, CallToolResultSchema)`, and waits 50 ms or calls `capture.flush()`. Random values use `randomUUID()`. `getServerTrackingData(server.server)` reads the stored config.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | Identify Feature > Basic Identification Test > should call identify on the tool call and stamp the identity onto that event | `identify(request, extra)` asserts both defined and returns `{userId: "user-<uuid>", userData: {name, email}}`. `add_todo` `{text: "Test todo for identification", context: "Adding a todo item for identification test"}` → `content[1].text` contains `Added todo`; identify was called; no `agentcat:identify` event; the tool-call event has `resourceName "add_todo"`, `identifyActorGivenId` = userId, `identifyActorData` = userData. `Object.keys(getServerTrackingData(server.server)).sort()` equals `["options", "projectId"]`. | port |
| 2 | … > Basic Identification Test > should call identify on every tool call and stamp every event, with no dedup | Calls `add_todo`, `list_todos`, `complete_todo` (`id: "1"`). The identify count is 1, 2, 3 right after each call (the hook starts on the request path). No identify events; 3 tool-call events, each with the userId and `identifyActorData` `{name: userName}`. | port |
| 3 | … > Basic Identification Test > should properly identify when calling tools added after track() | `track` with `enableToolCallContext: true`, then `server.tool("post_track_tool", "A tool added after tracking was enabled", {message: z.string().describe("A message to process")}, …)` returning `` Processed message: ${message} ``. Call with `{message: "Testing post-track identification", context: "Verifying identification works for dynamically added tools"}` → `content[1].text` contains `Processed message: Testing post-track identification`; the event has `userIntent "Verifying identification works for dynamically added tools"` and the identity. | adapt: rmcp tool routers are usually fixed at build time. Test a tool added to a dynamic router after wrapping; `Tracked<H>` intercepts every `call_tool` anyway. |
| 4 | … > User Data Persistence Across Tool Calls > should maintain user identification across multiple tool calls | `session_id: sid("identify_persistence")` on three calls → 3 tool-call events, all with that `sessionId`; each has the identity (`name`, `department: "Engineering"`, `customField`). | port |
| 5 | … > Null/Undefined Identity Handling > should handle when identify function returns null | `identify: async () => null` → `content[1].text` contains `Added todo`; no identify event; `identifyActorGivenId` undefined, `identifyActorName` undefined, `identifyActorData` `{}`. | port |
| 6 | … > Null/Undefined Identity Handling > should work without identify function (anonymous sessions) | No identify; `add_todo` and `list_todos` → 2 tool-call events, each `sessionId` defined and not `""`; no identify event. | port |
| 7 | … > Identity Data on Published Events > should populate actorGivenId, actorName, and actorData on the event | Identity `{userId, userName, userData: {name, role: "Developer", team: "Platform"}}` → tracking data keys `["options", "projectId"]`; the event has all three fields. | port |
| 8 | … > Identity Data on Published Events > should re-resolve identity per request so a changed identity lands on the next event | Identities `[{userId: first, userName: "First", userData: {seat: "a"}}, {userId: second, userName: "Second", userData: {seat: "b"}}]` returned in order; `add_todo` `"First call"`, `"Second call"` → event 0 has First/`{seat: "a"}`, event 1 has Second/`{seat: "b"}`. | port |
| 9 | … > Async Identity Resolution > should handle async operations in identify function | identify waits 100 ms, then returns `{userId, userData: {source: "async-lookup"}}`. Right after the call returns, the lookup is not done. After `flush()` it is done; the event has the identity and `duration < 95`. | port |
| 10 | … > Async Identity Resolution > should handle errors in identify function gracefully | identify throws `Error("Failed to identify user")` → `content[1].text` contains `Added todo`; no identify event; actor id and name undefined; `identifyActorData` `{}`. | port |
| 11 | … > Async Identity Resolution > should handle identify function that returns invalid data | identify returns `{invalidField: "invalid"}` → actor id and name undefined, `identifyActorData` `{}`. | N/A: the Rust hook returns a typed `UserIdentity`, so this shape cannot occur. |
| 12 | … > tools/list no longer identifies or publishes > should not run identify or publish an event for a tools/list request | A `tools/list` request → identify not called; 0 captured events. | port |

Quirks:

- Row 11: a truthy result without `userId` still overwrites the actor fields with `undefined` and logs nothing.
- Row 2 relies on the hook starting on the request path, although its result is awaited in the background.
- Row 9 asserts `duration < 95` ms, which is timing-sensitive.
- Rows 1 and 7 pin the internal storage shape (`["options", "projectId"]`).

---

### src/tests/ksuid.test.ts (25 tests)

Purpose: the vendored KSUID library (`src/thirdparty/ksuid/index.js`) that mints `ses_` and `evt_` ids.

Harness: direct calls on the Node `Buffer` API. KSUID epoch is `14e11` ms (2014-05-13T16:53:20Z). A KSUID is 20 bytes: a 4-byte big-endian seconds-since-epoch timestamp, then a 16-byte payload. The string form is base62 (`0-9A-Za-z`), left-padded with `0` to 27 chars. `withPrefix(p)` returns strings `` `${p}_${ksuid}` ``.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | KSUID > Basic functionality > should create a KSUID from randomSync | Instance of `KSUID`; `string` length 27; `raw` is a `Buffer` of length 20. | adapt: a Rust type over `[u8; 20]`. |
| 2 | KSUID > Basic functionality > should create a KSUID from random async | Same checks for `await KSUID.random()`. | N/A: the async variant exists only for Node's async `randomBytes`. |
| 3 | KSUID > Basic functionality > should create KSUIDs with different timestamps | `randomSync(t)` and `randomSync(t + 1000)` → `date.getTime() === Math.floor(t / 1000) * 1000` for each; first `compare` second `< 0`. | port |
| 4 | KSUID > fromParts > should create KSUID from timestamp and payload | `fromParts(now, Buffer.alloc(16, 42))` → date floored to the second; `payload` equals the input. | port |
| 5 | KSUID > fromParts > should throw error for invalid timestamp | `fromParts(100, payload)` throws; `fromParts(Date.now() + 1e15, payload)` throws. Valid range: integer ms in `[14e11, 1e3 * (2^32 - 1) + 14e11]`. | port |
| 6 | KSUID > fromParts > should throw error for invalid payload | Payload of 15 or 17 bytes throws (`Valid KSUID payloads are 16 bytes`). | adapt: with a `[u8; 16]` parameter this cannot compile; with `&[u8]` it returns `Err`. |
| 7 | KSUID > String encoding/parsing > should encode and parse KSUID strings correctly | `parse(original.string)` equals the original and has the same string. | port |
| 8 | KSUID > String encoding/parsing > should handle MIN and MAX string values | `parse("000000000000000000000000000")` and `parse("aWgEPTl1tmebfsQzFP4bxwgy80V")` round-trip; min `compare` max `< 0`. | port |
| 9 | KSUID > String encoding/parsing > should throw error for invalid string length | `parse("too_short")` and `parse("way_too_long_string_for_ksuid")` throw (`Valid encoded KSUIDs are 27 characters`). | port |
| 10 | KSUID > Comparison and equality > should compare KSUIDs correctly | `k1 < k2` (1 s later): `compare` `< 0`, reverse `> 0`, self `0`. | port |
| 11 | KSUID > Comparison and equality > should check equality correctly | `k1.equals(k1)`, `k1.equals(parse(k1.string))` true; a different random KSUID false. | port |
| 12 | KSUID > Buffer validation > should validate buffers correctly | `isValid(Buffer.alloc(20))` true; 19 and 21 bytes false; `null` false; `"not a buffer"` false. | adapt: the length checks port; `null` and string inputs are excluded by the Rust type. |
| 13 | KSUID > Buffer validation > should throw error for invalid buffer in constructor | `new KSUID(Buffer.alloc(19))` and `(21)` throw (`Valid KSUID buffers are 20 bytes`). | adapt: `TryFrom<&[u8]>` returns `Err`. |
| 14 | KSUID > Serialization > should serialize to JSON correctly | `JSON.parse(JSON.stringify(k)) === k.string`; `k.toJSON() === k.string`. | adapt: `serde::Serialize` as the 27-char string. |
| 15 | KSUID > Serialization > should have proper toString representation | `toString()` contains `KSUID` and the 27-char string (format `KSUID { <string> }`). | adapt: a `Debug` or `Display` format. |
| 16 | KSUID > Properties > should have correct timestamp property | `timestamp === Math.floor((t - 14e11) / 1000)`. | port |
| 17 | KSUID > Properties > should have correct date property | `date.getTime() === Math.floor(t / 1000) * 1000`. | port |
| 18 | KSUID > Properties > should have 16-byte payload | `payload.length === 16`, a `Buffer`. | port |
| 19 | KSUID > Edge cases > should handle KSUID epoch boundary | `randomSync(14e11)` → `timestamp 0`, `date.getTime() === 14e11`. | port |
| 20 | KSUID > Edge cases > should create unique KSUIDs | 100 `randomSync()` strings are all distinct. | port |
| 21 | KSUID > Prefix functionality > should create prefixed KSUIDs with randomSync | `withPrefix("event").randomSync()` is a string matching `^event_[0-9A-Za-z]{27}$`. | port |
| 22 | KSUID > Prefix functionality > should create prefixed KSUIDs with random async | `await withPrefix("user").random()` matches `^user_[0-9A-Za-z]{27}$`. | adapt: Rust needs only the sync form; keep the format check. |
| 23 | KSUID > Prefix functionality > should create prefixed KSUIDs with fromParts | `withPrefix("task").fromParts(now, Buffer.alloc(16, 42))` matches `^task_[0-9A-Za-z]{27}$`. | port |
| 24 | KSUID > Prefix functionality > should handle different prefixes | `event`, `user`, `task`, `session`, `req` → `^${prefix}_[0-9A-Za-z]{27}$`. | port |
| 25 | KSUID > Prefix functionality > should create unique prefixed KSUIDs | 10 `withPrefix("event").randomSync()` values are distinct and match `^event_[0-9A-Za-z]{27}$`. | port |

Quirks:

- `KSUID.parse` does not check the character set (audit 02 §7.11): any 27-char string decodes. A value above 2^160 throws `RangeError` (probe: `aWgEPTl1tmebfsQzFP4bxwgy80W` → `Fixed length of 20 is too small, expected at least 21`). `toUUIDv7` depends on this.
- `compare(other)` returns `0` when `other` is not a KSUID.
- The MAX value decodes to `2150-06-19T23:21:35.000Z`.

---

### src/tests/logging-target.test.ts (5 tests)

Purpose: `resolveLogTarget(loadModules)`, which picks the log destination.

Harness: direct calls with a fake loader `nodeModules(homedir)` that returns `{fs: {appendFileSync() {}}, os: {homedir}, path: {join: (...p) => p.join("/")}}`.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | resolveLogTarget > targets the home log file on a healthy Node runtime | `homedir = () => "/home/u"` → `kind "file"`, `path "/home/u/agentcat.log"`. | adapt: Rust resolves the home dir (for example `std::env::home_dir()` or `$HOME`) and uses `<home>/agentcat.log`. |
| 2 | resolveLogTarget > targets console only when the runtime lacks fs/os modules (edge isolates) | The loader throws `Error("no module")` → `kind "console"`. | N/A: edge isolates without `fs`; a Rust `wasm32` target needs its own decision. |
| 3 | resolveLogTarget > goes silent — never console — when homedir resolves falsy on Node | `homedir = () => undefined` → `kind "silent"`. Reason in the test: stdout is the JSON-RPC wire of stdio servers. | adapt: no home dir → no file logging and never stdout. |
| 4 | resolveLogTarget > goes silent — never console — when homedir throws on Node | `homedir` throws `Error("ENOENT: no such file or directory, uv_os_homedir")` → `kind "silent"`. | adapt: a failed home lookup → silent. |
| 5 | resolveLogTarget > goes silent when os.homedir is not implemented at all | `nodeModules(undefined)` (no `homedir` function) → `kind "silent"`. | N/A: a missing JS function. |

Quirks:

- `setupTestHooks()` (used elsewhere) deletes `./agentcat.log`, but the real target is `~/agentcat.log`. Test runs write to the real home log file and never clean it.

---

### src/tests/logging-version-prefix.test.ts (4 tests)

Purpose: every log line carries a version prefix.

Harness: the real `writeToLog`. `setDiagnosticsSink(fn)` collects whole log lines; `afterEach` resets it to `null`. Row 2 reads `~/agentcat.log` from the byte offset it had before the write. `PREFIXED_LINE = /^\[\d{4}-\d{2}-\d{2}T[0-9:.]+Z\] \[sdk=([^\s\]]+) node=([^\s\]]+)( mcp=([^\s\]]+))?( mcp2=([^\s\]]+))?\] (.*)$/`. Line format: `` [${new Date().toISOString()}] [sdk=<ver> node=<process.version> mcp=<v1 ver> mcp2=<v2 ver>] <message> ``. When neither MCP package resolves, the prefix ends `mcp=unknown`; on failure it is `[sdk=unknown node=unknown mcp=unknown]`.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | version-info prefix on every log line > prefixes sink entries with sdk, node, and mcp versions | `writeToLog("hello prefix")` → exactly 1 sink entry that matches `PREFIXED_LINE`; `sdk` = package version (`2.1.1-beta.1`); `node` = `process.version`; `mcp` matches `^1\.`; `mcp2` matches `^2\.`; message `hello prefix`. | adapt: Rust fields, for example `sdk=<CARGO_PKG_VERSION>`, the rustc version, and `rmcp=<version>` (rmcp has no runtime version query; record it at build time). |
| 2 | version-info prefix on every log line > writes the prefixed entry to the log file | `writeToLog("file prefix check")` → `~/agentcat.log` exists; the appended line containing `file prefix check` matches `PREFIXED_LINE` and contains `sdk=<package version>`. | adapt: same file, Rust prefix fields. |
| 3 | version-info prefix on every log line > caches the prefix so repeated logging stays consistent | Two writes (`first`, `second`) → the six prefix capture groups are equal. | port |
| 4 | version-info prefix on every log line > skips MCP package resolution on Cloudflare Workers | After `_resetVersionCacheForTest()` and `navigator.userAgent = "Cloudflare-Workers"` → `getRuntimeVersions()` has `sdk` = package version, `mcpV1 null`, `mcpV2 null`. | N/A: Cloudflare Workers module resolution. |

Quirks:

- The test comment says the v1 dev dependency is `~1.30.0`; `package.json` pins `~1.31.0`.
- The prefix is computed once per process. Tests need `_resetVersionCacheForTest()` to change it.

---

### src/tests/mcp-version-compatibility.test.ts (1 test)

Purpose: the installed TS MCP SDK `Server` passes the shape check.

Harness: `new Server({name: "test-server", version: "1.0.0"}, {capabilities: {}})` from `@modelcontextprotocol/sdk`.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | MCP Version Compatibility > should be compatible with currently installed MCP version | `isCompatibleServerType(server)` returns the same object (`toBe(server)`). | N/A: TS runtime shape detection; Rust uses the trait bound `H: ServerHandler`. |

Quirks: none.

---

### src/tests/posthog-exporter.test.ts (27 tests)

Purpose: the PostHog exporter payload: regular capture events, `$exception`, and `$ai_span`.

Harness: `globalThis.fetch` is replaced in `beforeEach` by `vi.fn()` that resolves `{ok: true, status: 200, text: () => Promise.resolve("ok")}` and restored in `afterEach`. Tests call `new PostHogExporter(config).export(makeEvent(overrides))` directly and parse `fetch.mock.calls[0][1].body`. `makeEvent()` = `{id: "evt_test123", sessionId: "ses_session45600000000000000000", projectId: "proj_1", eventType: "mcp:tools/call", timestamp: new Date("2025-01-15T10:00:00Z"), resourceName: "get_weather", serverName: "weather-server", serverVersion: "1.0.0", clientName: "claude-desktop", clientVersion: "2.0.0", duration: 150, isError: false, ...overrides}`. `expectUUIDv7(v)` checks `uuid.validate(v)` and `uuid.version(v) === 7`. Golden values (probe): `toUUIDv7("ses_session45600000000000000000") = 6c350afa-fa75-7081-b0c6-8ad3272daa6c`, `toUUIDv7("evt_test123") = 16abe8cf-1d05-7811-87f2-fd2f9686af87`. Rust: a local mock HTTP server instead of a global `fetch` patch.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | PostHogExporter > should send correct payload structure for regular events | Config `{type: "posthog", apiKey: "phc_test_key"}`. One fetch to `https://us.i.posthog.com/batch`, method `POST`, header `Content-Type: application/json`. Body `api_key "phc_test_key"`, `batch` length 1. `batch[0]`: `event "mcp_tool_call"`, `type "capture"`, `distinct_id "ses_session45600000000000000000"`, `timestamp "2025-01-15T10:00:00.000Z"`. Properties: `$session_id` UUIDv7, `tool_name "get_weather"`, `resource_name "get_weather"`, `duration_ms 150`, `server_name "weather-server"`, `server_version "1.0.0"`, `client_name "claude-desktop"`, `client_version "2.0.0"`, `project_id "proj_1"`, `is_error false`. | port |
| 2 | PostHogExporter > should use custom host when provided | `host: "https://eu.i.posthog.com"` → URL `https://eu.i.posthog.com/batch`. | port |
| 3 | PostHogExporter > should strip trailing slash from host | `host: "https://my-posthog.example.com/"` → `https://my-posthog.example.com/batch`. | port |
| 4 | PostHogExporter > should use identifyActorGivenId as distinct_id when available | `identifyActorGivenId: "user_abc123"` → `distinct_id "user_abc123"`. | port |
| 5 | PostHogExporter > should fall back to sessionId when identifyActorGivenId is not set | `identifyActorGivenId: undefined` → `distinct_id "ses_session45600000000000000000"`. | port |
| 6 | PostHogExporter > should omit $session_id entirely for sessionless events (empty sessionId) | `enableAITracing: true`; event `sessionId ""`, `isError true`, `error {message: "boom"}` → `batch` length 3; no entry has `properties.$session_id`; the `$ai_span` has no `$ai_session_id`; its `$ai_trace_id` is defined and UUIDv7. | port |
| 7 | PostHogExporter > should send $exception event alongside regular event when isError is true | `isError true`, `error {message: "Connection timeout", type: "TimeoutError", stack: "TimeoutError: Connection timeout\n    at fetch (/app/index.js:10:5)"}` → 1 fetch, `batch` length 2. `[0].event "mcp_tool_call"`, `is_error true`. `[1]`: `event "$exception"`, `distinct_id "ses_session45600000000000000000"`, `$exception_message "Connection timeout"`, `$exception_type "TimeoutError"`, `$exception_stacktrace` equal to the stack string, `$exception_source "backend"`, `$session_id` UUIDv7, `resource_name "get_weather"`, `tool_name "get_weather"`, `server_name "weather-server"`. | port |
| 8 | PostHogExporter > should not send $exception event when isError is false | `batch` length 1, `event "mcp_tool_call"`. | port |
| 9 | PostHogExporter > should not throw when fetch fails | fetch rejects `Error("Network error")` → `export()` resolves `undefined`. | port |
| 10 | PostHogExporter > should not throw when fetch returns non-ok response | `{ok: false, status: 400, text: "Bad Request"}` → resolves `undefined`. | port |
| 11 | PostHogExporter > should include $set person properties from identity data | `identifyActorGivenId "user_abc"`, `identifyActorName "Alice"`, `identifyActorData {email: "alice@example.com", plan: "pro"}` → `$set` equals `{name: "Alice", email: "alice@example.com", plan: "pro"}`. | port |
| 12 | PostHogExporter > should not include $set when no identity data is present | `$set` undefined. | port |
| 13 | PostHogExporter > should pass through parameters and response as-is (objects stay objects) | `parameters {city: "London", units: "celsius"}`, `response {temperature: 15, condition: "cloudy"}` → same objects in properties. | port |
| 14 | PostHogExporter > should pass through string parameters and response as-is | `"raw input"`, `"raw output"` → same strings. | port |
| 15 | PostHogExporter > should only set tool_name for tools/call events | `mcp:tools/call` + `get_weather` → `tool_name` and `resource_name` `"get_weather"`. `mcp:resources/read` + `my_resource` → `tool_name` undefined, `resource_name "my_resource"`. | port |
| 16 | PostHogExporter > should map event types to PostHog event names | `mcp:tools/call` → `mcp_tool_call`; `mcp:tools/list` → `mcp_tools_list`; `mcp:initialize` → `mcp_initialize`; `mcp:resources/read` → `mcp_resource_read`; `mcp:resources/list` → `mcp_resources_list`; `mcp:prompts/get` → `mcp_prompt_get`; `mcp:prompts/list` → `mcp_prompts_list`; `mcp:custom/type` → `mcp_custom_type` (fallback: remove `mcp:`, `/` → `_`, prefix `mcp_`). | port |
| 17 | PostHogExporter > should spread customer-defined tags directly into properties | `tags {env: "production", trace_id: "abc-123"}` → `properties.env "production"`, `properties.trace_id "abc-123"`. | port |
| 18 | PostHogExporter > should spread customer-defined properties directly into properties | `properties {device: "mobile", feature_flags: ["dark_mode"]}` → spread at top level. | port |
| 19 | PostHogExporter > should not include customer tag or property keys when not set on event | `source "agentcat"`; `env` and `device` undefined. | port |
| 20 | PostHogExporter > should include userIntent in properties | `userIntent "Check the weather in London"` → `user_intent` with the same text. | port |
| 21 | PostHogExporter > should emit $ai_span alongside regular event for tool calls when enableAITracing is true | `enableAITracing: true`, `duration 250`, `parameters {city: "London"}`, `response {temp: 15}` → `batch` length 2; `[0]` `mcp_tool_call`; `[1]`: `event "$ai_span"`, `type "capture"`, `distinct_id "ses_session45600000000000000000"`, `timestamp "2025-01-15T10:00:00.000Z"`, `$ai_session_id "agentcat_ses_session45600000000000000000"`, `$ai_trace_id` and `$ai_span_id` defined and different (golden: trace `6c350afa-…`, span `16abe8cf-…`), `$ai_span_name "get_weather"`, `$ai_latency` ≈ `0.25`, `$ai_is_error false`, `$ai_input_state {city: "London"}`, `$ai_output_state {temp: 15}`, `$session_id` UUIDv7, `source "agentcat"`, `server_name "weather-server"`, `client_name "claude-desktop"`. | port |
| 22 | PostHogExporter > should generate deterministic UUIDs for $ai_span trace and span IDs | Random `ses_` KSUID and two `evt_` KSUIDs A and B. Export A, B, A again. `$ai_session_id === "agentcat_" + sesId` for A and B; trace ids of A and B equal; span ids of A and B differ; span ids of A and the second A equal; trace ≠ span; both UUIDv7. | port |
| 23 | PostHogExporter > should NOT emit $ai_span when enableAITracing is false or unset | Unset, then `false` → each `batch` length 1, `mcp_tool_call`. | port |
| 24 | PostHogExporter > should NOT emit $ai_span for non-tool-call events even with enableAITracing | `mcp:initialize`, `mcp:tools/list`, `mcp:resources/read`, `mcp:resources/list`, `mcp:prompts/get`, `mcp:prompts/list` → no `$ai_span`. | port |
| 25 | PostHogExporter > should spread customer tags and properties directly on $ai_span (not namespaced) | `tags {env: "production", region: "us-east"}`, `properties {feature_flag: "new_ui", count: 42}` → on `$ai_span`: `env`, `region`, `feature_flag`, `count 42`; on the regular event: `env`, `feature_flag`, `count`. | port |
| 26 | PostHogExporter > should allow customer tags to override $ai_* defaults on $ai_span | `tags {$ai_trace_id: "custom-trace-uuid-from-customer", $ai_span_name: "custom_name"}` → `$ai_trace_id "custom-trace-uuid-from-customer"`, `$ai_span_name "custom_name"`. | port |
| 27 | PostHogExporter > should emit regular + $exception + $ai_span for error tool calls with enableAITracing | `isError true`, `error {message: "Tool execution failed", type: "ExecutionError"}` → `batch` length 3 in order `mcp_tool_call`, `$exception`, `$ai_span`; span `$ai_is_error true`, `$ai_error` equals `{message: "Tool execution failed", type: "ExecutionError"}`. | port |

Quirks:

- The SDK emits only `mcp:tools/call` and `agentcat:custom`. The fallback maps `agentcat:custom` to `mcp_agentcat:custom` (audit 02 §8 item 18). No test covers it.
- For a sessionless event, `$ai_trace_id` and `$ai_span_id` are equal (both `toUUIDv7(event.id)`). `distinct_id` falls back to `anonymous` without an actor or session. No test asserts either.
- `is_error` is set only when `isError !== undefined`. Real success events have no `isError`, so they have no `is_error` property; the fixture always sets `false`.
- Customer tags and properties can overwrite any key, including `source`, `$session_id`, and `$set`.
- The `$exception` event carries no tags, properties, or `project_id`.
- `fetch` has no timeout (audit 02 §8 item 2).

---

### src/tests/posthog-uuidv7.test.ts (12 tests)

Purpose: `toUUIDv7(prefixedId)`, which turns a session or event id into a deterministic UUIDv7 for PostHog.

Harness: direct calls; `realSessionId = KSUID.withPrefix("ses").randomSync()`; `uuid.validate` and `uuid.version`. Algorithm (`posthog.ts:13-59`): `hash = sha256(prefixedId)`; strip `^[a-z]+_`; KSUID path: `timestampMs = KSUID.parse(rest).date.getTime()`; fallback on any parse error: `timestampMs = hash.readUIntBE(10, 6)`. Bytes 0–5 = `timestampMs` (48-bit big-endian); byte 6 = `0x70 | (hash[0] & 0x0f)`; byte 7 = `hash[1]`; byte 8 = `0x80 | (hash[2] & 0x3f)`; bytes 9–15 = `hash[3..10]`; format `8-4-4-4-12` lowercase hex.

Golden vectors (probe with the real KSUID library; same as audit 02 §7.11):

| Input | `toUUIDv7` | Path |
| --- | --- | --- |
| `ses_2yXlSmKnrVgSWKGxWkkGlLaGu7v` (= `ses_` + `fromParts(2025-06-15T12:00:00Z, 16 × 0xab)`) | `01977375-f600-7355-b83a-0dea9024f86c` | KSUID |
| `ses_session45600000000000000000` | `6c350afa-fa75-7081-b0c6-8ad3272daa6c` | fallback |
| `evt_test123` | `16abe8cf-1d05-7811-87f2-fd2f9686af87` | fallback |
| `ses_invalid_ksuid_string` | `8a9bdc3e-a187-7187-bbe0-cee6b92a5751` | fallback |
| `my-customer-task-id` | `634902a7-d0c7-7220-91c6-0fa56fad8828` | fallback |
| `task-a` | `2084b1c4-8b52-77bb-bbed-5fd87cbc8c1b` | fallback |
| `task-b` | `8c37e6bc-5830-7977-9a3c-7aa32912c971` | fallback |
| `""` | `f4c8996f-b924-73b0-8442-98fc1c149afb` | fallback |

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | toUUIDv7 > should produce a valid UUIDv7 | `toUUIDv7(realSessionId)` is a valid UUID with version 7. | port |
| 2 | toUUIDv7 > should be deterministic — same input always produces same output | Three calls → equal. | port |
| 3 | toUUIDv7 > should produce different UUIDs for different session IDs | Two random `ses_` KSUIDs → different results. | port |
| 4 | toUUIDv7 > should embed the KSUID timestamp in the UUIDv7 | `KSUID.fromParts(Date("2025-06-15T12:00:00Z").getTime(), Buffer.alloc(16, 0xab))` with prefix `ses_` → the first 12 hex digits, read as ms, are within 1000 of the known time (exact: `0x01977375f600` = `1749988800000`). | port |
| 5 | toUUIDv7 > should set version bits to 7 (0111) | Hex digit at raw index 12 is `7`. | port |
| 6 | toUUIDv7 > should set variant bits to 10xx | Hex digit at raw index 16 is in `8..b`. | port |
| 7 | toUUIDv7 > should handle evt_ prefix | A random `evt_` KSUID → valid UUIDv7. | port |
| 8 | toUUIDv7 > should handle invalid KSUID gracefully with fallback timestamp | `"ses_invalid_ksuid_string"` → valid UUIDv7. | port |
| 9 | toUUIDv7 > should be deterministic for non-KSUID input (fallback path) | `"my-customer-task-id"` before and after a 5 ms wait → equal. | port |
| 10 | toUUIDv7 > should produce different UUIDs for different non-KSUID inputs | `"task-a"` ≠ `"task-b"`. | port |
| 11 | toUUIDv7 > should be deterministic for empty-string input | `""` before and after 5 ms → equal; valid UUIDv7. | port |
| 12 | toUUIDv7 > should produce timestamps that are before or equal to current time | For `realSessionId`, the embedded ms `<= Date.now()`. | port |

Quirks:

- The fallback timestamp is 6 hash bytes read as ms, so it is an arbitrary date, often far in the future. Row 12 covers only the KSUID path.
- The prefix regex `^[a-z]+_` also strips a customer prefix such as `my_` in `my_task`.
- `KSUID.parse` accepts any 27-char string (no charset check). A Rust port must copy the vendored base62 decode to match on garbage input.

---

### src/tests/publishCustomEvent.test.ts (15 tests)

Purpose: `publishCustomEvent(serverOrSessionId, projectId, eventData?)`: argument checks, session id choice, and the event it builds.

Harness:

- Mocks: `vi.mock` of `../modules/logging.js`, `../modules/internal.js`, `../modules/session.js`, `../modules/eventQueue.js`, `../modules/constants.js`, `../thirdparty/ksuid/index.js`.
- `publishEvent` (imported as `publishEventToQueue`) is a no-op spy. `eventQueue.add` is replaced with `vi.fn()`.
- `KSUID.withPrefix` returns `{random: async () => "evt_test123", randomSync: () => "ses_test12300000000000000000000"}`. The test reassigns `AGENTCAT_CUSTOM_EVENT_TYPE = "agentcat:custom"` on the mocked module.
- "with tracked server": the mock server is `{}`; `getServerTrackingData` returns `{projectId: "proj_tracked", sessionId: "ses_tracked12300000000000000000", options: {}}`; `projectId` argument `"proj_test123"`.
- Assertions use `toHaveBeenCalledWith(…, expect.objectContaining({...}))`.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | publishCustomEvent > with tracked server > should publish custom event with tracked server | `eventData {sessionId: "ses_srv000000000000000000000000", resourceName: "custom-action", parameters: {action: "test"}, message: "Testing custom event"}` → `getServerTrackingData(mockServer)` called; `publishEvent(mockServer, objectContaining({sessionId: "ses_srv000000000000000000000000", projectId: "proj_test123", eventType: "agentcat:custom", resourceName: "custom-action", parameters: {action: "test"}, userIntent: "Testing custom event"}))`; a log line contains `Published custom event`. | adapt: Rust API takes a tracked-server handle; `message` maps to `user_intent`. |
| 2 | … > with tracked server > should use eventData.sessionId as the session id, not any ambient server session | `{sessionId: "ses_srv000000000000000000000000"}` → event `sessionId` is that value (not the tracking data's `ses_tracked…`). | port |
| 3 | … > with tracked server > should publish with an empty session id and warn when no sessionId is provided | `{resourceName: "custom-action"}` → event `sessionId ""`, `projectId "proj_test123"`, `resourceName "custom-action"`; a log line contains `no sessionId provided` (full: `publishCustomEvent: no sessionId provided; event will be published without a session. Pass eventData.sessionId to attribute it to a task.`). | adapt: Rust uses `None` for "no session" and sends JSON `null`. |
| 4 | … > with tracked server > should handle error data correctly | `{isError: true, error: {message: "Test error", code: "ERR_001"}}` → event has `isError true` and `error` exactly `{message: "Test error", code: "ERR_001"}` (passed through, not `captureException`). | adapt: Rust takes the error as JSON (`serde_json::Value`) or a typed struct; decide whether to require `message`. |
| 5 | … > with tracked server > should throw error if server is not tracked | `getServerTrackingData` returns `undefined` → rejects with a message containing `Server is not tracked` (full: `Server is not tracked. Please call agentcat.track() first or provide a session ID string.`). | adapt: return `Err` with the same text. |
| 6 | … > with tracked server > should handle high-level server objects | `publishCustomEvent({server: mockServer}, projectId)` → `getServerTrackingData` called with `mockServer` (unwraps `.server`). | N/A: TS `McpServer` versus `.server` unwrapping. |
| 7 | … > with session ID string > should use the session ID string verbatim as the session ID | `("user-session-12345", "proj_test123", {resourceName: "custom-action", parameters: {action: "test"}})` → `eventQueue.add(objectContaining({sessionId: "user-session-12345", projectId: "proj_test123", eventType: "agentcat:custom", resourceName: "custom-action", parameters: {action: "test"}}))`. | adapt: Rust takes a session-id string variant. |
| 8 | … > with session ID string > should let eventData.sessionId take precedence over the string | `("ignored-string", "proj", {sessionId: "ses_wins00000000000000000000000"})` → `sessionId "ses_wins00000000000000000000000"`, `projectId "proj"`. | adapt: same precedence in the Rust API. |
| 9 | … > with session ID string > should handle all event data fields | `{resourceName: "full-test", parameters: {key: "value"}, response: {result: "success"}, message: "Complete test", duration: 1500, isError: false, error: null}` → event has the same values, with `userIntent "Complete test"`. | adapt: Rust field names (`message` → `user_intent`); `error: null` becomes `None`. |
| 10 | … > parameter validation > should throw error if projectId is not provided | `projectId` `""`, `null`, `undefined` → each rejects with `projectId is required` (full: `projectId is required for publishCustomEvent`). | adapt: Rust has only the empty-string case. |
| 11 | … > parameter validation > should throw error if first parameter is invalid | First argument `123`, `null`, `undefined` → rejects with `First parameter must be either an MCP server object or a session ID string`. | N/A: the Rust type system (an enum target) prevents these inputs. |
| 12 | … > event structure > should always use 'agentcat:custom' as event type | `("test-session", "proj_test")` → `eventType "agentcat:custom"`. | port |
| 13 | … > event structure > should include timestamp | `timestamp` is a `Date` between the time before and after the call. | port |
| 14 | … > event structure > should handle undefined event data gracefully | `eventData undefined` → `resourceName`, `parameters`, `response`, `userIntent`, `duration`, `isError`, `error` all `undefined`. | adapt: all `None`, keys omitted. |
| 15 | … > event structure > should handle empty event data object | `eventData {}` → same as row 14. | adapt: `Default::default()` data. |

Quirks:

- Tracked-server path: the event is built with the `projectId` argument, but the real `publishEvent` replaces it with the tracking data's `projectId` (audit 02 §8 item 8). These tests mock `publishEvent`, so they never see the replacement.
- The mocked tracking data has a `sessionId` field that the SDK no longer stores.
- `error` is passed through without `captureException`, so a custom-event error has no `platform` or `frames` and may lack `message`.
- `validateTags` can return `null`, so `event.tags` can be `null`.
- The session-string path skips the `redactEvent` hook and every tracked option. Custom events never get string redaction (audit 02 §8 item 7).

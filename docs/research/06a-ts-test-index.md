# TS test index, part A (06a)

Source: `/home/ubuntu/agentcat-typescript-sdk`, commit `9b56e9e`, package `agentcat` 2.1.1-beta.1.
Scope: the test files `api-base-url` through `error-capture` named in the task, plus the
`e2e-http/` folder and its helpers. All paths are relative to `src/tests/`.

Each row says what the test asserts and whether the same test applies to a Rust `rmcp` server:

- `port`: the same behavior applies to a Rust rmcp server.
- `adapt: <how>`: the behavior applies, but the Rust shape differs.
- `N/A: <reason>`: the test covers Node/JS-only behavior.

A `×N` in the `#` column marks an `it.each` row that expands to N test cases.

## Summary

### Counts

| Measure | Value |
| --- | --- |
| Test files indexed | 33 (plus 3 helper modules in `e2e-http/`) |
| `it`/`test` declarations | 250 |
| Test cases after `it.each` expansion | 363 (1 skipped: `error-capture.test.ts` "should detect in_app frames correctly") |
| `port` | 222 cases |
| `adapt` | 68 cases |
| `N/A` | 73 cases |

Cases per file: `api-base-url` 11, `basic-server` 8, `client-identity` 17, `compatibility` 36,
`concurrency` 1, `constants` 11, `context-parameters-edge-cases` 12, `context-parameters` 10,
`context-preservation` 2, `custom-context-description` 8, `debug-tool-schema` 2, `detect-shapes` 2,
`diagnostics-attributes` 5, `diagnostics-auth` 3, `diagnostics-export` 3,
`diagnostics-integration` 2, `diagnostics-no-payload` 3, `diagnostics-optout` 17,
`diagnostics-record` 5, `diagnostics-sink` 3, `diagnostics-test-env` 2,
`e2e-http/header-capture` 2, `e2e-http/sse-smoke-v1` 1, `e2e-http/sse-smoke-v2` 1,
`e2e-http/v1-matrix` 42, `e2e-http/v2-matrix` 63, `e2e-sanitization` 4, `e2e-truncation` 4,
`edge-runtime-compatibility` 23, `engine-callwrap` 25, `engine-innertap` 11, `engine-listwrap` 12,
`error-capture` 12.

The N/A cases are mostly TS MCP SDK shape detection (`compatibility`, `detect-shapes`), JS
registry and monkey-patch seams (`engine-innertap`), Node `process` and edge runtimes
(`edge-runtime-compatibility`), and JS values that cannot exist in typed Rust (null content
entries, boolean or cyclic or BigInt schemas).

### How the TS suite runs

- `vitest.config.ts` defines the projects. Project `v1` runs `src/tests/**/*.test.ts` except
  `v2/`, `e2e-http/` and `workers/`. Project `e2e-http` is opt-in (`pnpm test:e2e:http`) and
  has `testTimeout: 30_000`.
- All projects run in one fork, one test at a time (`pool: "forks"`, `singleFork: true`), with
  `setupFiles: ["./src/tests/setup.ts"]`. Module singletons (the event queue, diagnostics
  state, tracking `WeakMap`s) are shared across tests in a file.
- A Rust suite must serialize the tests that change process environment variables or global
  state (for example with the `serial_test` crate).

### Shared helpers and fixtures a Rust suite must replicate

**1. Global setup (`setup.ts`).** Sets `process.env.DISABLE_DIAGNOSTICS = "true"` before any test
so no run ships diagnostics to the live collector. Diagnostics tests opt back in with
`DISABLE_DIAGNOSTICS=false` and a mocked `fetch`. Rust: set the same variable for the whole
test binary, or construct the SDK with diagnostics off by default in tests.

**2. `sid(label)` (`test-utils.ts`).** Builds a valid session ID that still reads as its label:
remove every character outside `[0-9A-Za-z]`, append 27 `"0"`s, keep the first 27 characters,
and prefix `ses_`. The result always matches `^ses_[0-9A-Za-z]{27}$` (31 characters).
Values used in this index:

| Call | Result |
| --- | --- |
| `sid("session_A")` | `ses_sessionA0000000000000000000` |
| `sid("ctxoff")` | `ses_ctxoff000000000000000000000` |
| `sid("strict1")` | `ses_strict100000000000000000000` |
| `sid("strict")` | `ses_strict000000000000000000000` |
| `sid("raw1")` | `ses_raw100000000000000000000000` |
| `sid("loose")` | `ses_loose0000000000000000000000` |
| `sid("redact")` | `ses_redact000000000000000000000` |
| `sid("e2e")` | `ses_e2e000000000000000000000000` |
| `sid("supplied")` | `ses_supplied0000000000000000000` |

**3. `EventCapture` (`test-utils.ts`).** The main way tests read published events.

- `start()` imports the `eventQueue` singleton once and replaces two methods:
  `add(event)` pushes the event object onto an internal array and then calls the real `add()`;
  `sendEvent()` becomes a no-op, so no HTTP request and no telemetry export happens.
- The full queue pipeline still runs: stage 0 resolves deferred hook results (identity, tags,
  properties, hook-mode session), then `redactEvent`, `redactSensitiveInformation`,
  sanitization, truncation, and the `evt_` ID. The pipeline mutates the captured object in
  place, so assertions made after a wait see the processed values.
- Capture happens at `add()` time. An event that `redactEvent` later drops is still in the
  array (in its unprocessed state).
- `stop()` restores both methods. `getEvents()` returns a copy in `add()` order.
- `flush(timeoutMs = 5000)` polls every 5 ms until every captured event has an `id` (the last
  pipeline step), and throws `"EventCapture.flush timed out waiting for captured events to finish processing"` on timeout.
- `clear()`, `findEventByType(eventType)` (first match), `findEventsByResourceName(name)`.
- The captured object is the internal camelCase `Event` (`sessionId`, `resourceName`,
  `parameters: {request, extra}`, `userIntent`, `tags`, `identifyActorGivenId`, ...), not the
  wire `PublishEventRequest`.

Rust: put a test seam where the publisher calls the API. A capturing publisher (or a fake
`agentcat-api` HTTP server) records the processed event and keeps enqueue order. Provide
`events()`, `flush(timeout)`, `find_by_type`, `find_by_resource_name`, and `clear()`.

**4. `setupTestHooks()` / `cleanupLogFile()` / `LOG_FILE = "agentcat.log"`.** Deletes
`./agentcat.log` (in the working directory) before and after each test. The SDK writes to
`~/agentcat.log`, so this cleanup does nothing useful. Rust does not need it.

**5. Waits.** Many integration tests sleep a fixed 50 ms or 100 ms before reading events.
Rust: replace fixed sleeps with `flush()`.

**6. Todo fixture (`test-utils/client-server-factory.ts`).**

- Module-level state: `todos = []`, `nextId = 1`; `resetTodos()` resets both.
- `setupTestServerAndClient()` builds a v1 `McpServer({name: "test server", version: "1.0"})` with:
  - `add_todo` (`server.tool`, description `"Add a new todo item"`, `{text: string}` described
    `"The text of the todo item"`): pushes `{id: String(nextId++), text, completed: false}` and
    returns text `Added todo: "<text>" with ID <id>`.
  - `list_todos` (`server.tool`, `"List all todo items"`, no args): returns lines
    `<id>: <text> ✓` or `<id>: <text> ○` joined with `"\n"`, or `"No todos found"`.
  - `complete_todo` (`registerTool`, `"Mark a todo item as completed"`, `{id: string}` described
    `"The ID of the todo to complete"`): throws `Error("Todo with ID <id> not found")`, else
    returns `Completed todo: "<text>"`.
  - `get_stats` (`registerTool`, `"Get todo stats"`, no input, `outputSchema {count: number}`
    described `"Number of todos"`): returns text `<n> todos` and `structuredContent {count: n}`.
- Client: `Client({name: "test client", version: "1.0"}, {capabilities: {sampling: {}}})` with a
  `sampling/createMessage` handler that returns
  `{model: "test-model", role: "assistant", content: {type: "text", text: "This is a test response"}}`.
- Transport: `InMemoryTransport.createLinkedPair()`; both sides connect in parallel.
  `cleanup()` closes both transports.
- Returns `{server, client, clientTransport, serverTransport, cleanup}`.

Rust: the same four tools on an rmcp handler, served over `tokio::io::duplex` to an rmcp client.

**7. Synthetic engine seams (`engine-callwrap`, `engine-listwrap`, `client-identity`).**
`fakeServer(callHandler)`, `fakeListServer(listHandler)`, `trackFake(server, overrides)`,
`data(overrides)`, `echoTool()`, and `serverWith(clientVersion)` are described in those sections.
Rust: a fake inner `ServerHandler` whose `call_tool`/`list_tools` run test closures, wrapped by
the SDK's tracking wrapper and driven through a real rmcp client over a duplex pipe (an rmcp
`RequestContext` needs a live `Peer`).

**8. Log capture.** Unit tests replace `writeToLog` with `vi.fn()` (`vi.mock("../modules/logging")`)
or register a sink with `setDiagnosticsSink(fn)`, then match substrings of the log line. Every
log line has the form `[<ISO time>] [sdk=<v> node=<v> mcp=<v>( mcp2=<v>)] <message>`. Rust:
a test log sink that records every line.

**9. Diagnostics fixtures.** `globalThis.fetch = vi.fn().mockResolvedValue({ok: true, status: 200})`;
`_resetDiagnosticsForTest()` clears the one-time init latch, buffer, static attributes, flush
timer and sink; `_getStaticAttributesForTest()` and `_buildRecordForTest(entry)` expose internals;
`asMap(attrs)` turns `[{key, value: {stringValue}}]` into a map. `vi.stubEnv`/`vi.unstubAllEnvs`
change environment variables. Rust: a fake OTLP HTTP endpoint and serialized env-var tests.

**10. `e2e-http` helpers** (`lanes.ts`, `scenario-types.ts`, `toolkit.ts`, and
`mintBackOf`/`handleFrom` from `v2/harness.ts`). See their sections. Key values:
`E2E_PROJECT_ID = "proj_test"`; default client `{name: "agentcat-e2e-client", version: "1.0.0"}`;
`waitForEvents(capture, count, timeout = 2000)` polls every 20 ms; `settle(ms = 100)`.

**11. Literal strings the assertions depend on.** The Rust SDK must copy these byte for byte.

`DEFAULT_CONTEXT_PARAMETER_DESCRIPTION`:

```text
Explain why you are calling this tool and how it fits into the user's overall goal. This parameter is used for analytics and user intent tracking. YOU MUST provide 15-25 words (count carefully). NEVER use first person ('I', 'we', 'you') - maintain third-person perspective. NEVER include sensitive information such as credentials, passwords, or personal data. Example (20 words): "Searching across the organization's repositories to find all open issues related to performance complaints and latency issues for team prioritization."
```

`AGENT_ID_PARAM_DESCRIPTION` (only partly pinned by tests):

```text
Agent identity handle, required on every call including your first. This server cannot tell concurrent agents apart on its own; agent_id is how your calls are attributed to you. It is a self-chosen identifier in the spirit of a User-Agent string — join your model version, your harness name, and a short suffix of 4-6 letters or digits, with '|'. Example: opus-4.80-1m|claude-code|k3n9x. Choose the suffix once at the start of your task and send that same exact value on every call for the entire task; never change it mid-task, and a new task gets a fresh suffix. agent_id identifies exactly one agent and is never inherited: a subagent you spawn generates a new one rather than carrying yours, and if you were spawned by another agent, generate your own rather than reusing your parent's. A call without agent_id cannot be attributed to you.
```

Mint-back text blocks (prepended as `content[0]`, lines joined with `"\n"`):

```text
[session_id issued — see this tool's session_id parameter description]
session_id: <ses_ id>
This is the first-call issuance described in this tool's session_id parameter description.
```

```text
[session_id unrecognized — see this tool's session_id parameter description]
The value sent was not issued by this server. Re-send the session_id issued earlier for this task; if none was issued yet, send start and one will be issued.
```

`get_more_tools` descriptor and answer:

- description: `Check for additional tools whenever your task might benefit from specialized capabilities - even if existing tools could work as a fallback.`
- `inputSchema`: `{type: "object", properties: {context: {type: "string", description: "A description of your goal and what kind of tool would help accomplish it."}}, required: ["context"]}`
- `annotations`: `{title: "Get More Tools", readOnlyHint: true, destructiveHint: false, idempotentHint: true, openWorldHint: false}`
- answer text: `Unfortunately, we have shown you the full tool list. We have noted your feedback and will work to improve the tool list in the future.`

Wire keys: tags `agentcat_session_id_source`, `agentcat_agent_id`, `agentcat_agent_id_source`,
`agentcat_protocol_version`, `agentcat_mrtr`; `_meta` keys `io.modelcontextprotocol/clientInfo`
and `io.modelcontextprotocol/protocolVersion`; structured mirror key `mcp_session`; event types
`mcp:tools/call` and `agentcat:custom`. The other pinned strings appear in the
`constants.test.ts` rows.

---

### src/tests/api-base-url.test.ts (11 tests)

Purpose: the `apiBaseUrl` option, the `AGENTCAT_API_URL`/`MCPCAT_API_URL` fallbacks, and `EventQueue.configure()`.

Harness: `vi.mock("agentcat-api")` replaces the generated `Configuration` and `EventsApi`
constructors (re-implemented with `function` so `new` works; `EventsApi` returns
`{publishEvent: vi.fn().mockResolvedValue({})}`). `logging`, `ksuid`, `compatibility`,
`session`, `internal`, `engine/index` and `telemetry` are also mocked: `isCompatibleServerType`
returns the mock server, `getServerTrackingData` returns `null`, `setServerTrackingData` does
nothing. The mock server is `{_requestHandlers: new Map(), _serverInfo: {name: "test-server", version: "1.0.0"}, getClientVersion: () => undefined, setRequestHandler: vi.fn()}`.
`vi.spyOn(eventQueue, "configure")` records the calls. Both env vars are deleted before each
test and restored after. No event is published.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | AgentCatOptions apiBaseUrl > should accept apiBaseUrl as an optional string property | `{apiBaseUrl: "https://custom.example.com"}` keeps `"https://custom.example.com"` (type-level test). | port (options field `api_base_url: Option<String>`) |
| 2 | AgentCatOptions apiBaseUrl > should be undefined when not set | `{}` gives `apiBaseUrl === undefined`. | port (default `None`) |
| 3 | EventQueue.configure() > should reconfigure the apiClient with the given base URL | After `mockClear()`, `eventQueue.configure("https://custom.example.com")` calls `new Configuration({basePath: "https://custom.example.com"})`. The queue's constructor default is `basePath: "https://api.agentcat.com"`. | adapt: assert the `base_path` of the `agentcat-api` `Configuration` the publisher builds; Rust builds the client once from the resolved URL instead of a later `configure()`. |
| 4 | track() URL resolution > should call configure() when apiBaseUrl option is provided | `track(mockServer, "proj_test123", {apiBaseUrl: "https://custom-api.example.com"})` calls `configure("https://custom-api.example.com")`. | port |
| 5 | track() URL resolution > should call configure() with MCPCAT_API_URL env var when no option is set | `MCPCAT_API_URL="https://env-api.example.com"`, options `{}`: `configure("https://env-api.example.com")`. | port (decide whether a new Rust SDK keeps the legacy variable) |
| 6 | track() URL resolution > should call configure() with AGENTCAT_API_URL env var when no option is set | `AGENTCAT_API_URL="https://agentcat-env.example.com"`: `configure("https://agentcat-env.example.com")`. | port |
| 7 | track() URL resolution > should prioritize AGENTCAT_API_URL over legacy MCPCAT_API_URL | Both set (`https://agentcat-env.example.com`, `https://legacy-env.example.com`): the `AGENTCAT_API_URL` value wins. | port |
| 8 | track() URL resolution > should fall back to legacy MCPCAT_API_URL when AGENTCAT_API_URL is unset | Only `MCPCAT_API_URL="https://legacy-env.example.com"`: `configure("https://legacy-env.example.com")`. | port |
| 9 | track() URL resolution > should prioritize apiBaseUrl option over MCPCAT_API_URL env var | Env `https://env-api.example.com`, option `https://option-api.example.com`: the option wins and `configure` is called exactly 1 time. | port |
| 10 | track() URL resolution > should prioritize apiBaseUrl option over AGENTCAT_API_URL env var | Env `https://agentcat-env.example.com`, option `https://option-api.example.com`: the option wins, 1 call. | port |
| 11 | track() URL resolution > should not call configure() when neither option nor env var is set | No option, no env: `configure` is never called (the queue keeps `https://api.agentcat.com`). | adapt: assert that the resolved base URL is `https://api.agentcat.com`. |

Quirks:

- Resolution uses `||` (`options.apiBaseUrl || AGENTCAT_API_URL || MCPCAT_API_URL`), so an empty string counts as unset. No test covers this.
- `track()` calls `eventQueue.configure()` on a process-wide singleton before its "already tracked" check. The last `track()` call sets the URL for every tracked server (audit 01 §15 #4). No test covers a second `track()`.

### src/tests/basic-server.test.ts (8 tests)

Purpose: smoke tests for a v1 `McpServer` with and without tracking, including tools registered before and after `track()`.

Harness: the todo fixture (`setupTestServerAndClient()`, `resetTodos()`), or a hand-built
`McpServer({name: "test calculator server", version: "1.0.0"})` plus `Client({name: "test client", version: "1.0"}, {capabilities: {sampling: {}}})`
over `InMemoryTransport.createLinkedPair()`. `track` is imported dynamically. Calls use
`client.request({method, params}, ListToolsResultSchema | CallToolResultSchema)`. Row 7 uses
`EventCapture` and a 50 ms wait.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | Basic Server Test > should be able to call tools without tracking | Untracked: `tools/list` returns more than 0 tools; `add_todo {text: "Test todo"}` returns `content[0].text` containing `"Added todo"`. | port (the fixture works with no wrapper) |
| 2 | Basic Server Test > should be able to access server info from regular Server instance | `server.server._serverInfo` is `{name: "test server", version: "1.0"}`. | adapt: read `get_info().server_info` (name, version); these become `serverName`/`serverVersion` on events. |
| 3 | Basic Server Test > should be able to access server info with McpServer if available | Dynamic import of `McpServer` (returns early with a log line below SDK v1.3.0); `new McpServer({name: "test-mcp-server-info", version: "2.0.0"}).server._serverInfo` has that name and version. | N/A: probes TS MCP SDK internals and versions. |
| 4 | Basic Server Test > should verify that server info is used by isCompatibleServerType | `isCompatibleServerType(server)` returns the same object; `_serverInfo.name` is `"test server"`. | N/A: runtime shape validation; Rust uses the `ServerHandler` trait bound. |
| 5 | Basic Server Test > should properly trace tools added after track() is called | `track(server, "test-project", {enableToolCallContext: true, enableTracing: true})`, then `server.tool("new_tool_after_track", "A tool added after track() was called", {data: z.string().optional()})`. Listing shows the tool with `properties.context.type === "string"` and `properties.data` kept. Call `{context: "Testing new tool after track", data: "test data"}`: `content[1].text` contains `"test data"` (`content[0]` is the mint-back block). | adapt: Rust injects on every `tools/list`, so a tool added to a stored `ToolRouter` after wrapping must also get `context`; test with a router changed at runtime. |
| 6 | Basic Server Test > should handle tools with shorthand Zod schema syntax (reproduces context injection issue) | After `track()`, add `calculator_add` with `{a: z.number(), b: z.number()}`. Listing has `context` (type `"string"`), `a`, `b`. Call `{context: "Testing calculator with shorthand syntax", a: 5, b: 3}`: `content[1].text === "8"`. | adapt: same as row 5, with a `schemars`-generated schema. |
| 7 | Basic Server Test > should publish events only once (no duplicates) | `track(server, "test-dedup-project", {enableToolCallContext: true, enableTracing: true})`. Calls: `add_todo {context: "test context 1", text: "First todo with unique ID: <ts>"}`, `add_todo {context: "test context 2", text: "Second todo with unique ID: <ts+1>"}`, `list_todos {context: "test context 3"}`. After 50 ms: no two `mcp:tools/call` events share the key `<eventType>_<resourceName>_<JSON(parameters)>`; exactly 2 `add_todo` events and 1 `list_todos` event; the 2 `add_todo` events have 2 distinct `parameters.request.params.arguments.text`. | port |
| 8 | Basic Server Test > should handle tools defined BEFORE track() with shorthand Zod schema | Tools `add` (`{a, b}`) and `calculate` (`operation: z.enum(["add", "subtract", "multiply", "divide"])`, `a`, `b`) registered and connected before `track(server, "test-project", ...)`. Listing: both have `context` (type `"string"`), plus `a`, `b`, `operation`. `add {context: "Testing add with context", a: 5, b: 3}` gives `content[1].text === "8"`; `calculate {context: "Testing calculate with context", operation: "multiply", a: 4, b: 7}` gives `content[1].text === "28"`. | port |

Quirks:

- Rows 5, 6 and 8 read `content[1]` because the SDK prepends the mint-back block at `content[0]` whenever a call has no `session_id`. A Rust port must keep the same offset.

### src/tests/client-identity.test.ts (17 tests)

Purpose: per-request client identity, protocol version, event session info, and the `identify` resolver.

Harness: pure unit tests, no transport, no events. `serverWith(clientVersion)` returns
`{getClientVersion: () => clientVersion, _serverInfo: {name: "srv", version: "9.9"}, setRequestHandler() {}, _requestHandlers: new Map()}`.
The envelope block uses a server whose `getClientVersion()` returns `{name: "from-handshake", version: "9.9.9"}`.
`dataWith(identify)` returns `{projectId: "p", options: {identify}}`. The ladder in the source:
`extra.mcpReq.envelope["io.modelcontextprotocol/clientInfo"]` → `request.params._meta[...]` →
`server.getClientVersion()`. A candidate is accepted when `name` or `version` is a string; a
non-string field becomes `undefined`.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | getClientInfoForRequest > prefers per-request _meta clientInfo (modern client) | `_meta["io.modelcontextprotocol/clientInfo"] = {name: "claude", version: "3.0"}` beats handshake `{name: "legacy", version: "1"}`: result `{name: "claude", version: "3.0"}`. | port (read client info from `context.meta` first) |
| 2 | getClientInfoForRequest > falls back to getClientVersion (legacy initialize capture) | No `_meta`: `{name: "legacy", version: "1"}`. | adapt: fall back to `context.peer.peer_info().client_info`. |
| 3 | getClientInfoForRequest > both missing -> undefined (client declined to identify) | Result `undefined`. | adapt: also treat rmcp's synthesized `Implementation::default()` (name `"rmcp"`, stateless HTTP) as missing. |
| 4 | getClientInfoForRequest > ignores malformed _meta clientInfo | `_meta clientInfo = "not-an-object"`, no handshake: `undefined`. | port (`_meta` is untyped JSON in rmcp too) |
| 5 | getClientInfoForRequest > drops non-string fields from partially malformed _meta clientInfo | `{name: "x", version: 42}` gives `{name: "x", version: undefined}`. | port |
| 6 | getProtocolVersion > reads _meta protocolVersion | `_meta["io.modelcontextprotocol/protocolVersion"] = "2026-07-28"` gives `"2026-07-28"`. | port |
| 7 | getProtocolVersion > absent -> undefined | `{params: {}}` and `undefined` both give `undefined`. | port |
| 8 | buildSessionInfo > stamps server, client, and identity without touching tracking data | Identity `{userId: "u1", userName: "Naseem", userData: {plan: "pro"}}`, client `{name: "claude", version: "3.0"}`: `serverName "srv"`, `clientName "claude"`, `identifyActorGivenId "u1"`, `identifyActorName "Naseem"`, `identifyActorData {plan: "pro"}`, `sdkLanguage "TypeScript"`. | adapt: `sdkLanguage` takes the Rust value (for example `"Rust"`); the other fields port. |
| 9 | buildSessionInfo > null identity yields anonymous info | `identifyActorGivenId` and `clientName` are `undefined` (the source also sets `identifyActorData: {}`). | port |
| 10 | envelope-first identity ladder (v2 ctx.mcpReq.envelope) > prefers ctx.mcpReq.envelope.clientInfo over _meta and handshake | Envelope `{name: "env-client", version: "2.0"}` beats `_meta {name: "meta-client"}` and the handshake: `{name: "env-client", version: "2.0"}`. | adapt: rmcp lifts `_meta` into `context.meta`, so envelope and `_meta` are one source; test that `context.meta` beats `peer_info()`. |
| 11 | envelope-first identity ladder (v2 ctx.mcpReq.envelope) > falls back to _meta (v1 passthrough), then getClientVersion() | `_meta {name: "meta-client"}` gives `{name: "meta-client", version: undefined}`; no `_meta` gives `{name: "from-handshake", version: "9.9.9"}`. | adapt: the same two-step ladder (`context.meta`, then `peer_info()`). |
| 12 | envelope-first identity ladder (v2 ctx.mcpReq.envelope) > rejects non-string envelope fields (field-level narrowing) | Envelope `{name: 42}` is ignored; the handshake wins: `{name: "from-handshake", version: "9.9.9"}`. | port |
| 13 | envelope-first identity ladder (v2 ctx.mcpReq.envelope) > getProtocolVersion prefers the envelope, then _meta | Envelope `"2026-07-28"` beats `_meta "2025-11-25"`; with extra `{}` the result is `"2025-11-25"`; with neither, `undefined`. | adapt: read only `context.meta`. Do not use rmcp's `RequestContext::protocol_version()`, which falls back to the handshake and would add a protocol tag on legacy lanes (the e2e matrices assert there is none). |
| 14 | resolveIdentity > returns the identify result | `async () => ({userId: "u1"})` gives `{userId: "u1"}`. | port |
| 15 | resolveIdentity > no identify configured -> null | Result `null`. | port |
| 16 | resolveIdentity > identify throwing -> null, never throws | A hook that throws `Error("boom")` synchronously gives `null`. | adapt: map a hook `Err` and a hook panic (`catch_unwind`) to `None`. |
| 17 | resolveIdentity > runs on every call — no caching between calls | A counter hook gives `"u1"`, then `"u2"`. | port |

Quirks:

- `getProtocolVersion` never falls back to the handshake protocol version, but rmcp's convenience accessor does. Row 13 and the e2e "no protocol tag on legacy lanes" checks depend on the TS rule.
- A half-valid client info object is kept (row 5), so an event can carry a client name with no version.

### src/tests/compatibility.test.ts (36 tests)

Purpose: runtime validation of TS MCP server object shapes, the error-to-string helper used in log lines, and the compatibility log line.

Harness: `vi.mock("../modules/logging", () => ({writeToLog: vi.fn()}))`; `vi.clearAllMocks()`
before each test. Fake servers are object literals with `setRequestHandler: vi.fn()`,
`_requestHandlers: new Map(...)`, `getClientVersion: vi.fn()`, `_serverInfo: {name: "TestServer"}`.
The last block imports the real v1 `McpServer`. `SUPPORT_MATRIX_SUFFIX` is
`AgentCat supports MCP TypeScript SDK v1.11+ (@modelcontextprotocol/sdk) and v2 (@modelcontextprotocol/server).`

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | Compatibility Module > isCompatibleServerType > should validate a fully compatible server | Returns the same object; `writeToLog` not called. | N/A: TS server shape detection; Rust uses trait bounds. |
| 2 | ... > should throw error and log warning for null server | Throws `/does not match any supported MCP SDK shape/`; logs. | N/A: same. |
| 3 | ... > should throw error and log warning for undefined server | Same as row 2 for `undefined`. | N/A: same. |
| 4 | ... > should throw error and log warning for non-object server | `"not an object"`, `42`, `true`: each throws the same error and calls `writeToLog` exactly 1 time. | N/A: same. |
| 5 | ... > unknown-shape error names the full support matrix, never 'upgrade to v1.11' | `{some: "unrecognized shape"}`: message contains `SUPPORT_MATRIX_SUFFIX` and does not match `/v1\.11 or higher/`. | N/A: same. |
| 6 | ... > logs a shape-fingerprint beacon when rejecting an unrecognized server shape | `{server: {}, setRequestHandler() {}}` throws; a log line contains `"unrecognized server shape \| signals"`, `"hasServerProp"`, and the suffix. | N/A: same. |
| 7 | ... > logs '(none)' as the fingerprint for signal-free non-object rejects | `null`: first log line contains `"unrecognized server shape \| signals (none)"`. | N/A: same. |
| 8 | ... > should throw the unknown-shape error when setRequestHandler is missing | Throws `/does not match any supported MCP SDK shape/`; logs. | N/A: same. |
| 9 | ... > should throw the unknown-shape error when setRequestHandler is not a function | `setRequestHandler: "not a function"`: same error. | N/A: same. |
| 10 | ... > should throw the unknown-shape error when _requestHandlers is missing | Same error. | N/A: same. |
| 11 | ... > should throw the unknown-shape error when _requestHandlers is not a Map | `_requestHandlers: {}`: same error. | N/A: same. |
| 12 | ... > should throw the unknown-shape error when _requestHandlers is a non-Map with a get method | `{get: "not a function"}`: same error. | N/A: same. |
| 13 | ... > should throw error and log warning when getClientVersion is missing | Throws `/getClientVersion/`; message contains the suffix; logs. | N/A: same. |
| 14 | ... > should throw error and log warning when getClientVersion is not a function | Throws `/getClientVersion/`. | N/A: same. |
| 15 | ... > should throw error and log warning when _serverInfo is missing | Throws `/_serverInfo/`. | N/A: same. |
| 16 | ... > should throw error and log warning when _serverInfo is not an object | `_serverInfo: "not an object"`: throws `/_serverInfo/`. | N/A: same. |
| 17 | ... > should throw error and log warning when _serverInfo.name is missing | `_serverInfo: {}`: throws `/_serverInfo/`. | N/A: same. |
| 18 | Compatibility Module > getMCPCompatibleErrorMessage > should handle Error instances with standard properties | `JSON.parse(result)`: `message "Test error message"`, `name "Error"` (if present), `stack` defined. | N/A: JS error-to-JSON helper for log lines; Rust logs `Display`. |
| 19 | ... > should handle Error instances with custom properties | `code "CUSTOM_ERROR"`, `statusCode 500`, `message "Test error"`. | N/A: same. |
| 20 | ... > should handle string errors | `"String error message"` is returned unchanged. | N/A: same. |
| 21 | ... > should handle plain objects | Equals `JSON.stringify({code: "ERROR_CODE", details: "Some details"})`. | N/A: same. |
| 22 | ... > should handle null | `"Unknown error"`. | N/A: same. |
| 23 | ... > should handle undefined | `"Unknown error"`. | N/A: same. |
| 24 | ... > should handle numbers | `42` gives `"Unknown error"`. | N/A: same. |
| 25 | ... > should handle booleans | `false` gives `"Unknown error"`. | N/A: same. |
| 26 | ... > should handle circular reference errors | Error with `error.circular = error` gives `"Unknown error"`. | N/A: same. |
| 27 | ... > should handle arrays | Equals `JSON.stringify(["error1", "error2"])`. | N/A: same. |
| 28 | Compatibility Module > McpServer compatibility > should handle McpServer instances with server property | High-level wrapper (`server`, `requestContext`, `toolCallId`, `_registeredTools`, `tool()`) is returned as is; no log. | N/A: TS high-level/low-level shapes. |
| 29 | ... > should validate the underlying server when McpServer wrapper is provided | Inner server without `setRequestHandler`: throws `/does not match any supported MCP SDK shape/`; logs. | N/A: same. |
| 30 | ... > should handle objects with server property that is not an object | `server: "not an object"` on a valid low-level object: returned as is; no log. | N/A: same. |
| 31 | ... > should handle objects with null server property | `server: null`: returned as is; no log. | N/A: same. |
| 32 | ... > should validate all required properties on the underlying server | Wrapper cases: missing `_requestHandlers` throws the shape error; missing `getClientVersion` throws `/getClientVersion/`; missing `_serverInfo` throws `/_serverInfo/`; each logs. | N/A: same. |
| 33 | Compatibility Module > logCompatibilityWarning > should log the correct compatibility message to the log file | 1 log call; message matches `/compatibility/i` and `/Model Context Protocol\|MCP/` (actual: `AgentCat SDK Compatibility: <suffix>`). | N/A: no runtime SDK compatibility check in Rust. |
| 34 | Compatibility Module > McpServer integration tests > should work with real McpServer instances if available | Real `McpServer({name: "test-mcp-server", version: "1.0.0"})` is returned as is. | N/A: TS MCP SDK. |
| 35 | ... > should validate real McpServer underlying server properties | Inner server has function `setRequestHandler`, `Map` `_requestHandlers`, function `getClientVersion`; `_serverInfo` name `"test-validation-server"`, version `"1.0.0"` when present. | N/A: TS MCP SDK internals. |
| 36 | ... > should show compatibility message when McpServer is not available | Returns early when `McpServer` exists (it always does); else asserts `hasCompatibleVersion === false`. | N/A: TS MCP SDK version probe. |

Quirks:

- Row 36 asserts nothing on any supported SDK version (it returns early). Row 35 skips its assertions if `_serverInfo` is missing.
- Row 26: an `Error` with a self-reference logs as `"Unknown error"`, so the log line loses the message too.

### src/tests/concurrency.test.ts (1 tests)

Purpose: concurrent tool calls keep their own session, agent and actor attribution.

Harness: `setupTestHooks()`; a fresh `EventCapture` per test. v1 `McpServer({name: "concurrent server", version: "1.0"})`
with tool `slow_echo` (`"echoes slowly"`, `{text: z.string(), delay_ms: z.number()}`) that sleeps
`delay_ms` and returns `text`. `track(server, "proj_test", {enableAgentTracking: true, identify: async (request) => ({userId: "actor-" + request.params?.arguments?.text})})`.
`Client({name: "test client", version: "1.0"})` over an in-memory pair; `client.listTools()` runs
first so the registry exists.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | concurrent tool calls do not clobber each other's attribution > each event keeps its own session_id, agent_id, and actor under interleaving | Three parallel calls, each with `context: "concurrency regression test"`: `{text: "A", delay_ms: 120, session_id: sid("session_A"), agent_id: "agt_A"}`, `{text: "B", delay_ms: 40, session_id: sid("session_B"), agent_id: "agt_B"}`, `{text: "C", delay_ms: 80, session_id: sid("session_C"), agent_id: "agt_C"}`. Exactly 3 `mcp:tools/call` events. Each event, matched by raw `arguments.text`: `sessionId === sid("session_X")`, `tags` include `agentcat_agent_id: "agt_X"`, `identifyActorGivenId === "actor-X"`. | port |

Quirks: none. (The test reads identity without `capture.flush()`; it relies on the hook settling before the 40–120 ms handler delays end.)

### src/tests/constants.test.ts (11 tests)

Purpose: byte-exact pins for wire literals and the agent-facing handle copy.

Harness: imports from `modules/constants.ts`; no server, no events.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | brand constants (wire literals) > emits the agentcat custom event type on the wire | `AGENTCAT_CUSTOM_EVENT_TYPE === "agentcat:custom"`. | port |
| 2 | brand constants (wire literals) > stamps exporter source as agentcat | `AGENTCAT_SOURCE === "agentcat"`. | port |
| 3 | brand constants (wire literals) > uses the agentcat diagnostics scope name | `DIAGNOSTICS_SCOPE_NAME === "agentcat-diagnostics"`. | port |
| 4 | brand constants (wire literals) > mirrors under the mcp_session key | `MCP_SESSION_KEY === "mcp_session"`. | port |
| 5 | handle param descriptions name the delivery channel > session_id pre-announces both delivery channels and both text headers | `SESSION_ID_PARAM_DESCRIPTION` contains `"mcp_session"`, `"a text block at the start of the result beginning [session_id issued"` and `"a text block beginning [session_id unrecognized"`. `MINT_BACK_HEADER_ISSUED` matches `/^\[session_id issued/`; `MINT_BACK_HEADER_UNRECOGNIZED` matches `/^\[session_id unrecognized/`. | port |
| 6 | handle param descriptions name the delivery channel > agent_id describes self-chosen generation, not server delivery | `AGENT_ID_PARAM_DESCRIPTION` contains `"opus-4.80-1m\|claude-code\|k3n9x"`, `"never inherited"`, `"generates a new one"`, and does not contain `"the server will issue"`. | port |
| 7 | v4 copy pins (byte-exact) > pins SESSION_ID_PARAM_DESCRIPTION | Equals: `Session continuity handle, one of two values: the ses_ ID issued for the task underway, or start. This server cannot link your calls between requests on its own, so session continuity travels in this parameter instead. If you were handed a session_id for this task — for example by the agent that spawned you — send that exact value from your first call. Otherwise send start on your first call; the server will issue an opaque correlation ID in the mcp_session field of the result, or in a text block at the start of the result beginning [session_id issued. Then send that exact ses_ value on every later call and hand it to any subagents working the same task. start always begins a new, unrelated task — never send it mid-task. If you send a value this server does not recognize, the result reports it: mcp_session.status of unrecognized, or a text block beginning [session_id unrecognized; re-send the ID issued for this task, or start if none was issued yet. Never invent a ses_ value.` | port |
| 8 | v4 copy pins (byte-exact) > pins MCP_SESSION_STATUS_DESCRIPTION | Equals: `issued: first call of a task; the session_id above was just created. active: the session_id you sent was accepted; keep sending it. unrecognized: the value sent was not issued by this server — re-send the one issued earlier for this task; if none was issued yet, send start to be issued a new one.` | port |
| 9 | v4 copy pins (byte-exact) > pins MINT_BACK_ISSUED_BODY | Equals: `This is the first-call issuance described in this tool's session_id parameter description.` | port |
| 10 | v4 copy pins (byte-exact) > pins MINT_BACK_UNRECOGNIZED_BODY | Equals: `The value sent was not issued by this server. Re-send the session_id issued earlier for this task; if none was issued yet, send start and one will be issued.` | port |
| 11 | v4 copy pins (byte-exact) > pins the session_id value-contract constants | `SESSION_ID_PARAM_PATTERN === "^(start\|ses_[0-9A-Za-z]{27})$"`; `SESSION_START_SENTINEL === "start"`. The compiled pattern accepts `"start"` and `"ses_" + "a"×27`; rejects `"ses_" + "a"×25`, `"restart"` and `"START"`. | port (the `regex` crate gives the same results) |

Quirks:

- Row 11: the advertised pattern rejects `"START"`, but call-time resolution trims and lowercases, so `" START "` still starts a new task. A schema-validating client and the server disagree. The source comment says this is deliberate.

### src/tests/context-parameters-edge-cases.test.ts (12 tests)

Purpose: skip and edge rules of `addContextParameterToTool(s)` (collisions, composed schemas, `additionalProperties`, empty schemas).

Harness: unit tests. `vi.spyOn(logging, "writeToLog").mockImplementation(() => {})` before each
test; `vi.restoreAllMocks()` after. Tool literals only.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | Context Parameters Edge Cases > existing context parameter collision > should skip injection for tools with existing context parameter | `test-tool` with `context: {type: "number", description: "existing"}` keeps `type "number"` and `description "existing"`; a log line contains `"already has 'context' parameter"`. | port |
| 2 | ... > existing context parameter collision > should warn with tool name when context exists | The log line contains `"my-special-tool"`. | port |
| 3 | ... > complex schema handling > should skip injection for oneOf schemas | `union-tool` (`oneOf` of two objects): `properties` stays `undefined`; log contains `"complex schema"`. | port |
| 4 | ... > complex schema handling > should skip injection for allOf schemas | `intersection-tool`: same result. | port |
| 5 | ... > complex schema handling > should skip injection for anyOf schemas | `anyof-tool` `{anyOf: [{type: "string"}, {type: "number"}]}`: same result. | port |
| 6 | ... > additionalProperties constraint > should inject context and remove additionalProperties:false constraint | `strict-tool`: `context.type "string"` added; `additionalProperties` is deleted (`undefined`). | port (`#[serde(deny_unknown_fields)]` makes `schemars` emit `additionalProperties: false`) |
| 7 | ... > additionalProperties constraint > should preserve additionalProperties:true when injecting | `additionalProperties` stays `true`; `context` added. | port |
| 8 | ... > edge case schemas > should handle empty object schema {} | `inputSchema: {}` gets `properties.context.type "string"`. | port |
| 9 | ... > edge case schemas > should handle schema with no inputSchema | No `inputSchema`: one is created with `type "object"` and `context`. | adapt: rmcp `Tool.input_schema` is never absent; cover it with an empty map. |
| 10 | ... > edge case schemas > should handle schema with only type specified | `{type: "object"}` gets `context`. | port |
| 11 | ... > addContextParameterToTools batch handling > should skip get_more_tools entirely | `get_more_tools` with `{}` keeps `properties` `undefined`; `other-tool` gets `context`. | port |
| 12 | ... > addContextParameterToTools batch handling > should handle mixed valid and complex schemas | `valid-tool` gets `context`; `complex-tool` (`oneOf`) unchanged; `collision-tool` keeps `context.type "number"`; `strict-tool` gets `context` and loses `additionalProperties`; `writeToLog` called exactly 2 times. | port |

Quirks:

- Row 8: `{}` gets `properties` and `required: ["context"]`, but no `type: "object"` (audit 01 §15 #12).
- Rows 6 and 12: injection deletes `additionalProperties: false`, so the listing no longer tells clients that the tool rejects unknown keys. Strict tools keep working only because the call wrapper strips injected keys first.
- Row 1: injection skips a customer-owned `context`, but the call wrapper still records its value as `userIntent` (audit 01 §15 #6). No test covers this.

### src/tests/context-parameters.test.ts (10 tests)

Purpose: `context` injection in listings, `userIntent` capture, and stripping `context` before the customer handler.

Harness: `beforeEach` runs `resetTodos()` and `setupTestServerAndClient()`; `afterEach` runs
`cleanup()`. Integration rows use `EventCapture` and wait 50–100 ms.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | Context Parameters > addContextParameterToTools > should add context parameter to tools without inputSchema | `{name: "simple_tool", description: "A simple tool"}` gets `inputSchema.type "object"`, `properties.context.type "string"`, `required` containing `"context"`. | adapt: rmcp `Tool.input_schema` is never absent; cover it with an empty map. |
| 2 | ... > should add context parameter to tools with existing inputSchema | `text` kept; `context.type "string"`; `required` contains `"text"` and `"context"`. | port |
| 3 | ... > should not duplicate context parameter if already exists | Existing `context` description `"Existing context"` kept; `required` has exactly one `"context"`. | port |
| 4 | ... > should handle tools with empty required array | `required: []` becomes `["context"]` (length 1). | port |
| 5 | Context Parameters > Integration with MCP server tracking > should capture context parameter when tools are called after tracking | `track(server, "test-project", {enableReportMissing: true, enableTracing: true, enableToolCallContext: true})`; no `tools/list` first. `add_todo {text: "Test todo item", context: "Testing context parameter injection for analytics"}`: `content[1].text` contains `"Added todo"`; the `add_todo` event has `userIntent === "Testing context parameter injection for analytics"`. | port |
| 6 | ... > should work with tools that have context parameter | `add_todo {text: "Todo to complete", context: "Creating a todo to test completion"}`, then `complete_todo {id: "1", context: "Testing completion with context tracking"}`: `content[1].text` contains `"Completed todo"`; that event's `userIntent === "Testing completion with context tracking"`. | port |
| 7 | ... > should capture userIntent only when context is provided | `list_todos {}` succeeds although `context` is schema-required (`content[0].text` defined); then `list_todos {context: "Listing todos to check current status"}`. At least 2 `list_todos` events; one has a falsy `userIntent`; one has `userIntent === "Listing todos to check current status"`. | port |
| 8 | ... > should inject context into tool schemas when listing tools | After `track(server, "test-project", {enableToolCallContext: true})` and `tools/list`, running `addContextParameterToTools` again on the result keeps at least 3 tools; `add_todo`, `list_todos`, `complete_todo` each have `context.type "string"` and `description === DEFAULT_CONTEXT_PARAMETER_DESCRIPTION` (injection is idempotent). | port |
| 9 | ... > should use default context description when no custom description is provided | The same 3 tools carry `DEFAULT_CONTEXT_PARAMETER_DESCRIPTION`. | port |
| 10 | ... > should remove context parameter before calling tool callback | Tool `test_context_removal` (`{testParam: z.string()}`) registered before `track(server, {projectId: "test-project", enableTracing: true})`. Call `{testParam: "test-value", context: "This context should be removed before callback"}`: `isError` is not `true`; the callback saw `testParam === "test-value"` and no `context` key. | port |

Quirks:

- Row 10 uses the legacy signature `track(server, {projectId, ...})`. The options object becomes the `projectId` and the real options default to `{}`. The test passes anyway (audit 01 §15 #16).
- Row 7: `context` is advertised as `required` but never enforced.

### src/tests/context-preservation.test.ts (2 tests)

Purpose: a customer's own `context` parameter is never replaced by the default.

Harness: unit tests on `RegisteredTool` literals with no `name` (the log names the tool
`"unknown"`) and `callback: async () => ({content: []})`.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | Context Parameter Preservation > should preserve existing context parameter with custom description | Description stays `"A custom context description that should be preserved"` and is not `DEFAULT_CONTEXT_PARAMETER_DESCRIPTION`. | port |
| 2 | Context Parameter Preservation > should add context parameter when it doesn't exist | Schema `{foo}` gets `context.description === DEFAULT_CONTEXT_PARAMETER_DESCRIPTION`. | port |

Quirks: none.

### src/tests/custom-context-description.test.ts (8 tests)

Purpose: `customContextDescription` replaces the default description on every injected `context`.

Harness: same as `context-parameters` (todo fixture per test, `cleanup()` after). Rows 4 and 6
use `EventCapture` and a 50 ms wait.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | Custom Context Description > should use custom description instead of default for JSON Schema tools | `customContextDescription: "Explain your reasoning for this action"`: `add_todo` `context.description` equals it and is not the default. | port |
| 2 | ... > should use custom description for Zod object schemas | `"Provide context about why you're doing this"` on `complete_todo`. | port |
| 3 | ... > should apply custom description to all tools when listing | `"Why are you calling this tool?"` on `add_todo`, `list_todos`, `complete_todo` (exactly 3 matched). | port |
| 4 | ... > should capture tool calls with custom description configured | Description `"Tell me what you're trying to accomplish"`; `add_todo {text: "Test todo item", context: "I need to add a task to my list"}`: `content[1].text` contains `"Added todo"`; event `userIntent === "I need to add a task to my list"`. | port |
| 5 | ... > should use default description when customContextDescription is not provided | `add_todo` uses `DEFAULT_CONTEXT_PARAMETER_DESCRIPTION`. | port |
| 6 | ... > should work end-to-end with custom description and multiple tool calls | Listing shows `"Explain your intent for this operation"`. Calls `add_todo {text: "First task", context: "Setting up my first task"}`, `complete_todo {id: "1", context: "Finishing the first task"}`, `list_todos {context: "Checking my progress"}`: at least 3 events, and each `userIntent` equals its context. | port |
| 7 | ... > should use custom description even with long descriptions | Verbatim: `Please provide a comprehensive explanation of your reasoning, including the broader context of this action within your workflow, the expected outcomes, and how this contributes to your overall objectives. Be as detailed as possible.` | port |
| 8 | ... > should use custom description with special characters | Verbatim: `Why? (explain in detail) - "Be specific!"` | port |

Quirks:

- An empty-string `customContextDescription` falls back to the default (`||`). No test covers this.

### src/tests/debug-tool-schema.test.ts (2 tests)

Purpose: a debugging aid that prints the TS SDK's `_registeredTools` before and after `track()`.

Harness: `new McpServer({name: "debug server", version: "1.0.0"})` per test; `console.log` output;
no client, no events.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | Debug Tool Schema Structure > should show the structure of tools registered with shorthand Zod syntax | Only `expect(server).toBeDefined()`. It logs `_registeredTools["add"].inputSchema` before and after `track(server, "test-project", {enableToolCallContext: true, enableTracing: true})`. | N/A: asserts no behavior; prints TS SDK internals. |
| 2 | Debug Tool Schema Structure > should show the structure of tools registered with description and schema | Only `expect(server).toBeDefined()` for tool `add_with_desc` (`"Adds two numbers"`). | N/A: same. |

Quirks:

- Both tests always pass; they assert nothing about AgentCat.

### src/tests/detect-shapes.test.ts (2 tests)

Purpose: `detectServer` null path and fake high-level shells.

Harness: fake object shapes only; no SDK import.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | detectServer (fake shapes) > returns null for a high-level shell passing low-level gates but with neither .tool nor registerTool | Result `null`. | N/A: TS MCP SDK v1/v2 shape detection. |
| 2 | detectServer (fake shapes) > detects fake high-level shells with .tool (v1) and registerTool (v2) | `{server: low, tool() {}}` gives `{major: 1, flavor: "high"}`; `{server: low, registerTool() {}}` gives `{major: 2, flavor: "high"}`. | N/A: same. |

Quirks: none.

### src/tests/diagnostics-attributes.test.ts (5 tests)

Purpose: static OTLP resource attributes on SDK diagnostics.

Harness: `_resetDiagnosticsForTest()` before and after each test;
`process.env.DISABLE_DIAGNOSTICS = "false"` force-enables diagnostics past test detection;
`_getStaticAttributesForTest()` reads the attributes; `asMap` flattens them. No flush, no network.
The source also adds `process.runtime.version`, `process.pid`, `os.version`, `host.arch`,
`host.cpu.count` and `deployment.environment` (from `NODE_ENV`); empty values are omitted.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | diagnostics static attributes > uses projectId verbatim when present | `agentcat.project_id === "proj_ABC"`; no `agentcat.install_id`. | port |
| 2 | diagnostics static attributes > falls back to a stable install_id when projectId is absent | `projectId: null`: no `agentcat.project_id`; `agentcat.install_id` is truthy and equal across two inits (source: first 16 hex chars of `sha256(hostname + "\|" + import.meta.url)`). | adapt: seed with the hostname and a stable Rust value (for example the executable path); keep 16 hex chars. |
| 3 | diagnostics static attributes > includes sdk language, sdk version, and os/runtime metadata | `agentcat.sdk.language === "typescript"`; `agentcat.sdk.version`, `os.type`, `process.runtime.name` truthy (`"nodejs"` or `"other"`). | adapt: language `"rust"`; runtime name and version from Rust (for example a build-time `rustc` version). |
| 4 | diagnostics static attributes > reports the installed v1 SDK version despite the exports stub | `agentcat.mcp_sdk.version` matches `/^1\./`. | adapt: report the compiled `rmcp` version (a build-time constant). |
| 5 | diagnostics static attributes > reports the installed v2 SDK version despite sealed exports | `agentcat.mcp_sdk_v2.version` matches `/^2\./`. | N/A: second TS SDK major; Rust has one MCP crate. |

Quirks: none.

### src/tests/diagnostics-auth.test.ts (3 tests)

Purpose: the bearer token on the diagnostics POST.

Harness: `_resetDiagnosticsForTest()`, `DISABLE_DIAGNOSTICS="false"`, `delete DIAGNOSTICS_TOKEN`;
`globalThis.fetch = vi.fn().mockResolvedValue({ok: true, status: 200})`. Flow:
`initDiagnostics({projectId: "proj_1"})`, `writeToLog("Warning: something")`,
`await flushDiagnostics()`, then read `fetchSpy.mock.calls[0][1].headers`.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | diagnostics auth header > sends the default bearer token by default | `Authorization` matches `/^Bearer dgk_sdk_diag_/` (default token `dgk_sdk_diag_3f9a2c7e1b8d4065af2e9c1d7b6a4f80`). | port |
| 2 | diagnostics auth header > honors a custom DIAGNOSTICS_TOKEN override | `DIAGNOSTICS_TOKEN="custom-token-123"` gives `"Bearer custom-token-123"`. | port |
| 3 | diagnostics auth header > omits the Authorization header when token is explicitly empty | `DIAGNOSTICS_TOKEN=""` still sends the default token (`/^Bearer dgk_sdk_diag_/`). | port |

Quirks:

- Row 3's name says "omits", but it asserts that the header is present. An empty variable falls back to the default token, so the code branch that omits the header can never run.

### src/tests/diagnostics-export.test.ts (3 tests)

Purpose: OTLP/JSON log export of buffered diagnostics.

Harness: same `fetch` spy and `DISABLE_DIAGNOSTICS="false"`. Source facts: endpoint
`https://otel.agentcat.com` or `DIAGNOSTICS_ENDPOINT` (trailing slashes removed, `/v1/logs`
appended unless present); header `Content-Type: application/json`; body
`{resourceLogs: [{resource: {attributes}, scopeLogs: [{scope: {name: "agentcat-diagnostics", version: <sdk version>}, logRecords: [...]}]}]}`;
buffer cap 1000 records (oldest dropped); a 2000 ms batch timer that does not keep the process alive.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | diagnostics export > POSTs buffered entries as OTLP logs JSON to /v1/logs | `writeToLog("Warning: something happened")` then flush: 1 `fetch`; URL ends with `/v1/logs`; `resourceLogs[0].scopeLogs[0].logRecords[0].body.stringValue` contains `"something happened"`; resource attribute `agentcat.project_id === "proj_1"`. | port |
| 2 | diagnostics export > resolves even when fetch rejects (fire-and-forget isolation) | `fetch` rejects with `Error("network down")`; after `writeToLog("Failed to send event")`, `flushDiagnostics()` resolves to `undefined`. | port |
| 3 | diagnostics export > does nothing when disabled | `initDiagnostics({projectId: "proj_1", disabled: true})`: no `fetch`. | port |

Quirks: none.

### src/tests/diagnostics-integration.test.ts (2 tests)

Purpose: `track()` feeds the diagnostics sink.

Harness: `fetch` spy; `DISABLE_DIAGNOSTICS="false"`; `_resetDiagnosticsForTest()` before and after.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | diagnostics integration with track() > captures and ships diagnostics when track() runs on an invalid server | `track({}, "proj_int")` hits its catch (`Warning: Failed to track server - ...`); after flush: 1 `fetch`; `logRecords.length > 0`. | adapt: Rust cannot pass an invalid server; produce any log line during setup (for example the setup beacons) and assert 1 POST with at least 1 record. |
| 2 | diagnostics integration with track() > ships nothing when disabled via option | `track({}, "proj_int", {disableDiagnostics: true})`: no `fetch`. | port (option `disable_diagnostics: true`) |

Quirks: none.

### src/tests/diagnostics-no-payload.test.ts (3 tests)

Purpose: the diagnostics sink receives metadata only, never event payloads.

Harness: `DISABLE_DIAGNOSTICS="1"` and `_resetDiagnosticsForTest()` keep real diagnostics off;
the todo fixture; `EventCapture`; each test registers `setDiagnosticsSink((entry) => captured.push(entry))`.
`afterEach` clears the sink, the env var and the capture, then runs `cleanup()`. Secrets come from
`randomUUID()`. 50 ms waits. Beacon formats in the source:
`AgentCat setup started | project <projectId or "(telemetry-only)"> | sdk v<major>/<flavor> | signals <list>` and
`AgentCat setup complete | project <...> | tracing=<bool> context=<bool> reportMissing=<bool> exporters=<n>`.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | diagnostics sink never receives event payloads > resolved identity is stamped on the event but never reaches the sink | `identify` returns `{userId: "actor-<uuid>", userName: "name-<uuid>", userData: {secret: "data-<uuid>"}}`; call `add_todo {text: "hi", context: "adding a todo"}`. The `mcp:tools/call` event has `identifyActorGivenId === userId` and `identifyActorName === SECRET_NAME`. No sink line contains `"with identity:"`, the user ID, the name, or the secret data. | port |
| 2 | diagnostics sink never receives event payloads > emits start + complete setup beacons (metadata only) on track() | `track(server, "test-project", {enableTracing: true})`. The `"AgentCat setup started"` line contains `"test-project"`, `"sdk v1/high"`, `"signals "`. The `"AgentCat setup complete"` line contains `"test-project"`, `"tracing=true"`, `"reportMissing=true"`. | adapt: keep both beacons and the `tracing=`/`reportMissing=` fields; replace `sdk v1/high` and `signals` with Rust facts (for example the rmcp version). |
| 3 | diagnostics sink never receives event payloads > report-missing log carries only the context length, not the text | `get_more_tools {context: "i-wish-there-was-a-tool-for-<uuid>"}`: a line contains `"Missing tool reported"` and `"context length: <N>"` (N = JS string length); no line contains the context text. | port (ASCII here; document the length unit Rust uses) |

Quirks: none.

### src/tests/diagnostics-optout.test.ts (17 tests)

Purpose: when diagnostics turn on and off (option, env var, test-runner detection).

Harness: `_resetDiagnosticsForTest()` and `delete DISABLE_DIAGNOSTICS` before and after each test;
`vi.stubEnv` and `vi.unstubAllEnvs()`. `stubNonTestEnvironment()` sets `VITEST`, `JEST_WORKER_ID`,
`NODE_ENV` and `DISABLE_DIAGNOSTICS` to `undefined`. Reads `isDiagnosticsEnabled()`. Rule in the
source: `enabled = !opts.disabled && flag !== "disabled" && (flag === "force-enabled" || !isTestEnvironment())`.
`DISABLE_DIAGNOSTICS` is trimmed and lowercased: `false`, `0`, `no`, `off` force-enable; any
other non-blank value disables; blank means unset. `initDiagnostics` runs once per process.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | diagnostics opt-out > is enabled by default outside test environments | Non-test env, `initDiagnostics({projectId: "proj_1"})`: `true`. | port |
| 2 | diagnostics opt-out > is disabled via the option | `disabled: true`: `false`. | port |
| 3 | diagnostics opt-out > is disabled via the env var | `DISABLE_DIAGNOSTICS="1"`: `false`. | port |
| 4 | diagnostics opt-out > is disabled when DISABLE_DIAGNOSTICS=true | `"true"`: `false`. | port |
| 5 | diagnostics opt-out > is disabled when DISABLE_DIAGNOSTICS=TRUE | `"TRUE"`: `false`. | port |
| 6 | diagnostics opt-out > is disabled when DISABLE_DIAGNOSTICS=1 | `"1"`: `false`. | port |
| 7 | diagnostics opt-out > is disabled when DISABLE_DIAGNOSTICS=yes | `"yes"`: `false`. | port |
| 8 | diagnostics opt-out > is disabled when DISABLE_DIAGNOSTICS=on | `"on"`: `false`. | port |
| 9 | diagnostics opt-out > stays enabled when DISABLE_DIAGNOSTICS=false | `"false"` under vitest (test env not stubbed): `true`. | port |
| 10 | diagnostics opt-out > stays enabled when DISABLE_DIAGNOSTICS=FALSE | `"FALSE"`: `true`. | port |
| 11 | diagnostics opt-out > stays enabled when DISABLE_DIAGNOSTICS=0 | `"0"`: `true`. | port |
| 12 | diagnostics opt-out > stays enabled when DISABLE_DIAGNOSTICS=no | `"no"`: `true`. | port |
| 13 | diagnostics opt-out > stays enabled when DISABLE_DIAGNOSTICS=off | `"off"`: `true`. | port |
| 14 | diagnostics opt-out > treats a whitespace-only value as unset (default behavior applies) | `"  "` in a non-test env: `true`. | port |
| 15 | diagnostics opt-out > is disabled by default when VITEST=true (test environment) | `false`. | adapt: Rust has no standard test-runner variable; choose Rust signals or document that customers set `DISABLE_DIAGNOSTICS` in tests. |
| 16 | diagnostics opt-out > is disabled by default when JEST_WORKER_ID=1 (test environment) | `false`. | adapt: same as row 15. |
| 17 | diagnostics opt-out > is disabled by default when NODE_ENV=test (test environment) | `false`. | adapt: same as row 15. |

Quirks:

- `initDiagnostics` latches on the first `track()` in the process. A later `track()` with `disableDiagnostics: true` does nothing. The tests reset the latch, so they do not show this.

### src/tests/diagnostics-record.test.ts (5 tests)

Purpose: OTLP log record building.

Harness: `_resetDiagnosticsForTest()` and `initDiagnostics({projectId: "proj_1"})` before each
test. Diagnostics stay disabled (setup.ts sets `DISABLE_DIAGNOSTICS="true"`), but
`_buildRecordForTest(entry)` works anyway. Severity numbers: ERROR 17, WARN 13, INFO 9.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | diagnostics record building > carries the raw message as the body, verbatim | `"[2026-01-01T00:00:00Z] Warning: boom"` is the body unchanged. | port |
| 2 | diagnostics record building > infers severity from message content | `"Warning: x"` gives `WARN`; `"Failed to send event"` gives `ERROR`; `"Some error happened"` gives `ERROR`; `"Initialized telemetry"` gives `INFO`. | port |
| 3 | diagnostics record building > prioritizes fail/error over the Warning prefix | `"Warning: Failed to track server - boom"` gives `ERROR`; `"warning: nothing here"` gives `INFO` (the `Warning:` check is case-sensitive); `""` gives `INFO`. | port |
| 4 | diagnostics record building > sets a nanosecond timestamp string | `timeUnixNano` matches `/^\d+$/` (milliseconds × 1,000,000 as a decimal string). | port |
| 5 | diagnostics record building > attaches sdk, node, and mcp versions as record attributes | `agentcat.sdk.version` equals the package version; `process.runtime.version === process.version`; `agentcat.mcp_sdk.version` matches `/^1\./`; `agentcat.mcp_sdk_v2.version` matches `/^2\./`. | adapt: Rust SDK version, Rust runtime version, rmcp version; no v2 key. |

Quirks:

- The severity rule `/fail\|error/i` matches substrings, so any line that names `isError` or a tool called `error_tool` becomes ERROR. Lines that start with `WARN:` are INFO (audit 01 §15 #13).

### src/tests/diagnostics-sink.test.ts (3 tests)

Purpose: `writeToLog` copies every entry to a registered sink.

Harness: `setDiagnosticsSink(fn)`; `afterEach` runs `setDiagnosticsSink(null)`.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | logging diagnostics sink > forwards every log entry to a registered sink | `writeToLog("hello world")`: 1 entry, containing `"hello world"`. | port |
| 2 | logging diagnostics sink > never throws when the sink throws | Sink throws `Error("sink boom")`; `writeToLog("still fine")` does not throw. | adapt: call the sink inside `catch_unwind`; a sink panic must not escape. |
| 3 | logging diagnostics sink > stops forwarding after the sink is cleared | After `setDiagnosticsSink(null)`, `writeToLog("ignored")` adds 0 entries. | port |

Quirks: none.

### src/tests/diagnostics-test-env.test.ts (2 tests)

Purpose: diagnostics turn off automatically under a test runner.

Harness: `_resetDiagnosticsForTest()`, `delete DISABLE_DIAGNOSTICS`, `fetch` spy. Relies on the
`VITEST` and `NODE_ENV=test` values that vitest sets.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | diagnostics are disabled in test environments > track() without options does not enable diagnostics under vitest | `track({}, "proj_test_env")`: `isDiagnosticsEnabled() === false`; flush sends no `fetch`. | adapt: depends on the Rust test-runner signal chosen for `diagnostics-optout` rows 15–17. |
| 2 | diagnostics are disabled in test environments > DISABLE_DIAGNOSTICS=false force-enables diagnostics even under vitest | `true`. | port |

Quirks: none.

### src/tests/e2e-http/lanes.ts (0 tests)

Purpose: helper module. Five real-HTTP server topologies ("lanes") plus wait helpers for the e2e matrix.

Harness (exact behavior):

- `listenOnEphemeralPort(handler)`: `http.createServer`, listen on `127.0.0.1` port `0`; base URL `http://127.0.0.1:<port>/mcp`.
- `makeInstance(...)`: `connectClient(info = {name: "agentcat-e2e-client", version: "1.0.0"})` and an idempotent `close()`. Teardown order: every client `close()` (errors ignored), then the server-side close (errors ignored), then `httpServer.closeAllConnections()`, then an awaited `httpServer.close()`.
- `E2E_PROJECT_ID = "proj_test"`. `track()` gets the scenario's options.
- Lanes:

| Lane id | Major | Topology | Envelope | Wiring |
| --- | --- | --- | --- | --- |
| `v1-stateful` | 1 | stateful | no | One v1 toolkit server, tracked once. A `Map` keyed by `mcp-session-id` gives each session its own `StreamableHTTPServerTransport({sessionIdGenerator: randomUUID, enableJsonResponse: true})`. v1 client. |
| `v1-stateless` | 1 | per-request | no | Non-POST gets 405 with `Allow: POST` and body `{"jsonrpc":"2.0","error":{"code":-32000,"message":"Method not allowed."},"id":null}`. Each POST builds a fresh toolkit server, calls `track()`, uses `sessionIdGenerator: undefined`, and closes both on response close. v1 client. |
| `v2-handler-legacy` | 2 | per-request | no | `createMcpHandler(() => track(buildV2Toolkit(sink), "proj_test", options))` through `toNodeHandler`. v2 client with `versionNegotiation: {mode: "legacy"}`. |
| `v2-handler-auto` | 2 | per-request | yes | Same handler; v2 client with `mode: "auto"` (2026-07-28 envelope carries client info and protocol version per request). |
| `v2-longlived` | 2 | stateful | no | One tracked v2 server on one `NodeStreamableHTTPServerTransport({sessionIdGenerator: randomUUID, enableJsonResponse: true, keepAliveMs: 0})` (one session only). v2 client, legacy mode. |

- `laneById(id)` throws `unknown lane: <id>` on a typo.
- `waitForEvents(capture, count, timeout = 2000)`: `vi.waitFor` with 20 ms interval until the capture holds at least `count` events; error text `expected >= <count> captured events, saw <n>`.
- `settle(ms = 100)`: a fixed sleep for pipeline mutations.
- Re-exports `mintBackOf(result)` (returns `content[0].text` only if `content[0]` is a text block starting with `"[session_id"`) and `handleFrom(text, name)` (first capture of the regex `<name>: (\S+)`), both from `v2/harness.ts`.

Rust lane mapping (recommended):

| Rust lane | Replaces | Wiring |
| --- | --- | --- |
| stateful | `v1-stateful`, `v2-longlived` | `StreamableHttpService` with `LocalSessionManager`, `legacy_session_mode: true`, `json_response: true`, mounted at `/mcp` on an axum server bound to `127.0.0.1:0`. The factory returns a tracked wrapper that shares one AgentCat instance. |
| stateless-legacy | `v1-stateless`, `v2-handler-legacy` | `legacy_session_mode: false`; the factory runs per request; a client on a 2025 protocol version. rmcp synthesizes `Implementation::default()` as the client; the SDK must treat it as unknown. |
| stateless-2026 | `v2-handler-auto` | A 2026-07-28 rmcp client; `context.meta` carries client info and protocol version. |

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| — | (helper module, no tests) | — | — |

### src/tests/e2e-http/scenario-types.ts (0 tests)

Purpose: helper module. The scenario type and the shared table of 22 scenario configs (name, options, lanes).

Harness (exact values):

- `ScenarioContext = {client, capture, sink, lane, instance}`; `Scenario = {name, options, lanes?, script(ctx)}`; omitted `lanes` means every lane.
- Fixture constants: `SESSION_HOOK_VALUE = "corr-7"`, `AGENT_SESSION_HOOK_VALUE = "corr-agent-7"`, `CUSTOM_CONTEXT_DESCRIPTION = "why?"`, `REDACTION_SECRET = "SECRET_TOKEN"`, `REDACTION_REPLACEMENT = "[REDACTED]"`, `IDENTITY_USER_ID = "e2e-user-1"`, `IDENTITY_USER_NAME = "E2E User"`, `IDENTITY_USER_DATA = {plan: "enterprise"}`, `CUSTOMER_TAG_KEY = "e2e_customer_tag"`, `CUSTOMER_TAG_VALUE = "customer-tag-present"`, `COLLIDING_TAG_VALUE = "customer-collision-loses"`, `CUSTOMER_PROP_KEY = "e2e_customer_prop"`, `CUSTOMER_PROP_VALUE = "prop-value"`.
- Configs, in order:

| # | Name | Options | Lanes |
| --- | --- | --- | --- |
| 1 | `baseline-defaults` | `{}` | all |
| 2 | `tracing-off` | `{enableTracing: false}` | all |
| 3 | `tracing-off-x-report-missing-on` | `{enableTracing: false}` | all |
| 4 | `report-missing-off` | `{enableReportMissing: false}` | all |
| 5 | `report-missing-call` | `{}` | all |
| 6 | `context-off` | `{enableToolCallContext: false}` | all |
| 7 | `context-custom-description` | `{customContextDescription: "why?"}` | all |
| 8 | `agent-tracking-mint` | `{enableAgentTracking: true}` | all |
| 9 | `agent-tracking-supplied` | `{enableAgentTracking: true}` | all |
| 10 | `agent-tracking-x-hook` | `{enableAgentTracking: true, resolveSessionId: () => "corr-agent-7"}` | all |
| 11 | `task-hook-deterministic` | `{resolveSessionId: () => "corr-7"}` | all |
| 12 | `task-hook-null` | `{resolveSessionId: () => null}` | all |
| 13 | `task-supplied-continuity` | `{}` | all |
| 14 | `mint-back-structured-mirror` | `{}` | all |
| 15 | `strict-schema-injection` | `{}` | all |
| 16 | `strip-proof-loose` | `{}` | all |
| 17 | `error-path` | `{}` | all |
| 18 | `hooks-identity-tags-props` | `identify: async () => ({userId: "e2e-user-1", userName: "E2E User", userData: {plan: "enterprise"}})`; `eventTags: () => ({agentcat_session_id_source: "customer-collision-loses", e2e_customer_tag: "customer-tag-present"})`; `eventProperties: () => ({e2e_customer_prop: "prop-value"})` | all |
| 19 | `redaction` | `redactSensitiveInformation: async (text) => text.split("SECRET_TOKEN").join("[REDACTED]")` | all |
| 20 | `client-identity-ladder` | `{}` | all |
| 21 | `rebuild-on-demand` | `{}` | `v1-stateless`, `v2-handler-legacy`, `v2-handler-auto` |
| 22 | `session-continuity-stateful` | `{}` | `v1-stateful`, `v2-longlived` |

- `scenarioConfig(name)` throws `unknown scenario config: <name>` on a typo.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| — | (helper module, no tests) | — | — |

### src/tests/e2e-http/toolkit.ts (0 tests)

Purpose: helper module. The same five tools for both SDK majors, each recording what the customer handler received.

Harness (exact behavior):

- `ToolSink = {tool, args}[]`. Every handler first pushes `{tool, args: {...args}}`. The sink shows what reached the customer handler after AgentCat stripped injected params.
- `buildV1Toolkit(sink)`: `McpServer({name: "e2e-http-v1", version: "1.0.0"})`, zod v3. `buildV2Toolkit(sink)`: v2 `McpServer({name: "e2e-http-v2", version: "1.0.0"}, {capabilities: {tools: {}}})`, zod v4.
- Tools (same names and behavior on both):
  - `echo` (`"Echo a message back"`, `{msg: string}` described `"Message to echo"`): returns text `String(msg)`.
  - `loose` (`"Loose schema: unknown keys pass through to the handler"`, `{msg?: string}` with passthrough / `looseObject`): returns `JSON.stringify(args)`.
  - `strict` (`"Strict schema: additionalProperties are rejected"`, `{msg: string}` `.strict()` / `strictObject`): returns text `msg`.
  - `structured` (`"Returns structuredContent per its outputSchema"`, input `{msg: string}`, output `{echoed: string}` described `"The echoed message"`): returns text `msg` and `structuredContent {echoed: msg}`.
  - `boom` (`"Always throws"`, empty passthrough object): throws `Error("boom")`; the MCP SDK turns it into an `isError` result.

Rust toolkit notes: `loose` = a struct with `#[serde(flatten)] extra: serde_json::Map<String, Value>`;
`strict` = `#[serde(deny_unknown_fields)]`; `structured` = a `Json<T>` return so rmcp declares
an output schema; `boom` must return a tool error result (for example `Err(String)` through
rmcp's `IntoCallToolResult`) to match the TS `isError` result. An `Err(McpError)` is a JSON-RPC
error instead, which has no content to decorate; cover it as a separate case.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| — | (helper module, no tests) | — | — |

### src/tests/e2e-http/header-capture.test.ts (2 tests)

Purpose: inbound HTTP headers reach `parameters.extra` on published events.

Harness: lanes `v2-longlived` and `v1-stateful` started with `{}` options. Each test builds its
own client transport with `requestInit.headers {"x-agentcat-health-run": "hcrun:header-capture"}`
(client `{name: "agentcat-e2e-client", version: "1.0.0"}`, v2 in legacy mode). `EventCapture`
per test; `waitForEvents(capture, 1)` and `settle()`. `deepContains(value, needle)` is a
recursive exact-value search over arrays and object values (mirrors `_deep_contains` in the
health harness `verify_events.py`).

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | e2e-http: HTTP headers land in published event parameters > v2 (longlived lane): sentinel header at extra.http.req.headers | After `listTools()` and `echo {msg: "hi", context: "header capture"}`: `parameters.extra.http.req.headers["x-agentcat-health-run"] === "hcrun:header-capture"`; `extra.http.req.method === "POST"`; `typeof extra.http.req.url === "string"`; `deepContains(parameters, "hcrun:header-capture") === true`. | adapt: project rmcp's `http::request::Parts` from `context.extensions` into `{http: {req: {method, url, headers}}}`. |
| 2 | e2e-http: HTTP headers land in published event parameters > v1 (stateful lane): sentinel header at extra.requestInfo.headers, url a string | `extra.requestInfo.headers["x-agentcat-health-run"] === "hcrun:header-capture"`; `typeof extra.requestInfo.url === "string"` and it contains `"/mcp"`; `deepContains` true. | N/A: TS v1 `requestInfo` shape. Rust has one projection shape (row 1), which keeps the `deepContains` check. |

Quirks:

- Every inbound header is published as is, including `Authorization` and `Cookie` (audit 02, defect 20). The tests check only the sentinel.

### src/tests/e2e-http/sse-smoke-v1.test.ts (1 tests)

Purpose: the baseline scenario over SSE-framed POST responses on the v1 stateful topology.

Harness: copies the `v1-stateful` wiring from `lanes.ts` with `enableJsonResponse: false`.
Options come from `scenarioConfig("baseline-defaults")` (`{}`); project `"proj_test"`. One v1
client `{name: "agentcat-e2e-client", version: "1.0.0"}`. `EventCapture` per test; idempotent close.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | e2e-http v1 SSE smoke (stateful, enableJsonResponse: false) > baseline-defaults over streamed POST responses | Listing has `echo` and `get_more_tools`; `echo` has `context` and `session_id`. `echo {msg: "hi", context: "sse smoke intent"}`: mint-back contains `MINT_BACK_HEADER_ISSUED`; its `session_id` matches `/^ses_/`. Exactly 1 event: `sessionId` equals that ID, `userIntent "sse smoke intent"`, tag `agentcat_session_id_source: "minted"`. Sink: one `echo` entry with `msg "hi"`, no `session_id`, no `context`. | port (rmcp `json_response: false`) |

Quirks: none.

### src/tests/e2e-http/sse-smoke-v2.test.ts (1 tests)

Purpose: the baseline scenario on the v2 long-lived topology, and proof that responses really stream as SSE.

Harness: local `NodeStreamableHTTPServerTransport({sessionIdGenerator: randomUUID, enableJsonResponse: false, keepAliveMs: 0})`;
`track(server, "proj_test", {})`. On each POST, `res.writeHead` is wrapped to record the
`content-type` header. v2 client in legacy mode. Teardown: client close, transport close,
`closeAllConnections()`, `close()`.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | e2e-http v2 SSE smoke: long-lived lane, streaming POST responses > baseline-defaults over a real text/event-stream response | Listing has `echo` and `get_more_tools`; `echo` has `context` and `session_id`. `echo {msg: "hi", context: "sse smoke intent"}`: mint-back ID matches `/^ses_/`. Exactly 1 `echo` event with `sessionId` matching `/^ses_/` and `userIntent "sse smoke intent"`. Sink stripped (`msg "hi"`, no `session_id`, no `context`). At least 3 POST responses have a content type containing `"text/event-stream"` (initialize, tools/list, tools/call); none contains `"application/json"`. | port (record response content types in an axum middleware) |

Quirks: none.

### src/tests/e2e-http/v1-matrix.test.ts (42 tests)

Purpose: 22 scenarios over real HTTP on both v1 lanes.

Harness: `describe.each([v1-stateful, v1-stateless])("e2e-http v1 lane: $id")` and
`it.each(applicable)("$name")`. Each case: new `ToolSink`, `lane.start(scenario.options, sink)`,
`instance.connectClient()`, run the script, then `instance.close()` in `finally`. `EventCapture`
per case. `rawArgsOf(event)` = `event.parameters.request.params.arguments` (raw, after
redaction). 20 scenarios run on both lanes; `rebuild-on-demand` runs only on `v1-stateless`;
`session-continuity-stateful` runs only on `v1-stateful`. Rust: run each row on the matching Rust
lanes from the `lanes.ts` section.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 ×2 | e2e-http v1 lane: $id > baseline-defaults | Lanes: both. Listing has `echo`, `get_more_tools`; `echo.context.description === DEFAULT_CONTEXT_PARAMETER_DESCRIPTION`; `session_id` present, in `required`, with `pattern "^(start\|ses_[0-9A-Za-z]{27})$"`. `echo {msg: "hi", context: "e2e smoke intent"}` (no `session_id`): mint-back contains `"[session_id issued — see this tool's session_id parameter description]"`; ID matches `/^ses_/`. Exactly 1 event: `sessionId` = that ID, `userIntent "e2e smoke intent"`, tag source `"minted"`, raw args `context "e2e smoke intent"` and `msg "hi"`. Sink: 1 `echo` entry, `msg "hi"`, no `session_id`, no `context`. | port |
| 2 ×2 | ... > tracing-off | Lanes: both. `echo` has no `session_id` and no `agent_id`, but has `context`. `echo {msg: "hi"}`: `content[0].text === "hi"`; no mint-back. After `settle()` (100 ms): 0 events; sink has 1 `echo` entry. | port |
| 3 ×2 | ... > tracing-off-x-report-missing-on | Lanes: both. `get_more_tools` is listed; `get_more_tools {context: "need a search tool"}`: `content[0].text` contains `"we have shown you the full tool list"`; no mint-back; 0 events after 100 ms. | port |
| 4 ×2 | ... > report-missing-off | Lanes: both. No `get_more_tools`; `echo` has `session_id` and `context`. `echo {msg: "hi", context: "no gmt"}`: mint-back ID matches `/^ses_/`; exactly 1 event. | port |
| 5 ×2 | ... > report-missing-call | Lanes: both. `get_more_tools {context: "Need a database query tool for SQL operations"}`: `content[1].text` contains `"Unfortunately"` and `"we have shown you the full tool list"`; mint-back ID `/^ses_/`. Exactly 1 event: `resourceName "get_more_tools"`, `userIntent` = the context, `sessionId` = the mint-back ID; `JSON.stringify(event.response)` does not contain `"[session_id"`. | port |
| 6 ×2 | ... > context-off | Lanes: both. `echo` has no `context`, has `session_id`. `echo {msg: "hi", session_id: sid("ctxoff")}`: event `sessionId === "ses_ctxoff000000000000000000000"`, tag source `"supplied"`, `userIntent` undefined. | port |
| 7 ×2 | ... > context-custom-description | Lanes: both. `echo.context.description === "why?"` (not the default); `get_more_tools.context.description !== "why?"`. `echo {msg: "hi", context: "custom description intent"}`: event `resourceName "echo"`, `userIntent "custom description intent"`. | port |
| 8 ×2 | ... > agent-tracking-mint | Lanes: both. `structured` has `agent_id` and `session_id`, both in `required`; `get_more_tools` has `agent_id`, and its `required` contains `agent_id` and `session_id`. `structured {msg: "hi", context: "agent minting", session_id: "start"}`: mint-back contains the issued header and not `"agent_id"`; ID `/^ses_/`; `structuredContent.mcp_session` has `session_id` = ID, no `agent_id`, `status "issued"`; `structuredContent.echoed === "hi"` (the v1 client validated it against the injected output schema). Event: `sessionId` = ID, tag source `"minted"`, no `agentcat_agent_id` or `agentcat_agent_id_source` tag. | port |
| 9 ×2 | ... > agent-tracking-supplied | Lanes: both. Call 1 `echo {msg: "one", context: "agent supplied 1", session_id: "start"}`: issued block without `"agent_id"`. Call 2 `echo {msg: "two", context: "agent supplied 2", session_id: <ID>, agent_id: "opus-4.80-1m\|claude-code\|k3n9x"}`: no mint-back. 2 events: event 0 has no `agentcat_agent_id`; event 1 has `sessionId` = ID and tags `agentcat_session_id_source "supplied"`, `agentcat_agent_id "opus-4.80-1m\|claude-code\|k3n9x"`, `agentcat_agent_id_source "supplied"`. | port |
| 10 ×2 | ... > agent-tracking-x-hook | Lanes: both. `echo` and `get_more_tools` have no `session_id`; both have `agent_id` in `required`. `echo {msg: "hi", context: "hook plus agent", agent_id: "opus-4.80-1m\|claude-code\|k3n9x"}`: no mint-back. Event: `sessionId` matches `/^ses_/`; tags `agentcat_session_id_source "hook"`, `agentcat_agent_id "opus-4.80-1m\|claude-code\|k3n9x"`, `agentcat_agent_id_source "supplied"`. | port |
| 11 ×2 | ... > task-hook-deterministic | Lanes: both. Two `echo` calls (`msg "one"`, `"two"`; `context "hook determinism"`): no mint-back. 2 events with the same `/^ses_/` ID (on the stateless lane, across fresh tracked instances); both tag source `"hook"`. | port (also pin the derived ID for `"corr-7"` + `"proj_test"` as a golden value) |
| 12 ×2 | ... > task-hook-null | Lanes: both. Two calls (`context "hook null"`): no mint-back; 2 events, each `/^ses_/` with tag source `"minted"`; the two IDs differ. | port |
| 13 ×2 | ... > task-supplied-continuity | Lanes: both. Call 1 `{msg: "one", context: "continuity 1", session_id: "start"}` gives an ID; call 2 `{msg: "two", context: "continuity 2", session_id: <ID>}` has no mint-back. 2 events, both with that ID; tag sources `"minted"`, then `"supplied"`. | port |
| 14 ×2 | ... > mint-back-structured-mirror | Lanes: both. Listed `structured.outputSchema.properties.mcp_session` is defined. `structured {msg: "mirrored", context: "structured mirror", session_id: "start"}`: mirror `session_id` = mint-back ID, `status "issued"`; `echoed "mirrored"`; `Object.keys(structuredContent)[0] === "mcp_session"`. Event `sessionId` = ID; `JSON.stringify(event.response)` does not contain `"mcp_session"`. | port (the first-key rule needs ordered JSON maps) |
| 15 ×2 | ... > strict-schema-injection | Lanes: both. `strict.inputSchema.additionalProperties` is undefined; `session_id` and `context` present. `strict {msg: "strict ok", context: "strict schema test", session_id: sid("strict1")}`: `isError` falsy; the text block is `"strict ok"`. Event `sessionId === sid("strict1")`; sink `strict` args equal exactly `{msg: "strict ok"}`. | port |
| 16 ×2 | ... > strip-proof-loose | Lanes: both. `loose {msg: "keep", customer_extra: "customer says hi", context: "strip proof", session_id: sid("raw1")}`. Sink: `msg "keep"`, `customer_extra "customer says hi"`, no `session_id`, no `context`. Event raw args equal exactly `{msg: "keep", customer_extra: "customer says hi", context: "strip proof", session_id: sid("raw1")}`; `sessionId === sid("raw1")`; `userIntent "strip proof"`. | port |
| 17 ×2 | ... > error-path | Lanes: both. `boom {context: "error path intent"}`: `isError === true`; a mint-back is present (ID `/^ses_/`). Event: `isError true`, `error.message` contains `"boom"`, `userIntent "error path intent"`, `sessionId` = ID, tag source `"minted"`. Follow-up `echo {msg: "still alive", context: "post-error"}`: `isError` falsy; 2 events; event 1 has `resourceName "echo"` and `isError` undefined. | adapt: the Rust `boom` must return an `isError` tool result; add a second case where it returns `Err(McpError)` (a JSON-RPC error with no content, so no mint-back). |
| 18 ×2 | ... > hooks-identity-tags-props | Lanes: both. `echo {msg: "hi", context: "hooks combo"}`: `identifyActorGivenId "e2e-user-1"`, `identifyActorName "E2E User"`, `identifyActorData` equals `{plan: "enterprise"}`; `tags.e2e_customer_tag "customer-tag-present"`; `tags.agentcat_session_id_source "minted"` (the SDK value beats the customer's `"customer-collision-loses"`); `properties` includes `{e2e_customer_prop: "prop-value"}`. | port |
| 19 ×2 | ... > redaction | Lanes: both. `echo {msg: "payload with SECRET_TOKEN inside", context: "intent mentioning SECRET_TOKEN", session_id: sid("redact")}`. After `waitForEvents` and 100 ms: `JSON.stringify(event.response)` contains `"[REDACTED]"` and not `"SECRET_TOKEN"`; raw `msg === "payload with [REDACTED] inside"`; `userIntent === "intent mentioning [REDACTED]"`; `sessionId === sid("redact")` and `resourceName "echo"` are not redacted. | port |
| 20 ×2 | ... > client-identity-ladder | Lanes: both. `echo {msg: "who am I", context: "identity ladder"}`. `v1-stateful`: `clientName "agentcat-e2e-client"`, `clientVersion "1.0.0"` (handshake). `v1-stateless`: both `undefined` (the handshake hit another instance; no `_meta`). Both: no `agentcat_protocol_version` tag. A second call with request `_meta {"io.modelcontextprotocol/protocolVersion": "2026-07-28"}` (`msg "again"`, `context "identity ladder meta"`): event 1 has tag `agentcat_protocol_version "2026-07-28"`. | adapt: stateful lane reads `peer_info()`; on the stateless lane rmcp synthesizes `Implementation::default()` (name `"rmcp"`), which the SDK must treat as unknown; read the protocol version from `context.meta` only. |
| 21 ×1 | ... > rebuild-on-demand | Lane: `v1-stateless` only. No `tools/list`. Low-level `request({method: "tools/call", params: {name: "echo", arguments: {msg: "cold start", context: "rebuild on demand"}}}, CallToolResultSchema)`: mint-back ID `/^ses_/`. Exactly 1 event: `resourceName "echo"`, `sessionId` = ID, `userIntent "rebuild on demand"`. Sink args equal exactly `{msg: "cold start"}` (the rebuilt registry, not the fallback, did the strip). | port (the stateless rmcp factory gives a fresh wrapper per request) |
| 22 ×1 | ... > session-continuity-stateful | Lane: `v1-stateful` only. The client transport's `sessionId` (HTTP `mcp-session-id`) is a non-empty string and is unchanged after two calls. Call 1 `{msg: "one", context: "session continuity 1", session_id: "start"}`; call 2 `{msg: "two", context: "session continuity 2", session_id: <ID>}` has no mint-back. 2 events with that ID; tag sources `"minted"`, then `"supplied"`. | port |

Quirks:

- Row 2: with tracing off, `context` is still injected and advertised as required, but nothing records it (audit 01 §15 #3).
- Row 3: with tracing off, `get_more_tools` is still listed and answers "We have noted your feedback", but no event is published, so the report is lost.
- Row 20: on per-request lanes with a 2025-era client, events carry no client identity at all. The test pins this limitation.

### src/tests/e2e-http/v2-matrix.test.ts (63 tests)

Purpose: the same 22 scenarios over real HTTP on the three v2 lanes.

Harness: `describe.each([v2-handler-legacy, v2-handler-auto, v2-longlived])("e2e-http v2 lane: $id")`
and `it.each(applicable)("$name")`; the per-case flow matches `v1-matrix`. `rawArgsOf(event)` =
`event.parameters?.request?.params?.arguments ?? {}`. Absence checks use `settle(150)`.
`rebuild-on-demand` runs on `v2-handler-legacy` and `v2-handler-auto`;
`session-continuity-stateful` runs only on `v2-longlived`. Differences from `v1-matrix` are
listed per row.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 ×3 | e2e-http v2 lane: $id > baseline-defaults | Lanes: all three. Listing has `echo`, `get_more_tools`; `echo` has `context` and `session_id` (in `required`, `pattern "^(start\|ses_[0-9A-Za-z]{27})$"`). `echo {msg: "hi", context: "e2e smoke intent"}`: mint-back ID `/^ses_/`. Exactly 1 `echo` event: `sessionId` `/^ses_/`, `userIntent "e2e smoke intent"`, tag source `"minted"`, raw `msg "hi"` and `context "e2e smoke intent"`. Sink stripped. On `v2-handler-auto` only: tag `agentcat_protocol_version "2026-07-28"`. | port |
| 2 ×3 | ... > tracing-off | Lanes: all three. `echo` properties lack `session_id` and `agent_id`, have `context`. `echo {msg: "hi"}`: `content[0].text === "hi"`; no mint-back; after 150 ms, 0 events; 1 sink entry. | port |
| 3 ×3 | ... > tracing-off-x-report-missing-on | Lanes: all three. `get_more_tools` listed; `{context: "need a search tool"}`: `content[0].text` contains `"we have shown you the full tool list"`; no mint-back; 0 events after 150 ms. | port |
| 4 ×3 | ... > report-missing-off | Lanes: all three. No `get_more_tools`; `echo` has `session_id` and `context`. No tool call (unlike v1). | port |
| 5 ×3 | ... > report-missing-call | Lanes: all three. `get_more_tools {context: "Need a database query tool for SQL operations"}`: `content[1].text` contains `"Unfortunately"` and `"we have shown you the full tool list"`. Event: `resourceName "get_more_tools"`, `userIntent` = context, `sessionId` `/^ses_/`, raw `arguments.context` = context (its bespoke `context` is a real parameter). | port |
| 6 ×3 | ... > context-off | Lanes: all three. No `context`; has `session_id`. `echo {msg: "hi", session_id: sid("ctxoff")}`: `sessionId === sid("ctxoff")`, `userIntent` undefined (no tag check, unlike v1). | port |
| 7 ×3 | ... > context-custom-description | Lanes: all three. `echo.context.description === "why?"` (not the default); `get_more_tools` description is not `"why?"`. `echo {msg: "hi", context: "custom description intent"}`: `userIntent "custom description intent"`. | port |
| 8 ×3 | ... > agent-tracking-mint | Lanes: all three. `echo` has `agent_id` and `session_id`, both in `required`. `structured {msg: "mint", context: "agent minting", session_id: "start"}`: block contains `"[session_id issued"` and not `"agent_id"`; ID `/^ses_/`; `echoed "mint"`; mirror `session_id` = ID, no `agent_id`, `status "issued"`. Event: ID, tag source `"minted"`, no agent tags. | port |
| 9 ×3 | ... > agent-tracking-supplied | Lanes: all three. Call 1 `{msg: "one", context: "agent supplied — mint first", session_id: "start"}`: issued block without `"agent_id"`. Call 2 `{msg: "two", context: "agent supplied — reuse", session_id: <ID>, agent_id: "opus-4.80-1m\|claude-code\|k3n9x"}`: no mint-back. Event 0 has no agent tag; event 1 has `sessionId` = ID and tags `agentcat_agent_id "opus-4.80-1m\|claude-code\|k3n9x"`, `agentcat_agent_id_source "supplied"`, `agentcat_session_id_source "supplied"`. | port |
| 10 ×3 | ... > agent-tracking-x-hook | Lanes: all three. `echo` and `get_more_tools` lack `session_id`, have `agent_id` in `required`. Two calls `{msg: "one" / "two", context: "agent hook", agent_id: "opus-4.80-1m\|claude-code\|k3n9x"}`: no mint-back either time. 2 events with the same `/^ses_/` ID; both tags `{agentcat_session_id_source: "hook", agentcat_agent_id: "opus-4.80-1m\|claude-code\|k3n9x", agentcat_agent_id_source: "supplied"}` (v1 makes one call). | port |
| 11 ×3 | ... > task-hook-deterministic | Lanes: all three. Two `echo` calls (`context "hook determinism"`): no mint-back; same `/^ses_/` ID on both events; tag source `"hook"`. | port |
| 12 ×3 | ... > task-hook-null | Lanes: all three. Two calls (`context "hook null"`): no mint-back; exactly 2 events, each `/^ses_/` with tag source `"minted"`; IDs differ. | port |
| 13 ×3 | ... > task-supplied-continuity | Lanes: all three. Contexts `"continuity — mint"` (with `session_id: "start"`) and `"continuity — supply"` (with the ID): second call has no mint-back; both events have the ID; tag sources `"minted"`, then `"supplied"`. | port |
| 14 ×3 | ... > mint-back-structured-mirror | Lanes: all three. `structured {msg: "mirrored", context: "structured mirror", session_id: "start"}`: block contains `"[session_id issued"`; `echoed "mirrored"`; mirror `session_id` = ID, `status "issued"`, no `agent_id`; `Object.keys(structuredContent)[0] === "mcp_session"`. Event ID; `JSON.stringify(event.response)` has no `"[session_id"`. | port |
| 15 ×3 | ... > strict-schema-injection | Lanes: all three. `additionalProperties` undefined; `session_id` and `context` present. `strict {msg: "strictly", context: "strict schema test", session_id: sid("strict")}`: `isError` not `true`; a content entry has text `"strictly"`. Event `sessionId === sid("strict")`; sink args equal exactly `{msg: "strictly"}`. | port |
| 16 ×3 | ... > strip-proof-loose | Lanes: all three. `loose {msg: "hi", customer_extra: "keep-me", context: "strip proof", session_id: sid("loose")}`. Sink: `msg "hi"`, `customer_extra "keep-me"`; no `session_id`, `context` or `agent_id`. Event `sessionId === sid("loose")`; raw `msg`, `customer_extra`, `session_id`, `context "strip proof"` all present. | port |
| 17 ×3 | ... > error-path | Lanes: all three. `boom {context: "error path intent"}`: `isError === true`; mint-back ID `/^ses_/`. Event: `resourceName "boom"`, `isError true`, `error.message` contains `"boom"`, `userIntent "error path intent"`, `sessionId` `/^ses_/`, tag source `"minted"`, `JSON.stringify(event.response)` has no `"[session_id"`. Follow-up `echo {msg: "after", context: "error recovery"}`: `isError` not `true`; event 1 is `echo` with `isError` not `true`. | adapt: same as `v1-matrix` row 17. |
| 18 ×3 | ... > hooks-identity-tags-props | Lanes: all three. `echo {msg: "hooks", context: "hooks combo"}`: identity `"e2e-user-1"` / `"E2E User"` / `{plan: "enterprise"}`; `tags.e2e_customer_tag "customer-tag-present"`; `tags.agentcat_session_id_source "minted"` and not `"customer-collision-loses"`; `properties` includes `{e2e_customer_prop: "prop-value"}`. | port |
| 19 ×3 | ... > redaction | Lanes: all three. `echo {msg: "payload SECRET_TOKEN inside", context: "intent mentioning SECRET_TOKEN", session_id: sid("redact")}`: response JSON contains `"[REDACTED]"` and not `"SECRET_TOKEN"`; raw `msg === "payload [REDACTED] inside"`; `userIntent === "intent mentioning [REDACTED]"`; `sessionId === sid("redact")`; `resourceName "echo"`. | port |
| 20 ×3 | ... > client-identity-ladder | Lanes: all three. `echo {msg: "who am I", context: "identity ladder"}`. `v2-handler-auto`: `clientName "agentcat-e2e-client"`, `clientVersion "1.0.0"`, tag `agentcat_protocol_version "2026-07-28"` (envelope). `v2-longlived`: same client (handshake), no protocol tag. `v2-handler-legacy`: client name and version `undefined`, no protocol tag. | adapt: same as `v1-matrix` row 20; the stateless-2026 lane reads both from `context.meta`. |
| 21 ×2 | ... > rebuild-on-demand | Lanes: `v2-handler-legacy`, `v2-handler-auto`. No `tools/list`; `client.request({method: "tools/call", params: {name: "echo", arguments: {msg: "solo", context: "rebuild on demand"}}})`: a content entry has text `"solo"`; mint-back ID `/^ses_/`. Event: `resourceName "echo"`, `sessionId` = ID, `userIntent "rebuild on demand"` (no sink check, unlike v1). | port |
| 22 ×1 | ... > session-continuity-stateful | Lane: `v2-longlived` only. `client.transport.sessionId` is defined and unchanged after two calls (context `"stateful continuity"`, first with `session_id: "start"`); second call has no mint-back; both events have the ID; tag sources `"minted"`, then `"supplied"`. | port |

Quirks: the same as `v1-matrix` rows 2, 3 and 20.

### src/tests/e2e-sanitization.test.ts (4 tests)

Purpose: sanitization of non-text content and large base64 strings on real tool calls.

Harness: the todo fixture per test; `EventCapture`; `track(server, <project>, {enableToolCallContext: false, enableTracing: true})`
runs before the extra tool is registered (so the tool is added after tracking); 100 ms wait.
Source rules: `image` and `audio` blocks become text blocks; `resource` with `blob` becomes
`[binary resource content redacted - not supported by AgentCat]`; unknown types become
`[unsupported content type "<type>" redacted - not supported by AgentCat]`; strings in
`parameters` and `structuredContent` of length ≥ 10240 that match `/^[A-Za-z0-9+/\n\r]+=*$/`
become `[binary data redacted - not supported by AgentCat]`.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | E2E Sanitization - real MCP tool calls > should sanitize image content blocks in tool responses | Tool `get_attachment` returns `[{type: "text", text: "Here is the attachment:"}, {type: "image", data: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJ", mimeType: "image/png"}]`. Call `{id: "att_1", session_id: sid("e2e"), agent_id: "agt_e2e"}`. Event `response.content` has length 2: `{type: "text", text: "Here is the attachment:"}` and `{type: "text", text: "[image content redacted - not supported by AgentCat]"}`. | port |
| 2 | ... > should sanitize audio content blocks in tool responses | `get_audio_clip` returns `{type: "audio", data: "UklGRiQAAABXQVZFZm10IBAAAAABAAEA", mimeType: "audio/wav"}`; call `{clipId: "clip_1"}`. Event `content[0]` equals `{type: "text", text: "[audio content redacted - not supported by AgentCat]"}` (the wire result has a mint-back, the event does not). | port |
| 3 | ... > should sanitize large base64 strings in tool call parameters | `upload_file {filename: "photo.png", data: "A"×12000 + "="}`: event `arguments.data === "[binary data redacted - not supported by AgentCat]"`; `arguments.filename === "photo.png"`. | port |
| 4 | ... > should preserve normal todo tool calls while sanitizing problematic ones | `add_todo {text: "Buy groceries"}` event: `response.content[0].type "text"`, text contains `"Added todo"`. `get_todo_screenshot` (no args; returns text `"Screenshot of current todos:"` and image `data "iVBORw0KGgo="`): `content[0]` is the text block unchanged; `content[1]` is the image redaction text. | port |

Quirks:

- The base64 rule matches any 10 KB run of letters and digits with no spaces, so long non-binary strings (for example a hex dump or a long identifier list) are also redacted.
- Row 1's comment says that both handles suppress the mint-back. Only `session_id` matters (agent tracking is off), and the event keeps the original result anyway because the mint-back exists only on the wire.

### src/tests/e2e-truncation.test.ts (4 tests)

Purpose: truncation of large responses and parameters on real tool calls.

Harness: same as `e2e-sanitization`. Source limits: strings longer than 32,768 characters are cut
to 32,768 and get `"..."`; the whole event is kept at or under 102,400 bytes.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | E2E Truncation - real MCP tool calls > should truncate tool responses with text exceeding 32KB | `get_large_report {topic: "quarterly sales"}` returns `"Report on quarterly sales: " + "x"×50000` (50,027 chars). Event `response.content[0].text.length ≤ 32768 + 3` and ends with `"..."`. | port (JS length counts UTF-16 units; Rust must choose and document its unit) |
| 2 | ... > should keep total event size under 100KB for oversized parameters | `process_bulk_data {dataset: "d"×80000, metadata: "m"×40000}`: UTF-8 byte length of `JSON.stringify(event)` ≤ 102400. | adapt: measure the serialized publish body; Rust has no camelCase in-memory event to stringify. |
| 3 | ... > should preserve normal todo responses while truncating large ones | `add_todo {text: "Write tests"}`: `content[0].text` contains `"Added todo"`. `get_verbose_log` returns `"LOG: " + "entry "×10000` (60,005 chars): text length ≤ 32771. | port |
| 4 | ... > should handle both sanitization and truncation in the same tool call | `get_annotated_screenshot {page: "dashboard", session_id: sid("e2e"), agent_id: "agt_e2e"}` returns `[{type: "text", text: "x"×50000}, {type: "image", data: "iVBORw0KGgo=", mimeType: "image/png"}]`. Event content length 2; `content[0]` ≤ 32771 chars and ends with `"..."`; `content[1]` is the image redaction text; total ≤ 102400 bytes. | adapt: same size measurement as row 2. |

Quirks: none.

### src/tests/edge-runtime-compatibility.test.ts (23 tests)

Purpose: SDK behavior when Node modules or `process` are missing (Cloudflare Workers and other edge runtimes), plus exception capture robustness.

Harness: no server. Tests replace `globalThis.process` (delete `cwd`, a throwing `cwd`, delete
`once`, delete `process`, a minimal `{env: {}}`) and call `vi.resetModules()` so module-level code
runs again on the next dynamic import. `captureException` is imported directly. Source facts:
`platform` is always `"javascript"`; chains stop at 10 (`MAX_EXCEPTION_CHAIN_DEPTH`); frames stop
at 50; non-Error values stringify as `"null"`, `"undefined"`, the string itself, `String(n)`, or
`JSON.stringify`.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | Edge Runtime Compatibility > Exception Capture - Graceful Degradation > should capture basic exception info without filesystem access | `captureException(new Error("Test error"))`: `message "Test error"`, `type "Error"`, `platform "javascript"`. | adapt: Rust `platform "rust"`, `type` = short type name, `message` = `Display`. |
| 2 | ... > should parse stack traces without requiring fs module | `frames` is a non-empty array; frame 0 has `filename` and a numeric `lineno`. | adapt: frames exist only when a backtrace was captured (`RUST_BACKTRACE`, `anyhow`); otherwise none. |
| 3 | ... > should handle chained errors (Error.cause) | `"Wrapper error"` with cause `"Root cause"`: `message "Wrapper error"`, `chained_errors.length 1`, `chained_errors[0].message "Root cause"`. | adapt: walk `std::error::Error::source()`. |
| 4 | ... > should handle non-Error objects being thrown | `"string error"` gives `"string error"`; `{code: 404}` gives `'{"code":404}'`; `null` gives `"null"`; `undefined` gives `"undefined"`. | adapt: the Rust analog is a panic payload (`&str`/`String` gives its text; other payloads give a fixed placeholder). |
| 5 | Edge Runtime Compatibility > Process Object Availability > should handle missing process.cwd gracefully | Without `process.cwd`: `message "Test without cwd"`, `type "Error"`. | N/A: Node `process` object. |
| 6 | ... > should handle process.cwd throwing | A throwing `cwd`: `message "Test with throwing cwd"`. | N/A: same. |
| 7 | Edge Runtime Compatibility > Event Queue Signal Handlers > should not throw when process.once is unavailable | Importing `eventQueue.js` resolves. | N/A: Node signal handlers registered at module load; Rust installs none and uses `shutdown().await`. |
| 8 | ... > should handle process being undefined | `module.eventQueue` is defined. | N/A: same. |
| 9 | Edge Runtime Compatibility > Logging Module Behavior > should export writeToLog function | `typeof writeToLog === "function"`. | N/A: JS module export. |
| 10 | ... > should not throw when called | `writeToLog("Test message")`, `writeToLog("")`, `writeToLog("Special chars: \n\t\r")` never throw. | port (logging never panics) |
| 11 | ... > should handle rapid successive calls | 100 calls `Message <i>` never throw. | port |
| 12 | Edge Runtime Compatibility > Full SDK API Availability > should export track function | `typeof agentcat.track === "function"`. | N/A: compile-time in Rust. |
| 13 | ... > should export publishCustomEvent function | `typeof agentcat.publishCustomEvent === "function"`. | N/A: same. |
| 14 | ... > should export type definitions | The module loads. | N/A: same. |
| 15 | Edge Runtime Compatibility > Edge Environment Detection Patterns > should detect Node.js environment correctly | A helper defined in the test returns `true`. | N/A: tests a helper inside the test, not SDK code. |
| 16 | ... > should detect Cloudflare Workers pattern | Helper returns `false`. | N/A: same. |
| 17 | ... > should detect generic edge runtime pattern | Helper returns `false`. | N/A: same. |
| 18 | Edge Runtime Compatibility > Path Normalization Without cwd > should handle various path formats | `captured.frames` is defined (the `testPaths` array is never used). | N/A: V8 stack path normalization; asserts only that `frames` exists. |
| 19 | Integration: SDK in Limited Environment > should work when process has limited functionality | `process = {env: {}}`: `index.js` loads; `track` and `publishCustomEvent` defined; capture gives `message "Limited env test"`. | N/A: edge runtimes. |
| 20 | Integration: SDK in Limited Environment > should handle environment transitions gracefully | Two `writeToLog` calls (`"Normal environment"`, `"Second call"`) do not throw. | port (logging keeps working across calls) |
| 21 | Error Handling Robustness > should not crash on malformed stack traces | `stack = "Error: Malformed\n    at malformed line without proper format\n    garbage data"`: `message "Malformed"`, `type "Error"`, `frames` defined. | adapt: frame parsing must not panic on unexpected symbol text. |
| 22 | Error Handling Robustness > should handle circular reference in error.cause | Two errors that cause each other: `message "Error 1"`; `chained_errors` defined, length ≤ 10. | adapt: safe Rust `source()` chains rarely cycle; keep the cap of 10 and a seen-pointer guard. |
| 23 | Error Handling Robustness > should handle deeply nested error chains | `"Root"` wrapped 20 times: `message "Level 19"`; `chained_errors.length ≤ 10`. | port (cap at 10) |

Quirks:

- Row 18 builds a `testPaths` array and never uses it. Rows 15–17 test helpers that live in the test file. These tests protect nothing in the SDK.
- Row 22: the seen-set does not include the top-level error, so the chain for Error 1 → Error 2 → Error 1 is `[Error 2, Error 1]`. The top-level error appears again inside its own chain.

### src/tests/engine-callwrap.test.ts (25 tests)

Purpose: the `tools/call` wrapper: stripping, events, MRTR tags, the `get_more_tools` intercept, degraded paths, `extra` projection, deferred hooks, the bounded registry rebuild, and fault containment after the handler.

Harness: `vi.mock("../modules/logging", () => ({writeToLog: vi.fn()}))`.
`fakeServer(callHandler)` = `{_requestHandlers: Map{"tools/call" → callHandler}, setRequestHandler() {}, getClientVersion: () => undefined, _serverInfo: {name: "fake", version: "0"}}`.
`trackFake(server, overrides)` sets tracking data
`{projectId: "proj_test", options: {enableReportMissing: true, enableTracing: true, enableToolCallContext: true, enableAgentTracking: false, ...overrides}}`,
calls `initEngineState(server, {adapter: v2Adapter})`, sets the injected-params registry
`{echo → {"session_id", "context"}}` and an empty output registry, runs `installCallWrap(server)`,
and returns the wrapper, which tests call directly as `wrapped(request, extra)`. `EventCapture`
per test; `capture.flush()` where hooks are deferred. The rebuild tests set `originalList` and no
registry; one uses `vi.useFakeTimers()` and `advanceTimersByTimeAsync(5000)`. Rust: a fake inner
handler with closure-driven `call_tool`/`list_tools`, the tracking wrapper, a duplex rmcp client,
a capturing publisher, and `tokio::time::pause()` for the 5 s bound.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | MRTR helpers > isInputRequiredShape matches only resultType === 'input_required' | `true` only for `{resultType: "input_required"}`; `false` for `{resultType: "complete"}`, `{content: []}`, `null`, `[]`. | adapt: match `CallToolResponse::InputRequired`. |
| 2 | MRTR helpers > mrtrContinuationTags fires only when inputResponses ride the request | `{mcpReq: {inputResponses: {a: {}}}}` gives `{agentcat_mrtr: "continuation"}`; `{mcpReq: {}}` and `undefined` give `{}`. | adapt: test `input_responses.is_some()` on the rmcp request. |
| 3 | installCallWrap (synthetic seam) > strips registry-scoped args before the original handler, keeps raw on the event | `echo {msg: "hi", session_id: sid("supplied"), context: "why"}`: the handler sees exactly `{msg: "hi"}`; 1 event with `sessionId === sid("supplied")` and raw `arguments.session_id === sid("supplied")`; `result.content.length ≥ 1`. | port |
| 4 | installCallWrap (synthetic seam) > tags intermediate input_required rounds and skips mint-back on them | Handler returns `{resultType: "input_required", inputRequests: [{id: "q1"}]}`; no `session_id`; extra `{mcpReq: {inputResponses: undefined}}`. Result keeps `resultType "input_required"` and `content` stays `undefined`; event tag `agentcat_mrtr "input_required"`. | adapt: inner returns `CallToolResponse::InputRequired`; assert no decoration and the tag. |
| 5 | installCallWrap (synthetic seam) > tags continuation rounds from mcpReq.inputResponses | Extra `{mcpReq: {inputResponses: {q1: {value: "answer"}}}}`: tag `agentcat_mrtr "continuation"`. | adapt: the request carries `input_responses`. |
| 6 | installCallWrap (synthetic seam) > answers get_more_tools directly and publishes its event | The handler throws if reached. `get_more_tools {context: "need csv export"}`: `content[1].text` contains `"full tool list"`; event `resourceName "get_more_tools"`, `userIntent "need csv export"`. | port |
| 7 | installCallWrap (synthetic seam) > publishes an error event with __agentcat_error stack when present | Handler sets `extra.__agentcat_error = new Error("inner boom")` and returns `{content: [], isError: true}`: event `isError true`, `error.message` contains `"inner boom"`. | adapt: Rust needs an explicit side channel (for example a slot in `context.extensions`) to keep the original error when the router turns `Err` into an `isError` result; without it, capture the result text. |
| 8 | installCallWrap (synthetic seam) > degrades to the original handler with the UNSTRIPPED request when tracing setup fails | `getClientVersion` throws `"client info boom"`. The handler sees `{msg: "hi", session_id: "ses_x", context: "why"}` unstripped; `result.content` equals `[{type: "text", text: "ok"}]` (no mint-back); 0 events; log contains `"tracing failed for tool echo, falling back to original handler"`. | adapt: inject a panic in the setup step through a test seam and catch it. |
| 9 | installCallWrap (synthetic seam) > answers get_more_tools directly on the degraded path, without publishing | Same failure; `get_more_tools`: `content[0].text` contains `"full tool list"` (no mint-back); 0 events. | adapt: same fault injection as row 8. |
| 10 | installCallWrap (synthetic seam) > publishes an error event and rethrows when the original handler throws | Handler throws `"handler boom"`: the wrapper rejects with `"handler boom"`; event `isError true`, `error.message` contains `"handler boom"`, `duration ≥ 0`. | adapt: inner returns `Err(McpError)`; the wrapper returns the same `Err` and publishes an error event. |
| 11 | extra projection on captured events > publishes v1-shaped extra with headers verbatim and url as a string | Extra `{requestInfo: {headers: {"x-agentcat-health-run": "run:v1"}, url: new URL("http://localhost:4105/mcp")}, authInfo: {token: "tok"}}`. After 100 ms: `extra.requestInfo.headers["x-agentcat-health-run"] "run:v1"`, `extra.requestInfo.url "http://localhost:4105/mcp"`, `extra.authInfo.token "tok"`. | N/A: TS v1 `requestInfo`/`authInfo` shapes. |
| 12 | extra projection on captured events > publishes v2-shaped extra with the web Request as plain { method, url, headers } | Extra `{sessionId: "transport-1", mcpReq: {id: 5}, http: {req: new Request("http://127.0.0.1:4105/mcp", {method: "POST", headers: {"x-agentcat-health-run": "run:ts_v2"}})}}`: `extra.http.req.method "POST"`, `url "http://127.0.0.1:4105/mcp"`, `headers["x-agentcat-health-run"] "run:ts_v2"`. | adapt: put `http::request::Parts` in `context.extensions` and assert the same JSON shape. |
| 13 | get_more_tools gating > does not intercept when enableReportMissing is false — the customer's own tool runs | Options `{enableReportMissing: false}`; `get_more_tools {context: "need csv"}`: the customer handler runs once; the joined text contains `"customer answer"` and not `"full tool list"`. | port |
| 14 | get_more_tools gating > does not intercept on the tracing-off path when enableReportMissing is false | `{enableReportMissing: false, enableTracing: false}`: handler runs once; `content[0].text === "customer answer"`. | port |
| 15 | get_more_tools gating > does not intercept on the degraded path when enableReportMissing is false | `getClientVersion` throws: handler runs once; `content[0].text === "customer answer"`. | adapt: fault injection as in row 8. |
| 16 | deferred customer hooks > a hanging identify hook does not delay the tool call | `identify: () => new Promise(() => {})`: the result contains text `"ok"` within the 2000 ms test timeout. | port |
| 17 | deferred customer hooks > hanging resolveSessionId/eventTags/eventProperties do not delay the tool call | All three never settle: the result contains `"ok"` within 2000 ms. | port |
| 18 | deferred customer hooks > the handler starts before a slow hook settles | `identify` resolves `{userId: "u1"}` only after `releaseHook()`: `order[0] === "handler"`; after `capture.flush()`, `identifyActorGivenId "u1"`. | port |
| 19 | deferred customer hooks > a synchronously-throwing hook is contained and the event still publishes | `resolveSessionId` throws `"sync hook boom"`: result contains `"ok"`; event `sessionId` `/^ses_/`, tag `agentcat_session_id_source "minted"` (silent mint). | adapt: a panicking hook (`catch_unwind`) falls back to a silent mint. |
| 20 | deferred customer hooks > redactEvent sees resolved hook state, and pending never reaches it or the processed event | `identify` gives `{userId: "u-redact"}`, `eventTags` gives `{env: "prod"}`: the `redactEvent` input has `identifyActorGivenId "u-redact"`, `tags.env "prod"`, and no `pending` key; the captured event has no `pending`. | port (stage order: hooks resolve before `redactEvent`; the `pending` checks become type-level in Rust) |
| 21 | deferred customer hooks > error-path events carry hook results resolved after the throw | Handler throws `"handler boom"`; `identify` resolves after 20 ms to `{userId: "late-user"}`: the wrapper rejects; after flush, event `isError true` and `identifyActorGivenId "late-user"`. | port |
| 22 | registry rebuild bounding > bounds the rebuild: a hanging tools/list handler cannot hang tools/call | No registry; `originalList` never settles; fake timers advance 5000 ms: the result contains `"ok"`; log contains `"rebuild-on-demand failed"` (cause: `tools/list did not respond within 5000ms`). | port (`tokio::time::pause()` and `advance`) |
| 23 | registry rebuild bounding > keeps serving calls when the list handler fails every time | `originalList` throws `"list boom"`: two calls in a row both return `"ok"`. | port |
| 24 | finish() fault containment > returns the customer's isError result when a content entry is null | Result `{isError: true, content: [null, {type: "text", text: "quota exceeded, retry later"}]}`: returned `isError true`; content contains both original entries. | N/A: rmcp `Content` is typed; a null entry cannot exist. |
| 25 | finish() fault containment > returns the raw customer result when post-handler decoration throws | A content array whose iterator throws: the same result object is returned; log contains `"post-handler"`. | adapt: wrap decoration in `catch_unwind`; on panic return the inner result unchanged and log the `post-handler` line. |

Quirks:

- Row 8: on the degraded path the request reaches the handler unstripped. On a low-level server, a strict customer schema then rejects the injected `session_id` and `context`.
- Row 4: an `input_required` round without a `session_id` mints a new session ID that is never announced. The completing round mints a different ID, so the intermediate event belongs to a session the agent never learns.
- Row 3's comment ("mint-back only mints agent/nothing new") is stale; the SDK no longer mints agent IDs.
- Event `duration` includes the registry rebuild time (audit 01 §15 #14). No test covers this.

### src/tests/engine-innertap.test.ts (11 tests)

Purpose: in-place wrapping of the TS SDK's registered tool functions (a second strip and an error stash), the registry proxy, and the `setRequestHandler` patch.

Harness: `vi.mock("../modules/logging", () => ({writeToLog: vi.fn()}))`. Tool objects are
literals with `executor` (v2), `handler` or `callback` (v1). `server = {}` with
`setInjectedParamsRegistry(server, Map)`. The patch tests use a real v1 `Server` or a minimal
object with a `_requestHandlers` map.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | wrapToolEntry > wraps v2 executor in place: strips injected args, preserves customer args | Registry `{echo → {session_id, context}}`; the executor called with `{msg: "hi", session_id: "ses_x", context: "why"}` sees `{msg: "hi"}`. | adapt: Rust strips once in the wrapper's `call_tool`; assert the arguments the inner handler receives. |
| 2 | wrapToolEntry > captures thrown errors onto extra.__agentcat_error and rethrows | v1 `handler` throws `boom`: rejects with `"boom"`; `extra.__agentcat_error === boom`. | N/A: JS throw-to-result seam (see `engine-callwrap` row 7). |
| 3 | wrapToolEntry > supports the v1 no-args callback arity (extra only) | `callback(extra)` receives the same `extra` object. | N/A: TS v1 callback arity. |
| 4 | wrapToolEntry > preserves ctx position for v2 schema-less executors (args undefined) | `executor(undefined, {sessionId: "s"})` sees ctx `{sessionId: "s"}`. | N/A: TS v2 executor arity. |
| 5 | wrapToolEntry > is idempotent — wrapping twice does not double-wrap | The second wrap leaves the same function; one call gives `calls === 1`. | adapt: tracking an already tracked handler must not strip, decorate or publish twice. |
| 6 | wrapToolEntry > rewrapAllTools sweeps every registry entry | Sweeping twice leaves `a.executor` unchanged; `b.executor` is defined. | N/A: `_registeredTools` sweep. |
| 7 | inner-tap fault containment > rewrapAllTools swallows a _registeredTools whose enumeration throws | Proxy `ownKeys` throws: no throw; log contains `"inner-tap sweep failed"`. | N/A: JS Proxy seam. |
| 8 | inner-tap fault containment > registry proxy set-trap: a tool whose executor getter throws still lands in the registry | Tool `evil` with a throwing `executor` getter is still stored (`"evil" in _registeredTools`, listed in `Object.keys`); log contains `"Error in registry proxy for tool evil"`. | N/A: same. |
| 9 | inner-tap fault containment > installRegistryProxy swallows a highLevel whose _registeredTools assignment throws | No throw; log contains `"Failed to setup listener for registered tools"`. | N/A: same. |
| 10 | patchSetRequestHandler on v1 > introspects Zod schema registrations for the method literal | `setRequestHandler(CallToolRequestSchema, ...)` fires the callback exactly 1 time. | N/A: monkey-patching `setRequestHandler`. |
| 11 | patchSetRequestHandler on v1 > registration still succeeds when the re-arm callback throws | The callback throws `"re-arm boom"`: the customer's handler is still stored for `"tools/call"`; log contains `"engine re-arm failed"`. | N/A: same. |

Quirks: none.

### src/tests/engine-listwrap.test.ts (12 tests)

Purpose: the pure injection pipeline (`buildInjectedList`) and the `tools/list` wrapper's pass-through and fault rules.

Harness: `vi.mock("../modules/logging", () => ({writeToLog: vi.fn()}))`.
`data(overrides)` = `{projectId: "proj_test", options: {enableReportMissing: true, enableTracing: true, enableToolCallContext: true, enableAgentTracking: false, ...overrides}}`.
`echoTool()` = `{name: "echo", description: "Echo", inputSchema: {type: "object", properties: {msg: {type: "string"}}, required: ["msg"]}}`.
`fakeListServer(listHandler)` is `fakeServer` with `"tools/list"`. `listWith(tools)` tracks it,
installs the list wrapper, and calls it once.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | buildInjectedList > appends get_more_tools and injects session_id + context | Names `["echo", "get_more_tools"]`; `echo` has `session_id` and `context`; registry `echo → {"session_id", "context"}`, `get_more_tools → {"session_id"}`; `outputRegistry.size === 0`. | port |
| 2 | buildInjectedList > skips get_more_tools when enableReportMissing is false | Names `["echo"]`. | port |
| 3 | buildInjectedList > skips handle injection entirely when tracing is off, but still registers empty entries | `{enableTracing: false, enableToolCallContext: false}`: no `session_id`; registry `echo → {}` (empty set). | port |
| 4 | buildInjectedList > does not mutate the input tool objects | The input's `properties` still has no `session_id`. | port (copy on write, for example `Arc::make_mut`) |
| 5 | installListWrap (synthetic seam) > passes the original response through verbatim and warns when tracking data is missing | No tracking data: the same object is returned (1 tool `echo`, `nextCursor "page-2"`); log contains `"unable to find server tracking data"`. | N/A: the Rust wrapper owns its configuration, so tracking data cannot be missing. |
| 6 | installListWrap (synthetic seam) > passes through and warns on empty or missing tools; registries stay unset | `{tools: []}` and `{}`: the same object is returned; log contains `"No tools found in the original list"`; the registry stays undefined. | adapt: only the empty `Vec` case exists in rmcp (`tools` is not optional). |
| 7 | tools/list fault containment > lists a boolean-schema tool uninjected while the healthy tool keeps its handles | `{name: "poison", inputSchema: true}` passes through as `true`; `echo` gets `session_id` and `context`; a log line names `"poison"`. | N/A: rmcp `input_schema` is a JSON object; a boolean schema cannot be listed. |
| 8 | tools/list fault containment > passes a null tools-array entry through and injects the rest | The list still contains `null`; `echo` is injected. | N/A: typed `Vec<Tool>`. |
| 9 | tools/list fault containment > lists a circular-schema tool uninjected without failing the listing | `recursive.inputSchema` is the same object; `echo` is injected. | N/A: `serde_json::Value` cannot be cyclic. |
| 10 | tools/list fault containment > lists a BigInt-carrying schema uninjected without failing the listing | Tool `bigint` (`maximum: 9007199254740993n`) is still listed; `echo` is injected. | N/A: serde_json has no BigInt type. |
| 11 | tools/list fault containment > warns once when the customer owns get_more_tools while report-missing is enabled | Customer `get_more_tools` (description `"customer version"`) is kept, not replaced or duplicated; after 2 listings, exactly 1 log line contains `"shadow"`. Full text: `WARN: This server registers its own "get_more_tools" tool, which is shadowed by AgentCat's report-missing tool: calls to it are answered by AgentCat and never reach your handler. Rename your tool, or set enableReportMissing: false to keep yours reachable.` | port |
| 12 | tools/list fault containment > returns the customer's original listing when the whole pipeline fails | The tools array's iterator throws: the same original object (`nextCursor "p2"`) is returned; log contains `"serving the original tool list"`. | adapt: wrap injection in `catch_unwind`; on panic return the inner listing unchanged. |

Quirks:

- Row 11: the listing keeps the customer's `get_more_tools` schema and description, but every call to it gets AgentCat's canned answer. The tool advertises a handler that never runs.

### src/tests/error-capture.test.ts (12 tests)

Purpose: error events from real tool failures on a v1 `McpServer`.

Harness: the todo fixture per test; `EventCapture` before each test; every test calls
`track(server, {projectId: "test-project", enableTracing: true})` (legacy signature, see quirks);
100 ms waits; `findEventsByResourceName`. Calls use `client.request(..., CallToolResultSchema)`.

| # | Test (describe > it) | Behavior asserted, with exact literal values | Rust |
| --- | --- | --- | --- |
| 1 | Error Capture Integration Tests > should capture stack traces when tool throws Error | `complete_todo {id: "nonexistent-id", context: "Testing error capture"}`: `result.isError === true`. Event: `isError true`; `error.message` contains `"not found"` (full: `Todo with ID nonexistent-id not found`); `error.type "Error"`; `stack` defined; `frames.length > 0`. | adapt: `type` = the Rust error type's short name; `stack`/`frames` only when a backtrace was captured. |
| 2 | ... > should capture Error.cause chains | Tool `error_with_cause` throws `"Wrapper error"` caused by `"Root cause error"`: `type "Error"`; message contains `"Wrapper error"`; `stack` and `frames` defined; `chained_errors.length 1`; `chained_errors[0].message "Root cause error"`. | adapt: chain from `source()`. |
| 3 | ... > should capture TypeError with correct type | Tool `type_error_tool` reads a property of `null`: `type "TypeError"`; message contains `"null"`; `stack` and `frames` defined. | adapt: keep the concrete error type name (use a custom error type). |
| 4 | ... > should capture non-Error thrown values | Tool `throw_string` throws `"This is a string error"`: `result.isError true`; event `error.type` undefined, message contains the string, `stack` and `frames` undefined. | adapt: a tool returning `Err(String)` (rmcp makes an `isError` result): message from the text, no type, no frames. |
| 5 | ... > should detect in_app frames correctly (skipped) | `it.skip`. It would assert at least one frame with `in_app: true` for `complete_todo {id: "bad-id"}`. | adapt (skipped in TS): Rust `in_app` rules from audit 02 §5.7. |
| 6 | ... > should still propagate errors to MCP client | `complete_todo {id: "invalid", context: "Testing error propagation"}`: `isError true`; `content[1].text` contains `"not found"` (`content[0]` is the mint-back). | port |
| 7 | ... > should NOT publish identify events when identify function throws | `identify` throws `"Identify error"`; `list_todos {context: "Testing identify error capture"}`: no event of type `agentcat:identify`; a `mcp:tools/call` event exists. | port |
| 8 | ... > should handle successful tool calls without errors | `add_todo {text: "Test todo", context: "Testing successful tool calls"}`: the last `add_todo` event has `isError` and `error` undefined. | port |
| 9 | ... > should capture validation errors for invalid enum values | Tool `calculate` (`operation: z.enum(["add", "subtract", "multiply", "divide"])`); call `operation: "modulo"`, `a: 10`, `b: 3`. The try/catch accepts either outcome. Event: `isError` true; `error.message` contains `"Invalid"`; `type` in `["McpError", "Error", undefined]`; a present `stack` or `frames` is non-empty. | adapt: rmcp does no schema validation; a serde mismatch becomes an `isError` result with `failed to deserialize parameters: ...`; assert that message instead of `"Invalid"`. |
| 10 | ... > should capture errors for unknown tool names | `nonexistent_tool {context: "Testing unknown tool error"}`: an event with `resourceName "nonexistent_tool"`, `isError`, message containing `"not found"`, `type` in the same set. | adapt: rmcp's router returns `Err(McpError::invalid_params("tool not found"))`; the wrapper sees the call before routing and publishes an error event. |
| 11 | ... > should capture validation errors for missing required parameters | `add_todo {context: "Testing missing parameter"}` without `text`: event `isError`, message contains `"Invalid"`, `type` in the same set. | adapt: serde's `missing field` message inside `failed to deserialize parameters: ...`. |
| 12 | ... > should publish exactly one event per tool call | `list_todos {context: "Testing single event publishing"}`: exactly 1 non-error `list_todos` event. | port |

Quirks:

- Every test calls `track(server, {projectId, enableTracing})`. The object becomes the `projectId`, so events carry an object as `projectId` (audit 01 §15 #16).
- Rows 9, 10 and 11 call `expect.fail(...)` inside the `try`. The `catch` swallows that assertion error, so these tests pass whether or not the request throws.
- Row 5 is skipped, so `in_app` frame detection has no test.
- Row 4: non-Error throws lose `type` and stack because the MCP SDK converts them before AgentCat sees them.

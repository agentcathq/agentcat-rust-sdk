# TS SDK audit 01: architecture, public API, and server interception

Audit of the AgentCat TypeScript SDK for a native Rust port that instruments **Rust MCP servers** (mainly
servers built on the official `rmcp` crate). This document describes behavior and contracts. It is not a
code walkthrough for its own sake: every section ends with "Rust port notes".

- Source audited: `/home/ubuntu/agentcat-typescript-sdk`, commit `9b56e9e`, package `agentcat@2.1.1-beta.1`
  (`package.json:3`).
- Scope of this file: public API, `track()` lifecycle, the interception engine (`src/engine/*`), detection and
  adapters, intent (`context`) injection, `get_more_tools`, handles (`session_id` / `agent_id`), session and
  client identity, `identify` and the other hooks, diagnostics, logging, compatibility, registry, background
  tasks, `extra` projection, tag validation.
- Out of scope here (covered by sibling audit files): the event queue internals, redaction, sanitization,
  truncation, exception parsing, token estimation, and the four exporters. They appear here only where the
  interception layer hands data to them.
- Citations are `file:line` relative to the TS repo root unless a path is absolute.

---

## 0. Read this first: the cross-SDK brief is stale

`/home/ubuntu/2026-07-28-cross-sdk-changelog.md` describes the **2.0.0-beta.4** design. The handle wire
contract changed twice after that brief. The 2.0.0 release (squash commit `0bb144e`) renamed `task_id` to
`conversation_id` and then to `session_id`, and added rejection of values the server did not issue (`invalid`)
and of customer-declared parameters (`foreign`). The 2.1.0 release (commit `1c4f8b1`, "sturdier session
handles") moved the text block to the front, renamed the structured mirror to `mcp_session` with a `status`
enum, made `session_id` schema-required with a `pattern`, and added the `start` sentinel. The Go SDK carries the
2.1.0 change (`/home/ubuntu/agentcat-go-sdk`, commit `7e71df8`). **Port the current TS code, not the brief.**
The differences that matter:

| Concern | Cross-SDK brief (stale) | Current TS code (port this) |
| --- | --- | --- |
| Session handle parameter name | `task_id` | `session_id` (`src/modules/handles.ts:19`) |
| Parameter requiredness | optional; "omission is the minting signal" | added to `required` with a `pattern` (`src/modules/handle-injection.ts:160-166`); omission still mints server-side (`src/modules/handles.ts:289-295`) |
| Supplied value handling | trusted verbatim, no shape check | must match `^ses_[0-9A-Za-z]{27}$`, else source `invalid` and the event is sessionless (`src/modules/handles.ts:73-75,278-288`) |
| Explicit "new task" signal | none | `start` sentinel, case-insensitive, trimmed (`src/modules/constants.ts:32`, `src/modules/handles.ts:279`) |
| Mint-back text position | trailing block | **prepended** as the first content element (`src/modules/handles.ts:128-133`) |
| Mint-back text | `[MCP INSTRUCTIONS]: task_id issued. ...` | `[session_id issued — see this tool's session_id parameter description]` + `session_id: <id>` + body (`src/modules/constants.ts:39-52`) |
| Unrecognized-value feedback | none | `[session_id unrecognized — ...]` block (`src/modules/constants.ts:45-49`) |
| Structured mirror key | `_mcp_instructions` with an `instructions` string | `mcp_session` with `session_id`, `agent_id`, `status` enum (`src/modules/constants.ts:58`, `src/modules/handles.ts:135-179`) |
| Mirror position | appended key | **first** key of `structuredContent` (`src/modules/handles.ts:197-201`) |
| Hook option name | `resolveTaskId` | `resolveSessionId` (`src/types.ts:23-26`) |
| Session-source tag | `agentcat_task_id_source` | `agentcat_session_id_source` (`src/modules/constants.ts:81`) |
| Session sources | `supplied`, `minted`, `hook` | `hook`, `supplied`, `minted`, `invalid`, `foreign` (`src/modules/handles.ts:77-78`) |
| `agent_id` description | two variants (default and hook mode) | one description for both modes (`src/modules/constants.ts:34-35`, `src/modules/handle-injection.ts:178-180`) |
| `CustomEventData` session field | `taskId` | `sessionId` (`src/types.ts:283-284`) |

Other stale documents in the TS repo:

- `MIGRATION.md:12` says `session_id` is "optional", and `MIGRATION.md:21` shows the old `task_id`-era
  description. `MIGRATION.md:33` describes a "trailing `[MCP INSTRUCTIONS]:` text block". `MIGRATION.md:31`
  and `README.md:64` name `_mcp_instructions`. All of these are out of date with the code above.
- `src/index.ts:44` (JSDoc for `enableAgentTracking`) says agent IDs "are minted by the server on an agent's
  first call". The code never mints agent IDs; agents self-choose them (`src/modules/handles.ts:304-313`).
- `docs/superpowers/specs/2026-07-29-explicit-handles-copy.md:7` calls itself the "single source of truth"
  for agent-facing copy. It is not; `src/modules/constants.ts` is, and `src/tests/constants.test.ts` pins it.

---

## 1. Executive summary

1. **One public entry point.** `track(server, projectId, options)` (`src/index.ts:214-304`) plus
   `publishCustomEvent` (`src/index.ts:364-451`). `track()` never throws (`src/index.ts:300-303`), is
   idempotent per server object (`src/index.ts:244-250`), and returns the object the customer passed in.
2. **Only two MCP methods are intercepted: `tools/list` and `tools/call`.** Nothing else (initialize,
   resources, prompts, ping, tasks) is wrapped or published (`src/engine/registrationPatch.ts:31`,
   `src/engine/index.ts:21-24`; out-of-scope note in
   `docs/superpowers/specs/2026-07-31-mcp-sdk-v2-support-design.md:164`). Only `mcp:tools/call` events are
   emitted automatically (`src/engine/callWrap.ts:242`); `agentcat:custom` comes from `publishCustomEvent`.
3. **`tools/list` is a pure transform.** `buildInjectedList()` (`src/engine/listWrap.ts:34-83`) appends
   `get_more_tools`, injects `session_id` / `agent_id` (input schema) and `mcp_session` (output schema), then
   `context`, and records which names it injected per tool. It must stay deterministic, because
   `tools/call` re-runs it to rebuild state on instances that never served a listing
   (`src/engine/callWrap.ts:86-128`).
4. **`tools/call` is the hot path.** Resolve handles, fire hooks without awaiting them, strip injected
   arguments on a clone, dispatch, prepend mint-back text, mirror `mcp_session` into `structuredContent`,
   record the customer's **original** result on the event, publish to a background queue
   (`src/engine/callWrap.ts:147-356`).
5. **Customer hooks never delay a tool call.** `identify`, `eventTags`, `eventProperties` and
   `resolveSessionId` are started at request start and awaited only in the background queue with a 30 s
   budget (`src/engine/callWrap.ts:204-227`, `src/modules/pendingEvent.ts:13,59-101`). There is no identity
   cache.
6. **A "handle" is an analytics correlation ID that the agent echoes back as a tool argument.** It replaces
   protocol sessions that the stateless MCP 2026-07-28 revision removed. `session_id` is server-issued
   (`ses_` KSUID); `agent_id` is self-chosen by the agent and opt-in.
7. **The JS mechanics do not port.** The TS SDK monkey-patches `setRequestHandler`, swaps entries in the
   private `_requestHandlers` map, wraps `_registeredTools` in a `Proxy`, and rewrites `executor` /
   `handler` / `callback` properties in place. Rust needs a wrapper type around the server handler. The
   **contracts** (strings, wire keys, ordering, fail-open rules, determinism) must port byte-for-byte.

Top porting risks (detail in section 14):

- JSON key order is part of the contract (injected properties after the customer's; `mcp_session` first in
  `structuredContent`). `serde_json` sorts keys unless the `preserve_order` feature is on, and Cargo feature
  unification would turn that feature on for the customer's whole build.
- Wrapping a Rust handler trait must forward **every** method, or new trait methods silently fall back to
  defaults.
- Fail-open behavior (no panic, no error ever reaches the client because of AgentCat) needs `catch_unwind`
  around customer hooks and SDK code, and careful ownership so the background pipeline is `Send + 'static`.
- Pagination: the TS registry is replaced per listed page, which breaks stripping for tools on earlier
  pages (section 4.2.4). Decide the Rust behavior on purpose.

---

## 2. Architecture overview

```
customer code ──► track(server, projectId, options)            src/index.ts:214
                    │
                    ├─ initDiagnostics()        (module global, first call wins)   src/modules/diagnostics.ts:206
                    ├─ isCompatibleServerType() → detectServer()                   src/modules/compatibility.ts:39, src/detect.ts:47
                    ├─ eventQueue.configure(apiBaseUrl) (module global, last call wins) src/index.ts:230-234
                    ├─ already tracked? → return                                    src/index.ts:244-250
                    ├─ TelemetryManager (module global, first call wins)            src/index.ts:254-260
                    ├─ setServerTrackingData(lowLevel, {projectId, options})        src/index.ts:269-286
                    └─ installEngine(detection)                                     src/engine/index.ts:16
                          ├─ initEngineState(lowLevel, {adapter, highLevel})
                          ├─ patchSetRequestHandler(lowLevel, rearm)                src/engine/registrationPatch.ts:16
                          ├─ installRegistryProxy(...)   (high-level only)          src/engine/innerTap.ts:94
                          ├─ rearm(): installListWrap + installCallWrap             src/engine/listWrap.ts:90, src/engine/callWrap.ts:136
                          └─ rewrapAllTools(...)         (high-level only)          src/engine/innerTap.ts:73

request time
  tools/list ─► listWrapper ─► original list handler ─► buildInjectedList() ─► registries saved ─► injected list
  tools/call ─► callWrapper ─► ensureRegistries() ─► resolveHandles() ─► hooks fired (not awaited)
                            ─► strip on clone ─► original call handler ─► (SDK validation) ─► innerTap wrapper
                            ─► customer tool fn ─► finish(): mint-back + mirror ─► publishEvent() ─► queue
```

### 2.1 Layers (`CLAUDE.md:42-57`)

| Layer | Files | Role |
| --- | --- | --- |
| Public API | `src/index.ts`, `src/index.workerd.ts`, `src/types.ts` | `track`, `publishCustomEvent`, exported types |
| Detection | `src/detect.ts` | Probes the object in hand; returns `{ major: 1\|2, flavor: "high"\|"low", lowLevel, highLevel, signals }` (`src/detect.ts:6-13`) |
| Adapters | `src/adapters/v1.ts`, `src/adapters/v2.ts` | Name the registry property that dispatch calls: v1 `handler`/`callback`, v2 `executor` |
| Engine | `src/engine/*` | List wrap, call wrap, inner tap, registration re-arm, per-server state |
| Modules | `src/modules/*` | Handles, context injection, `get_more_tools`, session/client identity, hooks, queue, logging, diagnostics, and the event pipeline |

### 2.2 State model

There are two kinds of state (`CLAUDE.md:67`):

**Module-level singletons, first call wins.** Per-request factories call `track()` on every request, so this
state must be created once and reused.

| State | Where | Initialization rule |
| --- | --- | --- |
| Event queue (`eventQueue`) | `src/modules/eventQueue.ts:286` | Created at import. `configure(apiBaseUrl)` replaces its API client on **every** `track()` call that resolves a base URL, even for an already-tracked server (`src/index.ts:230-234` runs before the idempotency check at `src/index.ts:244`). Last caller wins. |
| Telemetry manager | `src/modules/eventQueue.ts:337-346`, `src/index.ts:254-260` | First `track()` call with `exporters` wins; later `exporters` are ignored. |
| Diagnostics | `src/modules/diagnostics.ts:11-12,206-224` | First `track()` call wins (its `projectId` and `disableDiagnostics`). |
| Log target | `src/modules/logging.ts:6-9,66-84` | Resolved on first `writeToLog`. |
| Signal handlers | `src/modules/eventQueue.ts:323-335` | Installed at module import (Node only). |
| Background-task registrar | `src/modules/backgroundTasks.ts:6,14-18` | Set by the workerd entry point at import. |

**Per-server state, keyed by the low-level server object in `WeakMap`s.** Nothing is stored per request.

| Map | Key | Value | Where |
| --- | --- | --- | --- |
| `_serverTracking` | low-level server | `AgentCatData { projectId, options }` | `src/modules/internal.ts:11` |
| `engineStates` | low-level server | `EngineState { adapter, highLevel, originalList, originalCall, listWrapper, callWrapper }` | `src/engine/registry.ts:99-118` |
| `registries` | low-level server | `Map<toolName, Set<injectedParamName>>` | `src/engine/registry.ts:5-21` |
| `outputRegistries` | low-level server | `Set<toolName>` whose `outputSchema` received `mcp_session` | `src/engine/registry.ts:23-38` |
| `reportedConflicts` | `AgentCatData` | tool names already logged for a `session_id` collision | `src/engine/registry.ts:41-50` |
| `declaredSessionParams` | `AgentCatData` | tool names whose own schema declares `session_id` (grow-only) | `src/engine/registry.ts:52-71` |
| `customerOwnedReportMissing` | `AgentCatData` | warn-once marker for a customer tool named `get_more_tools` | `src/engine/registry.ts:79-88` |
| `wrappedFns` | function | marker set for inner-tap idempotency | `src/engine/innerTap.ts:8` |

The TS SDK does **not** use `AsyncLocalStorage` or any ambient request context. Per-request data flows
through closures and the SDK's `extra` argument only (the one mutation is `extra.__agentcat_error`, section
4.4).

### 2.3 Rust port notes (architecture)

- Per-server `WeakMap`s become fields of the Rust wrapper struct (or an `Arc<Inner>` shared by clones of the
  wrapper). Lifetime then follows the server automatically; no weak maps are needed.
- Module-level singletons become `static` `OnceLock`/`LazyLock` values (queue, publisher HTTP client,
  telemetry, diagnostics). Keep "first wins" for telemetry and diagnostics. Decide whether `api_base_url` stays
  "last wins" (TS behavior, probably accidental) or becomes "first wins" like the rest. Recommendation: first
  wins, and document the difference.
- A Rust SDK needs an explicit shutdown/flush API (for example `agentcat::shutdown(timeout).await`) because
  there is no `beforeExit` hook and installing signal handlers in a library is hostile. The Go SDK returns a
  shutdown function from `Track` (`/home/ubuntu/agentcat-go-sdk/officialsdk/officialsdk.go:181,261`).
- Prior art for a statically typed port: the Go SDK uses the official Go MCP SDK's middleware instead of
  monkey-patching (`/home/ubuntu/agentcat-go-sdk/officialsdk/middleware.go:16-48`). Its design is the
  closest analogue to what `rmcp` will need.

---

## 3. Public API surface

### 3.1 Exports

`src/index.ts` exports:

| Export | Kind | Line |
| --- | --- | --- |
| `track` | function | `src/index.ts:468` (defined at `src/index.ts:214`) |
| `publishCustomEvent` | async function | `src/index.ts:364` |
| `AgentCatIDPrefixes` | runtime enum `{ Session = "ses", Event = "evt", Agent = "agt" }` | `src/index.ts:464`, `src/types.ts:87-91` |
| `AgentCatOptions`, `AgentCatData`, `UserIdentity`, `RedactFunction`, `RedactEventFunction`, `ExporterConfig`, `Exporter`, `CustomEventData` | types | `src/index.ts:453-462` |
| `IdentifyFunction` | type alias `AgentCatOptions["identify"]` | `src/index.ts:466` |

`src/index.workerd.ts` (selected by the `workerd` export condition, `package.json` `exports["."].workerd`)
re-exports everything from `index.ts` and installs a background-task registrar that calls Cloudflare's
`waitUntil(task)` (`src/index.workerd.ts:3-8`). This keeps a Worker invocation alive until event delivery
settles (section 11.3).

Not exported (internal only): `Event`, `UnredactedEvent`, `CompatibleRequestHandlerExtra`, `SessionInfo`,
and every module function. `src/modules/index.ts` re-exports some modules, but `src/index.ts` does not
re-export it.

The `AgentCatIDPrefixes.Agent = "agt"` value is reserved; no code mints `agt_` IDs (`src/types.ts:90`).

### 3.2 `track(server, projectId, options)` lifecycle, end to end

Signature: `track(server: any, projectId: string | null, options: AgentCatOptions = {}): any`
(`src/index.ts:214-218`). Everything runs inside one `try`; any throw logs
`Warning: Failed to track server - ${error}` and returns the **original** `server` argument untracked
(`src/index.ts:300-303`).

1. `initDiagnostics({ projectId, disabled: options.disableDiagnostics })` (`src/index.ts:220`). One-shot.
2. `isCompatibleServerType(server)` (`src/index.ts:225`). It runs detection and structural validation and
   throws on failure (section 3.7). The return value is `detection.highLevel ?? detection.lowLevel`
   (`src/modules/compatibility.ts:54`), which is the object the customer passed in.
3. `detectServer(server)` again to get the `Detection` (`src/index.ts:226`).
4. Resolve the API base URL: `options.apiBaseUrl || process.env.AGENTCAT_API_URL || process.env.MCPCAT_API_URL`;
   if any is set, `eventQueue.configure(url)` (`src/index.ts:229-234`). Otherwise the queue keeps its
   default `https://api.agentcat.com` (`src/modules/eventQueue.ts:43`).
5. Log the setup-started beacon:
   `AgentCat setup started | project ${projectId || "(telemetry-only)"} | sdk v${major}/${flavor} | signals ${signals}`
   (`src/index.ts:240-242`). This line feeds diagnostics (section 11.1) and is the fleet change-detection
   channel.
6. If the low-level server already has tracking data, log
   `[SESSION DEBUG] track() - Server already being tracked, skipping initialization` and return
   (`src/index.ts:244-250`). Options from the second call are ignored (except the base URL, step 4).
7. If `options.exporters` is set and no telemetry manager exists yet, create one (`src/index.ts:254-260`).
8. If `projectId` is falsy and there are no exporters, log
   `Warning: No projectId provided and no exporters configured. Events will not be sent anywhere.`
   (`src/index.ts:263-267`). Tracking still installs.
9. Build `AgentCatData` with defaults applied (`src/index.ts:269-284`); `projectId: projectId || ""`.
   `exporters`, `apiBaseUrl` and `disableDiagnostics` are **not** stored (they are global).
10. `setServerTrackingData(lowLevelServer, data)` then `installEngine(detection)` (`src/index.ts:286-287`).
    Tracking data is set first so the wrappers can read it.
11. Log the setup-complete beacon:
    `AgentCat setup complete | project ... | tracing=... context=... reportMissing=... exporters=N`
    (`src/index.ts:292-297`).
12. Return the validated server object (`src/index.ts:299`).

The JSDoc warns "Do not call `track()` multiple times on the same server instance" (`src/index.ts:65`), but
step 6 makes it safe.

### 3.3 `AgentCatOptions` (`src/types.ts:1-44`)

| Option | Type | Default | Behavior | Key lines |
| --- | --- | --- | --- | --- |
| `enableReportMissing` | `boolean` | `true` | Appends the `get_more_tools` tool to listings and answers calls to it without reaching the customer's handler. | `src/index.ts:272`, `src/engine/listWrap.ts:43-49`, `src/engine/callWrap.ts:154-156,334-341` |
| `enableTracing` | `boolean` | `true` | `false`: no `session_id`/`agent_id`/`mcp_session` injection, no resolution, no mint-back, no events; `tools/call` passes through **unstripped**. `context` injection and `get_more_tools` still apply. | `src/index.ts:273`, `src/engine/listWrap.ts:54-62`, `src/engine/callWrap.ts:159-166`, `src/modules/eventQueue.ts:365-367` |
| `enableToolCallContext` | `boolean` | `true` | Injects the required `context` string parameter into every tool except `get_more_tools`; records `arguments.context` as `Event.userIntent`. | `src/index.ts:274`, `src/engine/listWrap.ts:69-75`, `src/engine/callWrap.ts:255-263` |
| `customContextDescription` | `string` | `undefined` | Replaces the default `context` description when truthy (`""` falls back to the default; whitespace is used as is). | `src/index.ts:275`, `src/modules/context-parameters.ts:56-57` |
| `enableAgentTracking` | `boolean` | `false` | Injects a required `agent_id` parameter; a supplied value becomes event tags and is echoed in `mcp_session.agent_id`. | `src/index.ts:276`, `src/engine/listWrap.ts:61-62`, `src/modules/handle-injection.ts:170-188`, `src/modules/handles.ts:307-313` |
| `resolveSessionId` | `(request, extra?) => string \| null \| Promise<string \| null>` | `undefined` | Hook mode: no `session_id` injected, no mint-back text, no session status; session derived deterministically from the returned string and the project ID. | `src/types.ts:16-26`, `src/modules/handles.ts:255,260-267,329-369`, `src/engine/listWrap.ts:60`, `src/modules/pendingEvent.ts:74-84` |
| `identify` | `(request, extra?) => Promise<UserIdentity \| null>` | `undefined` | Runs on every tool call, concurrently with the handler; result stamped on that call's event. | `src/types.ts:27-30`, `src/engine/callWrap.ts:219-221`, `src/modules/internal.ts:69-81`, `src/modules/pendingEvent.ts:86-90` |
| `redactSensitiveInformation` | `(text: string) => Promise<string>` | `undefined` | String-level redaction applied in the queue to every non-protected string. | `src/types.ts:72`, `src/engine/callWrap.ts:244`, `src/modules/eventQueue.ts:130-139` |
| `redactEvent` | `(event: Event) => Event \| null \| Promise<Event \| null>` | `undefined` | Event-level hook; `null` drops the event; a throw drops the event. Runs before string redaction. | `src/types.ts:74-76`, `src/modules/eventQueue.ts:116-128,423` |
| `exporters` | `Record<string, ExporterConfig>` | `undefined` | Telemetry exporters (`otlp`, `datadog`, `sentry`, `posthog`). Module-level, first call wins. | `src/index.ts:254-260`, `src/modules/telemetry.ts:27-44` |
| `apiBaseUrl` | `string` | `https://api.agentcat.com` | Ingestion base URL. Precedence: option, `AGENTCAT_API_URL`, `MCPCAT_API_URL`, default. Module-level, last call wins. | `src/index.ts:229-234`, `src/modules/eventQueue.ts:43-50` |
| `disableDiagnostics` | `boolean` | `false` | Turns off SDK diagnostics shipping. | `src/index.ts:220`, `src/modules/diagnostics.ts:206-224` |
| `eventTags` | `(request, extra?) => Record<string,string> \| null \| Promise<...>` | `undefined` | Customer tags; validated; merged **under** SDK tags. | `src/types.ts:36-39`, `src/modules/internal.ts:30-44`, `src/modules/pendingEvent.ts:94-96` |
| `eventProperties` | `(request, extra?) => Record<string,any> \| null \| Promise<...>` | `undefined` | Free-form JSON properties on the event. | `src/types.ts:40-43`, `src/modules/internal.ts:50-62`, `src/modules/pendingEvent.ts:98-100` |

Hook argument contract: every hook receives the **raw** request (injected `session_id`, `agent_id` and
`context` still present) and the SDK's raw `extra` (not the projected copy). See
`src/engine/callWrap.ts:204-210`. On v1, `extra` is the SDK's `RequestHandlerExtra`; on v2 it is the SDK's
`ServerContext` (`{ sessionId, mcpReq, http }`), where `http.req.headers` is a Web `Headers` object
(`src/types.ts:164-184`).

### 3.4 Other public types

| Type | Shape | Lines |
| --- | --- | --- |
| `UserIdentity` | `{ userId: string; userName?: string; userData?: Record<string, any> }` | `src/types.ts:230-234` |
| `CustomEventData` | `{ sessionId?, resourceName?, parameters?, response?, message?, duration?, isError?, error?, tags?, properties? }` | `src/types.ts:282-294` |
| `ExporterConfig` | `{ type: string; [key: string]: any }` | `src/types.ts:78-81` |
| `Exporter` | `{ export(event: Event): Promise<void> }` | `src/types.ts:83-85` |
| `AgentCatData` | `{ projectId: string; options: AgentCatOptions }` (`""` in telemetry-only mode) | `src/types.ts:249-252` |
| `Event` (internal) | id, sessionId, projectId, eventType, timestamp, duration, inputTokens, outputTokens, ipAddress, sdkLanguage, agentcatVersion, serverName, serverVersion, clientName, clientVersion, identifyActorGivenId, identifyActorName, identifyActorData, resourceName, parameters, response, userIntent, isError, error, tags, properties, actorId, eventId, identifyData | `src/types.ts:93-141` |

### 3.5 `publishCustomEvent(serverOrSessionId, projectId, eventData?)` (`src/index.ts:364-451`)

- Throws `projectId is required for publishCustomEvent` when `projectId` is falsy (`src/index.ts:370-372`).
- **Server form** (first argument is a non-null object): uses `arg.server` if present (high-level), else the
  object itself (`src/index.ts:381-384`). Throws
  `Server is not tracked. Please call agentcat.track() first or provide a session ID string.` if untracked
  (`src/index.ts:385-390`). Session = `eventData.sessionId`, else `""` (published with `session_id: null`)
  and a log line (`src/index.ts:391-399`). Published through `publishEvent()` (`src/index.ts:441-442`), which:
  replaces `projectId` with the **tracked** project ID, not the argument (`src/modules/eventQueue.ts:390`);
  drops the event if `enableTracing` is false (`src/modules/eventQueue.ts:365-367`); stamps server/client
  metadata; and applies `redactEvent` (`src/modules/eventQueue.ts:423`). It does **not** apply
  `redactSensitiveInformation`, because the custom event never gets a `redactionFn`
  (`src/index.ts:410-429`, `src/modules/eventQueue.ts:422`).
- **String form**: the string is the session ID, verbatim, unless `eventData.sessionId` is set
  (`src/index.ts:400-402`). Goes straight to `eventQueue.add()` (`src/index.ts:443-446`): no server metadata,
  no `sdkLanguage`, no redaction hooks.
- Any other first argument throws
  `First parameter must be either an MCP server object or a session ID string` (`src/index.ts:403-407`).
- Event fields: `eventType: "agentcat:custom"` (`src/modules/constants.ts:3`), `message` maps to
  `userIntent`, tags pass `validateTags`, empty `properties` are dropped (`src/index.ts:410-437`).
- The function is `async` but awaits nothing; it resolves once the event is queued.

### 3.6 Environment variables

| Variable | Read at | Effect |
| --- | --- | --- |
| `AGENTCAT_API_URL` | `src/index.ts:232` | API base URL if the option is unset. |
| `MCPCAT_API_URL` | `src/index.ts:233` | Legacy fallback for the base URL. Must keep working permanently (`MIGRATION.md:88`). |
| `DISABLE_DIAGNOSTICS` | `src/modules/diagnostics.ts:190-204` | Empty or unset: no effect. `false`/`0`/`no`/`off` (trimmed, case-insensitive): force-enable, overriding test-environment detection. Any other value: disable. |
| `DIAGNOSTICS_ENDPOINT` | `src/modules/diagnostics.ts:26-35` | Collector base URL (default `https://otel.agentcat.com`); `/v1/logs` appended unless already present. |
| `DIAGNOSTICS_TOKEN` | `src/modules/diagnostics.ts:37-45` | Bearer token (default public key, `src/modules/constants.ts:13-14`). |
| `VITEST`, `JEST_WORKER_ID`, `NODE_ENV=test` | `src/modules/diagnostics.ts:179-188` | Test-environment detection disables diagnostics. |
| `NODE_ENV` | `src/modules/diagnostics.ts:121` | Reported as `deployment.environment` diagnostics attribute. |
| `HOME` (indirectly, `os.homedir()`) | `src/modules/logging.ts:49-54` | Log file location `~/agentcat.log`. |

### 3.7 Supported MCP SDKs, detection, adapters, compatibility

**Support matrix.** `@modelcontextprotocol/sdk >=1.11 <2` ("v1") and `@modelcontextprotocol/server >=2 <3`
("v2"), both optional peers (`package.json` `peerDependencies`, `peerDependenciesMeta`; `CLAUDE.md:9`). Each
major has a high-level flavor (`McpServer`, which wraps a low-level `server` property) and a low-level flavor
(`Server`, where the customer registers `tools/list` and `tools/call` request handlers directly). All four
combinations are supported through the same `track()` call.

**Detection never imports either SDK** (`CLAUDE.md:61`). It probes the object in hand. The probe list is
single-sourced in `collectSignals()` (`src/detect.ts:28-45`):

| Signal | Probe | Meaning |
| --- | --- | --- |
| `hasServerProp` | `s.server` is an object | High-level wrapper |
| `hasTool` | `typeof s.tool === "function"` | v1 high-level (`.tool()` was removed in v2) |
| `hasRegisterTool` | `typeof s.registerTool === "function"` | v2 high-level (v1 `McpServer` also has it, so `.tool` is checked first) |
| `hasSetRequestHandler` | on the low-level object | Required |
| `hasRequestHandlersMap` | `low._requestHandlers instanceof Map` | Required |
| `hasProjectCallToolResult` | `typeof low.projectCallToolResult === "function"` | v2 low-level |
| `hasNegotiatedWireCodec` | `typeof low._negotiatedWireCodec === "function"` | v2 low-level |

Decision (`src/detect.ts:47-77`): missing `setRequestHandler` or `_requestHandlers` means unrecognized.
High-level: `.tool` means v1, else `.registerTool` means v2, else unrecognized. Low-level: either v2 probe means
v2, else v1. `describeSignals()` joins the true signal names with commas (`src/detect.ts:79-84`);
`fingerprintServerShape()` renders the same for unrecognized shapes (`src/detect.ts:92-95`).

**Adapters** hold the one fact that differs per major: which property of a registered-tool entry the SDK's
dispatch calls.

| Adapter | `tapKeys` (first present wins) | Why | Lines |
| --- | --- | --- | --- |
| v1 | `["handler", "callback"]` | SDK 1.24+ uses `handler`, <= 1.23 `callback`; read live at dispatch | `src/adapters/v1.ts:3-10` |
| v2 | `["executor"]` | v2 dispatch calls `tool.executor(args, ctx)`, a closure built at registration; wrapping `handler` is a silent no-op | `src/adapters/v2.ts:3-12` |

**Structural validation** (`src/modules/compatibility.ts:39-103`). After detection succeeds, the low-level
object must have: `setRequestHandler` function, `_requestHandlers` `Map` with `get`, `getClientVersion`
function, and `_serverInfo.name` truthy. Each failure logs
`AgentCat SDK Compatibility: AgentCat supports MCP TypeScript SDK v1.11+ (@modelcontextprotocol/sdk) and v2 (@modelcontextprotocol/server).`
and throws a specific message (`src/modules/compatibility.ts:59-102`). An unrecognized shape logs
`AgentCat SDK Compatibility: unrecognized server shape | signals <fingerprint or (none)>. <matrix>` and throws
`AgentCat SDK compatibility error: server object does not match any supported MCP SDK shape. <matrix>`
(`src/modules/compatibility.ts:43-51`). `track()` catches all of these and returns untracked.

`src/modules/mcp-sdk-compat.ts` holds Zod v3/v4 introspection (`getObjectShape`, `getLiteralValue`,
`src/modules/mcp-sdk-compat.ts:97-155`), used only by the `setRequestHandler` patch to read the method
literal out of a v1 request schema (`src/engine/registrationPatch.ts:25-27`). Its `handler`/`callback` helpers
(`getToolFunction`, `getToolFunctionKey`, `hasToolFunction`, `createWrappedTool`,
`src/modules/mcp-sdk-compat.ts:21-67`) and `isHighLevelServer`/`isLowLevelServer`
(`src/modules/compatibility.ts:24-36`) are not referenced by any non-test source file: dead code for the port.

### 3.8 Rust port notes (public API)

- **Entry point shape.** The idiomatic Rust equivalent of `track(server, projectId, options) -> server` is a
  wrapper constructor: `agentcat::track(handler, project_id, options) -> AgentCat<H>` (or
  `handler.with_agentcat(...)` via an extension trait), where `AgentCat<H>` implements the same `rmcp` trait as
  `H`. Because the type changes, the customer must serve the returned value. Unlike TS, the Rust wrapper
  cannot be "applied in place" after the server starts serving.
- **"Never throws"** becomes "never returns an error and never panics". `track()` should return the wrapper
  directly (not a `Result`). If setup fails, the wrapper should be a pass-through that logs once. A
  `track()` that returns `Result` forces customers to handle errors that the TS contract says cannot happen.
- **Detection and adapters mostly disappear.** There is one Rust SDK (`rmcp`) with one dispatch model, so
  there is no v1/v2 or high/low flavor detection to port. What does port is the *posture*: compile-time
  support for a range of `rmcp` versions (Cargo feature flags or a version range), a CI matrix across `rmcp`
  minors, and a startup log line that fingerprints what was detected (rmcp version, which handler traits,
  whether a tool router is present). Keep the start/complete beacon pair for diagnostics.
- **High-level vs low-level in rmcp terms.** The likely analogues are: "high-level" = a handler using
  `#[tool_router]` / `ToolRouter` with `Parameters<T>` typed arguments and schemars schemas; "low-level" = a
  handler that implements `list_tools` / `call_tool` by hand. A wrapper at the handler level covers both, so
  the port does not need two code paths. Confirm with the rmcp research.
- **Hook signatures.** Rust hooks will be async closures or trait objects, for example
  `Arc<dyn Fn(&CallToolRequestParam, &RequestContext<RoleServer>) -> BoxFuture<'static, Option<UserIdentity>> + Send + Sync>`.
  The background pipeline needs owned, `'static` data, so hooks should receive owned clones (or the hook must
  return a `'static` future). This is an API design question to settle early.
- **`extra` equivalent.** rmcp's per-request context (`RequestContext<RoleServer>`: request id, peer,
  cancellation token, `meta`, `extensions`) is the analogue of `extra`. For streamable HTTP, HTTP request parts
  (headers, URI) are expected in `extensions`; this is what `identify` needs for auth headers. Confirm the
  exact location with the rmcp research.
- **Types to port.** `UserIdentity`, `CustomEventData`, the `Event` struct, `ExporterConfig` (as an enum),
  `AgentCatIdPrefix`. Keep `publish_custom_event` with both forms (tracked server or session-id string); in
  Rust these are two functions or an enum argument.
- **Behavior to keep or fix on purpose.** The TS custom-event server form ignores its `projectId` argument and
  skips string redaction (section 3.5). Decide whether Rust copies that or fixes it, and record the decision in
  the cross-SDK brief.
- **Environment variables** port unchanged: `AGENTCAT_API_URL`, `MCPCAT_API_URL`, `DISABLE_DIAGNOSTICS`,
  `DIAGNOSTICS_ENDPOINT`, `DIAGNOSTICS_TOKEN`. Rust test detection: `cfg!(test)` only covers the SDK's own
  unit tests; no standard environment variable marks a customer's `cargo test` run. Decide whether Rust keeps
  test-environment auto-disable at all.

---

## 4. The interception engine (`src/engine/*`)

### 4.1 Install order (`src/engine/index.ts:16-34`)

1. Pick the adapter from `detection.major` and store `EngineState { adapter, highLevel }` keyed by the
   low-level server (`src/engine/index.ts:18-19`).
2. Patch `setRequestHandler` so a later `tools/list` or `tools/call` registration re-arms the wraps
   (`src/engine/index.ts:26`).
3. High-level only: replace `_registeredTools` with a `Proxy` whose `set` trap wraps new tool entries and
   re-arms (`src/engine/index.ts:27-29`).
4. `rearm()` = `installListWrap(server)` then `installCallWrap(server)` (`src/engine/index.ts:21-24,30`).
5. High-level only: sweep every registered tool through the inner tap (`src/engine/index.ts:31-33`).

Interception happens at the **map seam**: the engine reads and replaces entries in the low-level server's
private `_requestHandlers: Map<method, handler>` (`src/engine/listWrap.ts:93-96,141`,
`src/engine/callWrap.ts:139-142,359`). The stored original handler keeps the SDK's own request validation
(v1 Zod parse, v2 codec), so AgentCat sees raw requests and calls through the SDK's normal path
(`docs/superpowers/specs/2026-07-31-mcp-sdk-v2-support-design.md:63-65`).

### 4.2 `tools/list` wrap (`src/engine/listWrap.ts`)

#### 4.2.1 Installing the wrapper (`src/engine/listWrap.ts:90-142`)

- No `tools/list` handler yet: do nothing; the registration patch or the registry proxy re-arms later
  (`src/engine/listWrap.ts:94-95`). High-level servers register their `tools/list` handler lazily, on the
  first tool registration.
- The current handler is already our wrapper: do nothing (`src/engine/listWrap.ts:96`).
- Otherwise remember the current handler as `originalList` and install a new wrapper
  (`src/engine/listWrap.ts:98-99,140-141`). A customer re-registration after `track()` therefore gets
  wrapped again, and the new handler becomes the "original".

#### 4.2.2 The wrapper, per request (`src/engine/listWrap.ts:101-138`)

1. Await the original handler. Its errors propagate unchanged (`src/engine/listWrap.ts:102`).
2. No tracking data: log
   `Warning: AgentCat is unable to find server tracking data. Please ensure you have called track(server, options) before using tool calls.`
   and return the original response (`src/engine/listWrap.ts:103-109`).
3. `tools` missing, not an array, or empty: log
   `Warning: No tools found in the original list. This is likely due to the tools not being registered before AgentCat.track().`
   and return the original response; registries stay **unset** (`src/engine/listWrap.ts:110-116`). So a
   server with zero tools does not advertise `get_more_tools`.
4. Run `buildInjectedList(data, tools)`, store both registries, run the inner-tap sweep (high-level), and
   return `{ ...originalResponse, tools: injected.tools }` (`src/engine/listWrap.ts:121-131`). The spread
   keeps `nextCursor`, result `_meta`, and future fields.
5. Any throw in step 4: log
   `Warning: AgentCat tool-list injection failed; serving the original tool list unmodified - ${error}`
   and return the original response (`src/engine/listWrap.ts:132-137`).

No event is published for `tools/list` (it was removed in 2.0; `MIGRATION.md:60`).

#### 4.2.3 `buildInjectedList(data, tools)`: the injection pipeline (`src/engine/listWrap.ts:34-83`)

Pure and deterministic: (config, listed tools) in; (advertised tools, injected-params registry,
output-injection registry) out (`CLAUDE.md:62`). The only side effects are log lines and the grow-only
`declaredSessionParams` / `reportedConflicts` sets.

1. Shallow-copy the tool array (`src/engine/listWrap.ts:38`).
2. If `enableReportMissing`: append the `get_more_tools` descriptor unless a tool with that name is already
   in this list; if it is, record the shadowing (warn once) (`src/engine/listWrap.ts:43-49`). It is appended
   **before** injection so it receives handle parameters.
3. Handle injection (section 7.2) with `injectSessionId = tracingEnabled && !resolveSessionId` and
   `injectAgentId = tracingEnabled && enableAgentTracking === true` (`src/engine/listWrap.ts:54-68`).
   `tracingEnabled` is `enableTracing !== false`.
4. If `enableToolCallContext`: context injection (section 5) (`src/engine/listWrap.ts:69-75`). Runs after
   handles, so the property order is `{ ...customer, session_id, agent_id, context }`.
5. Every listed tool that received no injection still gets an **empty** registry entry
   (`src/engine/listWrap.ts:76-81`). The empty entry means "seen, nothing to strip". The heuristic strip
   fallback applies only to tools absent from the registry.

#### 4.2.4 Pagination caveat (not handled in TS)

`buildInjectedList` runs on each page independently, and each page **replaces** the stored registries
(`src/engine/listWrap.ts:122-124`). Consequences on a paginated low-level server:

- `get_more_tools` is appended to every page that does not already contain it (`src/engine/listWrap.ts:44-47`).
- After page 2 is listed, a call to a page-1 tool finds a registry without that tool's entry. Stripping falls
  back to the heuristic (all three names) (`src/modules/handle-injection.ts:273-281`), and the
  `mcp_session` mirror is skipped because the output registry lacks the tool (`src/engine/callWrap.ts:302-303`).
- Rebuild-on-demand lists only the first page (`params: {}`, `src/engine/callWrap.ts:97`).

v1 and v2 `McpServer` do not paginate `tools/list`, so only hand-written low-level servers hit this. No test
covers pagination.

### 4.3 `tools/call` wrap (`src/engine/callWrap.ts`)

The call wrap is the outermost layer and runs exactly once per request: resolution, event build, publish and
mint-back all live here (`src/engine/callWrap.ts:130-135`). Installation mirrors the list wrap
(`src/engine/callWrap.ts:136-146,358-359`).

#### 4.3.1 Per-request sequence (`src/engine/callWrap.ts:147-356`)

1. Read tracking data. Define `interceptReportMissing(name)` = `enableReportMissing === true && name === "get_more_tools"`
   (`src/engine/callWrap.ts:148-156`).
2. **Tracing off** (`enableTracing === false`): answer `get_more_tools` if intercepted, else call the
   original handler with the **raw, unstripped** request. No resolution, no mint-back, no event
   (`src/engine/callWrap.ts:158-166`).
3. `startTime = new Date()` (`src/engine/callWrap.ts:168`). Event duration therefore includes the rebuild in
   step 5.
4. No tracking data: log the "unable to find server tracking data" warning and take the degraded path
   (`src/engine/callWrap.ts:176-179`).
5. `await ensureRegistries(server, extra)` (`src/engine/callWrap.ts:182`), section 4.3.2.
6. `sessionParamIsOurs = !(toolName && declaredSessionParams(data).has(toolName))`
   (`src/engine/callWrap.ts:183-193`). A tool never seen in a listing counts as "ours".
7. `resolveHandles(options, projectId || undefined, request, extra, sessionParamIsOurs)`
   (`src/engine/callWrap.ts:195-201`), section 7.3. Synchronous; reads only the arguments.
8. `getClientInfoForRequest(server, request, extra)` (`src/engine/callWrap.ts:202`), section 8.1.
9. Fire customer hooks **now**, without awaiting: `resolveSessionId` (hook mode only), `identify`,
   `eventTags`, `eventProperties`. Each yields a non-rejecting promise stored on `event.pending`
   (`src/engine/callWrap.ts:204-227,246`). Hooks receive the raw request.
10. Build the event (`src/engine/callWrap.ts:229-245`):
    - `sessionId: resolution.sessionId` (`""` for hook, invalid, foreign).
    - `resourceName: request.params.name || "Unknown Tool"`.
    - `parameters: { request /* raw, handles included */, extra: projectExtraForEvent(extra) }`.
    - `inputTokens: estimateInputTokens(request.params.arguments)` (raw arguments, before redaction).
    - `eventType: "mcp:tools/call"`, `timestamp: startTime`, `redactionFn: options.redactSensitiveInformation`.
11. On-path tags = MRTR continuation tag (if `extra.mcpReq.inputResponses` is an object) plus handle tags
    (`src/engine/callWrap.ts:248-253`). Customer tags merge under these later.
12. `userIntent = arguments.context` when `enableToolCallContext`, the tool is not `get_more_tools`, the
    arguments are an object, and the `context` key is present (`src/engine/callWrap.ts:255-263`). This also
    captures a customer-owned `context` value on a tool where injection was skipped (no registry check).
13. Any throw in steps 5 to 12: log
    `Warning: AgentCat tracing failed for tool ${name}, falling back to original handler - ${error}` and take
    the degraded path (`src/engine/callWrap.ts:266-270`).
14. **Degraded path**: answer `get_more_tools` if intercepted, else call the original handler with the raw
    request. No event, no mint-back (`src/engine/callWrap.ts:273-281`).
15. **Report-missing**: `userIntent = arguments.context` (regardless of `enableToolCallContext`), answer with
    the canned result, then `finish()` it (mint-back, mirror, event) (`src/engine/callWrap.ts:334-341`). The
    customer's handler is never called.
16. Otherwise call the original handler with `cloneRequestWithStrippedArguments(request, registry)`
    (`src/engine/callWrap.ts:342-348`) and `finish()` the result.
17. `finish(result)` (`src/engine/callWrap.ts:285-331`):
    - `resultType === "input_required"` (MRTR intermediate round): add tag `agentcat_mrtr: "input_required"`;
      no decoration (`src/engine/callWrap.ts:291-294`).
    - Else prepend the mint-back text when `buildMintBackText()` returns text, then mirror `mcp_session` when
      the output registry is missing or contains the tool (`src/engine/callWrap.ts:295-307`).
    - `isError === true`: `event.isError = true`; `event.error = captureException(extra.__agentcat_error)`
      when the inner tap stashed a real error (then delete it), else `captureException(result)`
      (`src/engine/callWrap.ts:308-317`).
    - `event.response = result` (the customer's **original** result), `outputTokens`, `duration`, then
      `publishEvent(server, event, { clientInfo })` and return the decorated result
      (`src/engine/callWrap.ts:318-324`).
    - Any throw: log
      `Warning: AgentCat post-handler processing failed for tool ${name}; returning the customer's result untouched - ${error}`
      and return the customer's raw result (`src/engine/callWrap.ts:325-330`). The event is lost if the throw
      happened before `publishEvent`.
18. **Handler throws**: `isError`, `error = captureException(error)`, `duration`, publish, then **re-throw the
    same error** (`src/engine/callWrap.ts:349-355`). No `response`, no mint-back.

Outcome table:

| Situation | Customer handler receives | Client receives | Event |
| --- | --- | --- | --- |
| `enableTracing: false` | raw request | original result | none |
| No tracking data, or setup throw (degraded) | raw request | original result | none |
| `get_more_tools` with `enableReportMissing` (traced) | not called | canned text with mint-back | one `mcp:tools/call` event, `userIntent = context` |
| `get_more_tools` with `enableReportMissing` (tracing off or degraded) | not called | canned text, no mint-back | none |
| Normal result | stripped clone | decorated result | event with original result |
| `resultType: "input_required"` | stripped clone | untouched result | event tagged `agentcat_mrtr=input_required` |
| Handler throws | stripped clone | same throw | event with `isError`, `error`, no `response` |
| Decoration throws | stripped clone | raw customer result | usually none |

#### 4.3.2 Registry rebuild-on-demand (`src/engine/callWrap.ts:71-128`)

Per-request factories (`createMcpHandler`, `serveStdio`) build a fresh server per request or connection, so a
`tools/call` can arrive at an instance that never served `tools/list`.

- Skip if a registry already exists, or if there is no tracking data or no stored original list handler
  (`src/engine/callWrap.ts:90-93`).
- Call `originalList({ method: "tools/list", params: {} }, extra)` with the **call's** `extra`, raced
  against a 5 000 ms timer (`REGISTRY_REBUILD_TIMEOUT_MS`, `src/engine/callWrap.ts:77,96-114`). The
  abandoned promise gets a no-op `catch` so it never becomes an unhandled rejection
  (`src/engine/callWrap.ts:99-100`).
- Run the result through `buildInjectedList`, store both registries, log
  `Rebuilt injection registries on demand (tools/call before tools/list on this instance)`
  (`src/engine/callWrap.ts:115-122`). No event; no inner-tap sweep.
- On failure or timeout log `Warning: registry rebuild-on-demand failed - ${error}`; the call continues with
  the heuristic strip; the next call retries (`src/engine/callWrap.ts:123-127`). A hanging list handler costs
  up to 5 s per call on that instance.
- Because the pipeline is deterministic, the rebuilt registries match what any listing instance advertised
  (`/home/ubuntu/2026-07-28-cross-sdk-changelog.md:314-322`, still valid).

#### 4.3.3 MRTR helpers

- `isInputRequiredShape(result)`: non-null, non-array object with `resultType === "input_required"`
  (`src/engine/callWrap.ts:53-61`).
- `mrtrContinuationTags(extra)`: `{ agentcat_mrtr: "continuation" }` when `extra.mcpReq.inputResponses` is an
  object, else `{}` (`src/engine/callWrap.ts:63-69`). If a continuation round again returns
  `input_required`, the `input_required` value overwrites `continuation` (`src/engine/callWrap.ts:294`).

### 4.4 Inner tap (`src/engine/innerTap.ts`)

High-level servers only. It wraps the function that the SDK's dispatch actually calls, **in place** on the
registry entry (`src/engine/innerTap.ts:10-21`; `CLAUDE.md:63`). In place matters because v2's
`update()`/`enable()`/`disable()` mutate the original entry; a copy would diverge from what `tools/list`
serves.

`wrapToolEntry(tool, name, server, adapter)` (`src/engine/innerTap.ts:22-70`):

- Walk `adapter.tapKeys`; the first key holding a function wins. If that function is already in the
  `wrappedFns` WeakSet (an original we wrapped, or one of our wrappers), stop: idempotent
  (`src/engine/innerTap.ts:29-32,65-66`).
- The wrapper preserves call arity: two or more parameters means `(args, extra)` (v1 with schema, v2 always,
  even schema-less tools where `args` is `undefined`); one parameter means v1's no-schema `cb(extra)`
  (`src/engine/innerTap.ts:34-48`).
- It strips injected arguments again using the current registry (`src/engine/innerTap.ts:49-53`). Normally a
  no-op because the call wrap already stripped; it matters on the tracing-off and degraded paths (where the
  call wrap passes the raw request) and for permissive validators.
- On a thrown `Error`, it stores it on `extra.__agentcat_error` and re-throws (`src/engine/innerTap.ts:57-63`).
  The SDK converts the throw into an `isError` result; the call wrap then publishes the real stack
  (`src/engine/callWrap.ts:310-313`).

`rewrapAllTools` sweeps every entry (`src/engine/innerTap.ts:73-87`); it runs at install and on every
listing (`src/engine/listWrap.ts:126-128`). This catches v2 `update()` regenerating `executor`. Accepted gap:
an `update()` between a listing and the next call dispatches one round unwrapped
(`docs/superpowers/specs/2026-07-31-mcp-sdk-v2-support-design.md:94`).

`installRegistryProxy` (`src/engine/innerTap.ts:94-129`) replaces `_registeredTools` with a `Proxy`. Its `set`
trap wraps the new entry, performs the set, then calls `rearm()`. This covers v2's lazy
`setToolRequestHandlers` on the first `registerTool`. Errors fall back to a plain `Reflect.set`.

### 4.5 `setRequestHandler` patch (`src/engine/registrationPatch.ts:16-39`)

- Reads the method from the first argument: a string (v2, 2- or 3-argument forms) or a v1 Zod request schema
  whose `method` literal is introspected (`src/engine/registrationPatch.ts:21-28`).
- Calls the original with **all** arguments forwarded verbatim (`src/engine/registrationPatch.ts:29`).
  Dropping the third argument of a v2 custom registration broke customer servers ("handler is required").
- After registration, for `tools/call` or `tools/list` only, calls `rearm()`; a throw logs
  `Warning: engine re-arm failed - ${error}` and registration still succeeds
  (`src/engine/registrationPatch.ts:30-37`).

### 4.6 Re-entrancy and double-wrap guards

| Guard | Mechanism | Lines |
| --- | --- | --- |
| `track()` twice on one server | tracking-data `WeakMap` check; second call returns early | `src/index.ts:244-250` |
| List/call wrap re-arm | `current === st.listWrapper` / `st.callWrapper` | `src/engine/listWrap.ts:96`, `src/engine/callWrap.ts:142` |
| Inner-tap re-sweep | `wrappedFns` WeakSet holds originals and wrappers | `src/engine/innerTap.ts:8,32,65-66` |
| Exactly one mint-back per response | mint-back only in the outer call wrap; inner tap never decorates | `src/engine/callWrap.ts:130-135`; test `src/tests/handles-integration.test.ts:600` |
| Exactly one event per call | only `finish()` or the throw path publishes, once | test `src/tests/error-capture.test.ts:623`, `src/tests/v2/options.test.ts:186` |
| Collision logs | `reportedConflicts` (per tool, per `AgentCatData`); `customerOwnedReportMissing` (once) | `src/engine/registry.ts:41-50,79-88` |
| Unhandled rejections | hook promises built non-rejecting; rebuild promise gets a no-op catch | `src/modules/handles.ts:329-346`, `src/engine/callWrap.ts:99-100` |

There is no per-request marker. Idempotency relies on per-object state, which is enough in TS because each
method has exactly one `_requestHandlers` entry to wrap. A Rust wrapper type can be nested, so Rust needs its
own guard (section 4.8).

### 4.7 What is not intercepted

Only `tools/list` and `tools/call` (`src/engine/registrationPatch.ts:31`, `src/engine/index.ts:21-24`).
`initialize`, `ping`, `resources/*`, `prompts/*`, `completion/*`, `logging/*`, `tasks/*` and notifications pass
through untouched and publish nothing. Tracking prompts and resources is explicitly out of scope
(`docs/superpowers/specs/2026-07-31-mcp-sdk-v2-support-design.md:164`). The ingestion API still defines event
types for them (`node_modules/agentcat-api/dist/models/PublishEventRequest.js:25-44`), and the PostHog
exporter maps them (`src/modules/exporters/posthog.ts:330-336`), but the SDK never emits them.

### 4.8 Rust port notes (engine)

**What is JS-specific and does not port:** the `_requestHandlers` map swap, the `setRequestHandler`
monkey-patch, the `_registeredTools` `Proxy`, in-place rewriting of `executor` / `handler` / `callback`,
arity sniffing, `WeakMap`/`WeakSet` state, and the `extra.__agentcat_error` side channel. All of these exist
because TS has to attach to an already-built object it does not own.

**Likely Rust design: a wrapper type that owns the customer's handler.** Candidate seams, to confirm with the
rmcp research:

| Seam | Shape | Pros | Cons |
| --- | --- | --- | --- |
| A. `Service<RoleServer>` wrapper | `struct AgentCat<S>(S)` with `impl<S: Service<RoleServer>> Service<RoleServer>`; match `ClientRequest::ListToolsRequest` / `CallToolRequest` in `handle_request`, forward everything else | Closest to the TS map seam: one choke point for every request, works for hand-written and macro-built handlers, sees raw params and `_meta` | Works with protocol enums; must re-wrap results into `ServerResult` |
| B. `ServerHandler` wrapper | `impl<H: ServerHandler> ServerHandler for AgentCat<H>`; override `list_tools` / `call_tool`, delegate all others | Typed parameters and results | Must forward **every** trait method by hand; a method added to `ServerHandler` in a later rmcp release silently uses the default instead of the customer's implementation |
| C. `ToolRouter` / route wrapping | wrap each route's handler | Analogue of the inner tap | Only covers `#[tool_router]` servers; cannot change the listing; misses hand-written `call_tool` |
| D. HTTP tower layer | middleware on the streamable-HTTP service | Transport-level | No stdio coverage; must parse and re-serialize JSON-RPC bodies; not recommended |

Recommendation: A or B as the single interception point, with C unnecessary. The TS inner tap's two jobs both
disappear in Rust: stripping happens before the inner handler deserializes `Parameters<T>`, and error capture
reads the returned `Result` (there is no exception to stash).

**Behaviors the Rust engine must keep:**

- Late registration needs no re-arm: a wrapper sees every `tools/list` and `tools/call` request, so tools
  added or removed at runtime are handled on the next listing. Port the *tests* (tool added after `track()` is
  listed with injected parameters and stripped on call), not the mechanism.
- Rebuild-on-demand stays. In rmcp, per-request or per-session handler construction (for example a stateless
  streamable-HTTP service factory) gives the same "call before list" topology. Implement as: on `call_tool`
  with no registry, call the **inner** `list_tools(None, ctx.clone())` under `tokio::time::timeout(5s)`, run
  the same pure pipeline, cache. On failure use the heuristic strip.
- Registries become fields behind `RwLock`/`ArcSwap` inside the wrapper (or `Arc<Inner>` shared by clones,
  because rmcp may clone the handler per session). Decide whether clones share registries; sharing is fine
  because the pipeline is deterministic.
- `track()` idempotency in Rust: wrapping twice produces `AgentCat<AgentCat<H>>` and would publish two events
  and prepend two mint-back blocks. Options: a request-extension marker set by the outer wrapper so inner
  wrappers pass through, or a `track()` signature that rejects an already-wrapped type through a sealed
  marker trait. The TS guarantee to preserve is "one event and one mint-back block per call"
  (`src/tests/v2/options.test.ts:186`).
- Fail-open: every AgentCat step around the call must be infallible from the client's point of view. In Rust
  that means no `?` on AgentCat errors in the request path and `catch_unwind` (`FutureExt::catch_unwind`)
  around AgentCat code and customer hooks. Open question: should a panic in the **customer's** tool still
  propagate exactly as without AgentCat? (TS re-throws the customer's error unchanged.) Recommendation: yes,
  record an error event and `resume_unwind`.
- Error capture: a returned `Err(ErrorData)` is the analogue of the TS throw path (publish `isError`, return
  the same error). A returned `CallToolResult { is_error: Some(true) }` is the analogue of an `isError`
  result. Rust errors carry no stack by default; capture `Display` plus the `source()` chain (TS caps the
  chain at 10, sibling audit) and a `Backtrace` only when the error type provides one.
- MRTR: check whether rmcp models the 2026-07-28 `input_required` result type and input responses. If it
  does, port both tags and the "no decoration on intermediate rounds" rule; if not, leave a TODO and a test.
- Pagination: decide on purpose (section 4.2.4). Recommendation: merge per-page results into the registry
  instead of replacing it, and append `get_more_tools` only to the last page (`next_cursor` is `None`).

---

## 5. Intent capture: the injected `context` parameter

"Intent" is the agent's one-sentence explanation of why it made a tool call. AgentCat gets it by adding a
required `context` string parameter to every tool's input schema and recording the value as
`Event.userIntent` (`README.md:61`, `CLAUDE.md:78`).

### 5.1 Schema transform (`src/modules/context-parameters.ts`)

`addContextParameterToTools(tools, customContextDescription, registry)` (`src/modules/context-parameters.ts:99-126`):

- Skips any tool named `get_more_tools` by name (it has its own bespoke `context`)
  (`src/modules/context-parameters.ts:105-108`).
- Per tool, inside `try`; a throw lists that tool unmodified, removes `context` from its registry entry, and
  logs `WARN: Context injection failed for tool "<name>"; listing it unmodified - <error>`
  (`src/modules/context-parameters.ts:109-124`).

`addContextParameterToTool(tool, desc, registry)` (`src/modules/context-parameters.ts:18-97`), in order:

1. Shallow-copy the tool (`src/modules/context-parameters.ts:24`).
2. `inputSchema.properties.context` already exists (truthy): keep the customer's property unchanged, log
   `WARN: Tool "<name>" already has 'context' parameter. Skipping context injection.`, return
   (`src/modules/context-parameters.ts:29-34`). Not recorded in the registry, so it is never stripped.
3. Root `oneOf`/`allOf`/`anyOf`: log
   `WARN: Tool "<name>" has complex schema (oneOf/allOf/anyOf). Skipping context injection.` and return
   (`src/modules/context-parameters.ts:36-42`).
4. No `inputSchema`: use `{ type: "object", properties: {}, required: [] }`
   (`src/modules/context-parameters.ts:48-54`). A schema of `{}` is *not* given `type: "object"`.
5. Description = `customContextDescription || DEFAULT_CONTEXT_PARAMETER_DESCRIPTION`
   (`src/modules/context-parameters.ts:56-57`). Empty string falls back; whitespace-only is used verbatim (no
   trim).
6. Deep-copy `inputSchema` via JSON round trip; create `properties` if missing; delete
   `additionalProperties` only when it is exactly `false` (`src/modules/context-parameters.ts:59-73`).
7. Add `properties.context = { type: "string", description }` (`src/modules/context-parameters.ts:75-79`).
8. Record `context` in the registry for this tool (`src/modules/context-parameters.ts:81-85`).
9. Append `"context"` to `required` (create the array if missing; no duplicates)
   (`src/modules/context-parameters.ts:87-94`). **`context` is required** in the advertised schema, but the
   server never enforces it: a call without `context` still succeeds and simply has no `userIntent`
   (test `src/tests/context-parameters.test.ts:245`).

Because injection runs on copies of what the original list handler returned, re-running it over an
already-injected listing is a no-op through the collision rule (test
`src/tests/context-parameters.test.ts:313-350`).

### 5.2 Call-time behavior

- Stripped before dispatch when recorded for the tool (`src/modules/handle-injection.ts:266-285`), and kept in
  the raw request recorded on the event (`src/engine/callWrap.ts:232-238`).
- `Event.userIntent = arguments.context` under the conditions in section 4.3.1 step 12
  (`src/engine/callWrap.ts:255-263`). For `get_more_tools`, `userIntent` is its `context` argument whatever
  `enableToolCallContext` says (`src/engine/callWrap.ts:334-335`).
- Customer-owned `context` (collision): not stripped (it reaches the handler), but its value still becomes
  `userIntent` because the capture has no registry check. Untested; decide in Rust.
- With `enableTracing: false` the `context` parameter is still injected (`src/engine/listWrap.ts:69`) but the
  call wrap passes the raw request through (`src/engine/callWrap.ts:165`). High-level servers strip it in the
  inner tap; low-level servers deliver `context` to the customer's handler. Recommendation for Rust: always
  strip what was injected.

### 5.3 Rust port notes (intent)

- `rmcp` tool schemas are JSON objects (`Tool.input_schema` is expected to be an `Arc<JsonObject>`); generated
  by `schemars` for `Parameters<T>`. Clone the map before editing. **Never mutate the shared `Arc` in place**:
  if the customer's router kept the injected property, the next listing would see `session_id`/`context` as
  customer-declared, flip the tool to "foreign", and stop stripping (section 7.2).
- `schemars` emits `additionalProperties: false` for `#[serde(deny_unknown_fields)]` structs. Remove it as TS
  does, and strip before the inner handler deserializes, or `deny_unknown_fields` will reject the call.
- Decide how to treat a root `$ref` (schemars can emit `$ref` + `$defs` for some types): TS only checks
  `oneOf`/`allOf`/`anyOf`, so a root `$ref` schema would get properties added next to the `$ref`. Treat it like
  a composed schema (skip and log) unless rmcp guarantees inline roots.
- Keep `custom_context_description` semantics: `Some("")` falls back to the default (TS `||`); decide and test
  whitespace-only.
- Key order (customer properties, then `session_id`, `agent_id`, `context`) needs an insertion-ordered map
  (section 14.2).

---

## 6. `get_more_tools` (report-missing)

### 6.1 Descriptor (`src/modules/tools.ts:5-31`)

```json
{
  "name": "get_more_tools",
  "description": "Check for additional tools whenever your task might benefit from specialized capabilities - even if existing tools could work as a fallback.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "context": {
        "type": "string",
        "description": "A description of your goal and what kind of tool would help accomplish it."
      }
    },
    "required": ["context"]
  },
  "annotations": {
    "title": "Get More Tools",
    "readOnlyHint": true,
    "destructiveHint": false,
    "idempotentHint": true,
    "openWorldHint": false
  }
}
```

After handle injection the advertised schema becomes properties `context, session_id[, agent_id]` and
required `["context", "session_id"[, "agent_id"]]` (test `src/tests/handle-injection.test.ts:235`). The
`context` injector skips it, so its bespoke `context` keeps its own description even with
`customContextDescription` set (test `src/tests/v2/options.test.ts:141-145`).

### 6.2 Behavior

- **Listing**: appended last, when `enableReportMissing` is true and the page does not already contain a tool
  with that name (`src/engine/listWrap.ts:43-49`). Still listed when `enableTracing` is false (handles are not
  injected then). Not listed when the server lists zero tools (`src/engine/listWrap.ts:110-116`).
- **Not exempt from handles**: receives `session_id` and (if enabled) `agent_id`, because its calls publish
  events and must carry the task (`src/modules/handle-injection.ts:73-80`).
- **Call**: answered by name inside the call wrap; the customer's handler (if any) never runs
  (`src/engine/callWrap.ts:154-156,334-341`). Response:
  `{ content: [{ type: "text", text: "Unfortunately, we have shown you the full tool list. We have noted your feedback and will work to improve the tool list in the future." }] }`
  (`src/modules/tools.ts:38-45`). On the traced path it goes through `finish()`, so a mint-back block may be
  prepended and an `mcp:tools/call` event is published with `resourceName: "get_more_tools"` and
  `userIntent = arguments.context`. On the tracing-off and degraded paths it is answered with no event
  (`src/engine/callWrap.ts:160-164,275-279`).
- **Logging**: `Missing tool reported (context length: N)`; the length only, never the text, because all log
  lines ship to diagnostics (`src/modules/tools.ts:34-36`; test `src/tests/diagnostics-no-payload.test.ts:118-142`).
- **Strip fallback** for `get_more_tools` removes only `session_id` and `agent_id`, never `context`
  (`src/modules/handle-injection.ts:276-280`).
- **Shadowing**: if the customer registers their own `get_more_tools` while the feature is on, their
  descriptor stays in the listing, AgentCat still answers calls, and a warning is logged once
  (`src/engine/registry.ts:79-88`):
  `WARN: This server registers its own "get_more_tools" tool, which is shadowed by AgentCat's report-missing tool: calls to it are answered by AgentCat and never reach your handler. Rename your tool, or set enableReportMissing: false to keep yours reachable.`
  With the feature off, the name is never intercepted on any path (`src/engine/callWrap.ts:149-156`).

### 6.3 Rust port notes (`get_more_tools`)

- Keep the TS approach (synthesize the descriptor in the listing, intercept by name in the call path). It
  works for any handler and needs no access to the customer's router. The Go SDK instead registers a real tool
  (`/home/ubuntu/agentcat-go-sdk/officialsdk/get_more_tools.go:13`); Rust cannot do that generically for an
  arbitrary `ServerHandler`.
- Build the descriptor as JSON (or the rmcp `Tool` struct with `ToolAnnotations`) and compare against the JSON
  above in a golden test. rmcp's `Tool` may also carry a top-level `title`; TS sets only
  `annotations.title`. Do not add fields the TS SDK does not send.
- Decide the pagination rule (append on the last page only, section 4.2.4).

---

## 7. Handles: `session_id` and `agent_id`

### 7.1 What a handle is

A **handle** is a short opaque ID that the server gives the agent (or that the agent chooses), which the agent
then copies into every later tool call as an ordinary tool argument. AgentCat uses handles to group tool calls
without any protocol-level session. The MCP 2026-07-28 revision removed the `initialize` handshake and the
`Mcp-Session-Id` header, so the transport no longer says which calls belong together; SEP-2567 recommends
explicit handles instead (`README.md:27-29`; `MIGRATION.md:1-6`).

| Handle | Who creates it | Scope | Default | Where it lands on the event |
| --- | --- | --- | --- | --- |
| `session_id` | the server mints a `ses_` KSUID; or the customer's `resolveSessionId` hook derives one | one task (goal), shared with subagents | on (when tracing is on and no hook) | `Event.sessionId` (wire `session_id`), same `ses_` prefix as the old sessions (`MIGRATION.md:5`) |
| `agent_id` | the agent itself, formatted `model\|harness\|suffix` (for example `opus-4.80-1m\|claude-code\|k3n9x`) | one agent; never inherited by subagents | off (`enableAgentTracking`) | tags `agentcat_agent_id` and `agentcat_agent_id_source` |

The server stores nothing between requests. Each call resolves its handles from its own arguments
(`src/modules/handles.ts:232-235`).

### 7.2 Schema injection (`src/modules/handle-injection.ts`)

`addHandleParametersToTools(tools, opts, registry, outputRegistry)` returns the input unchanged when neither
handle is injected (`src/modules/handle-injection.ts:87`); otherwise per tool, inside `try`, with rollback of
that tool's registry entries and the log
`WARN: Handle injection failed for tool "<name>"; listing it unmodified - <error>` on failure
(`src/modules/handle-injection.ts:88-103`).

Per tool (`src/modules/handle-injection.ts:106-194`):

1. Shallow-copy the tool.
2. Root `oneOf`/`allOf`/`anyOf`: if `session_id` injection is on and the root `properties.session_id` exists,
   record the tool as customer-declared; log
   `WARN: Tool "<name>" has complex schema (oneOf/allOf/anyOf). Skipping handle injection.`; return. No
   output-schema injection either (`src/modules/handle-injection.ts:116-129`). Such a tool still mints and
   announces sessions at call time (it is "ours") even though it never advertised `session_id` (test
   `src/tests/handles-integration.test.ts:379`), and its empty registry entry means an echoed `session_id`
   is **not** stripped.
3. Missing `inputSchema` becomes `{ type: "object", properties: {}, required: [] }`; deep-copy; create
   `properties`; delete `additionalProperties` when exactly `false` (`src/modules/handle-injection.ts:131-141`).
4. `session_id` (when `injectSessionId`):
   - Customer already declares it: record in `declaredSessionParams`, log once per tool (see the exact
     `ERROR:` text in section 12), do not inject (`src/modules/handle-injection.ts:145-158`).
   - Else add `{ type: "string", description: SESSION_ID_PARAM_DESCRIPTION, pattern: "^(start|ses_[0-9A-Za-z]{27})$" }`,
     record it, append to `required` (`src/modules/handle-injection.ts:159-167`).
5. `agent_id` (when `injectAgentId`):
   - Customer already declares it: log
     `WARN: Tool "<name>" already has 'agent_id' parameter. Skipping agent_id injection.`; not recorded, so
     never stripped (`src/modules/handle-injection.ts:171-174`). Resolution still reads `agent_id` from the
     arguments when tracking is on (there is no "foreign" state for `agent_id`), so the customer's value would
     become the agent tag. Untested quirk.
   - Else add `{ type: "string", description: AGENT_ID_PARAM_DESCRIPTION }` (no pattern), record, append to
     `required` (`src/modules/handle-injection.ts:175-187`).
6. Output schema (`addMcpSessionToOutputSchema`, `src/modules/handle-injection.ts:208-259`), only when the
   tool declares an `outputSchema`:
   - Composed (`oneOf`/`allOf`/`anyOf`): log
     `WARN: Tool "<name>" has complex outputSchema (oneOf/allOf/anyOf). Skipping mcp_session injection; mint-back stays content-only for this tool.`
     and skip.
   - Deep-copy; create `properties`; if `properties.mcp_session` already exists log
     `WARN: Tool "<name>" already declares 'mcp_session' in outputSchema. Skipping injection.` and skip.
   - Add `properties.mcp_session = { type: "object", description, properties: subProperties }` where
     `subProperties` is, in this order: `session_id` (if injecting session), `agent_id` (if injecting agent),
     `status: { type: "string", enum: ["issued", "active", "unrecognized"], description }` (if injecting
     session). Description is `MCP_SESSION_FIELD_DESCRIPTION`, or the hook-mode variant when `session_id` is
     not injected. `mcp_session` is **not** added to `required`; the customer's `additionalProperties: false`
     on the output schema is kept.
   - Record the tool in the output-injection registry.

Requiredness is soft: `required` drives schema-aware clients only; the server tolerates omission
(`src/modules/handle-injection.ts:43-50`). Final order: customer properties, `session_id`, `agent_id`,
`context`; `required` = customer entries, `session_id`, `agent_id`, `context` (test
`src/tests/handles-integration.test.ts:192`).

### 7.3 Per-call resolution (`resolveHandles`, `src/modules/handles.ts:245-316`)

`extractHandle(args, name)` returns the trimmed value only for a non-empty string; anything else counts as
omitted (`src/modules/handles.ts:60-66`). `isValidSessionId(v)` is `/^ses_[0-9A-Za-z]{27}$/`, shape only, no
KSUID decoding (`src/modules/handles.ts:73-75`).

Conditions are checked in this order:

| # | Condition | `sessionId` on the call path | `sessionSource` | Text block | `mcp_session` payload |
| --- | --- | --- | --- | --- | --- |
| 1 | `resolveSessionId` configured (hook mode) | `""` (finalized in the background, 7.6) | `hook` (final: `hook` or `minted`) | none, ever | `{ agent_id }` if supplied, else nothing |
| 2 | Tool's own schema declares `session_id` | `""` | `foreign` | none | `{ agent_id }` if supplied, else nothing |
| 3 | Supplied, not `start`, valid shape | the value | `supplied` | none | `{ session_id, agent_id?, status: "active" }` |
| 4 | Supplied, not `start`, invalid shape | `""` | `invalid` | unrecognized block | `{ agent_id?, status: "unrecognized" }` |
| 5 | Absent, empty, whitespace, non-string, or `start` (trimmed, case-insensitive) | new random `ses_` KSUID | `minted` | issued block | `{ session_id, agent_id?, status: "issued" }` |

Sources: `src/modules/handles.ts:260-296` (rows), `src/modules/handles.ts:100-115` (text),
`src/modules/handles.ts:161-179` (payload). Notes:

- The `start` sentinel is compared after trimming and lowercasing, and only on row 3 to 5's path; a
  customer-owned `session_id` of `start` is the customer's value (`src/modules/handles.ts:275-279`). The
  schema `pattern` is case-sensitive; case-insensitivity lives only in resolution.
- An invalid value is never adopted, because `Event.sessionId` is exempt from both redaction hooks; an agent
  could put a secret there (`src/modules/handles.ts:283-285`). The raw value still appears in
  `parameters.request` on the event, which *is* redacted.
- Omission keeps minting so stale schemas and scripted callers never error (`src/modules/handles.ts:290-292`).
- `agent_id`: when `enableAgentTracking === true` and a non-empty string is supplied, `agentId` = trimmed
  value, `agentSource = "supplied"`. Never minted, never announced in text (`src/modules/handles.ts:304-313`).

### 7.4 Mint-back text block

`buildMintBackText(res)` (`src/modules/handles.ts:100-115`): `null` in hook mode; issued block for `minted`;
unrecognized block for `invalid`; otherwise `null`. Exact bytes (lines joined with `\n`, em dashes U+2014):

```text
[session_id issued — see this tool's session_id parameter description]
session_id: ses_XXXXXXXXXXXXXXXXXXXXXXXXXXX
This is the first-call issuance described in this tool's session_id parameter description.
```

```text
[session_id unrecognized — see this tool's session_id parameter description]
The value sent was not issued by this server. Re-send the session_id issued earlier for this task; if none was issued yet, send start and one will be issued.
```

`appendMintBack(result, text)` (despite its name) **prepends** `{ type: "text", text }` as `content[0]` on a
shallow copy, so the ID survives client-side truncation of long results. It requires an array `content`;
anything else is returned untouched. It applies to `isError` results too (`src/modules/handles.ts:117-133`).

### 7.5 Structured mirror (`mcp_session`)

- `buildStructuredMintBack(res)` (`src/modules/handles.ts:161-179`): key order `session_id`, `agent_id`,
  `status`; `null` when empty. Suppression is per handle (a foreign tool can still echo `agent_id`).
- `mirrorStructuredMintBack(result, mint)` (`src/modules/handles.ts:191-202`): only when `structuredContent`
  is a plain object (not null, not an array, not a string); if `mcp_session` is already a key it is customer
  data and wins; otherwise a shallow copy with `mcp_session` as the **first** key. Never creates
  `structuredContent`.
- Gate (`src/engine/callWrap.ts:298-306`): mirror when the output-injection registry contains the tool, or
  when no output registry exists at all (rebuild failed; the client cannot be validating against a schema we
  know about). A tool without a declared `outputSchema` that still returns `structuredContent` gets no mirror
  while a registry exists.
- Unlike the text block, the mirror appears on **every** completed response with something to report, so an
  agent can re-read its handles mid-conversation (`status: "active"`).
- Not on MRTR intermediate rounds (`src/engine/callWrap.ts:291-294`). Wire-only: `event.response` is the
  original result (`src/engine/callWrap.ts:318-319`).

### 7.6 Hook mode (`resolveSessionId`) and deterministic derivation

- Call path: provisional `sessionId: ""`, source `hook`, arguments never read (`src/modules/handles.ts:260-267`).
  No `session_id` parameter anywhere; `mcp_session` declares only `agent_id` (hook-mode description).
- The hook is fired at request start via `invokeSessionHook` (`src/modules/handles.ts:329-346`): synchronous
  throw, rejection or nullish all become `null`, logging `resolveSessionId hook error: <err>`.
- Background stage 0 (`src/modules/pendingEvent.ts:74-84`): `sessionFromHookValue(value, event.projectId || undefined)`
  (`src/modules/handles.ts:358-369`): non-empty after trim gives `deriveSessionId(value.trim(), projectId)`
  with source `hook`; otherwise a new random ID with source `minted` (silent: no parameter exists, so the agent
  could never echo it). The tag `agentcat_session_id_source` is overwritten with the final source.
- `deriveSessionId(id, projectId?)` (`src/modules/handles.ts:35-49`), **must match byte-for-byte across SDKs**:

```text
input   = projectId (non-empty) ? id + ":" + projectId : id        # no trimming inside; caller trims
hash    = SHA-256(UTF-8(input))                                     # 32 bytes
ts_ms   = 1704067200000 + (u32_be(hash[0..4]) % 31536000000)        # 2024-01-01T00:00:00Z + offset
payload = hash[4..20]                                               # 16 bytes
ksuid   = u32_be(floor((ts_ms - 1400000000000) / 1000)) ++ payload  # 20 bytes
result  = "ses_" + base62(ksuid) left-padded with "0" to 27 chars
```

  Note: `u32 % 31_536_000_000` never wraps (u32 max is 4 294 967 295), so the offset is in practice at most
  about 49.7 days, not one year. Port the formula literally.

  Golden vectors (pinned in `src/tests/handles.test.ts:59-75`; independently re-derived with a Python
  re-implementation during this audit, all four match):

  | `id` | `projectId` | Result |
  | --- | --- | --- |
  | `customer-abc` | `proj_1` | `ses_2cOHEO0LYGADMzRvWTXXVbbgxgm` |
  | `customer-abc` | (none) | `ses_2cZY3tvyI25O2AmL2CGVo2B1IIj` |
  | ` x ` | `p` | `ses_2c3yR5mYKQdLaXsJNgZH6erbfQK` |
  | `x` | `p` | `ses_2bw285VY9apdgUgTPXKFnT6P4G0` |

### 7.7 Handle tags (`buildHandleTags`, `src/modules/handles.ts:212-230`)

| Tag | Value | When |
| --- | --- | --- |
| `agentcat_session_id_source` | `supplied` / `minted` / `invalid` / `foreign` / `hook` | always (hook mode finalizes it in the background) |
| `agentcat_agent_id` | supplied `agent_id`, each `\r` or `\n` replaced by a space, truncated to 200 (UTF-16 units) | tracking on and supplied |
| `agentcat_agent_id_source` | `supplied` | same |
| `agentcat_protocol_version` | for example `2026-07-28` | request carries a protocol version (section 8.1) |
| `agentcat_mrtr` | `input_required` / `continuation` | MRTR rounds (`src/engine/callWrap.ts:63-69,294`) |

SDK tags bypass `validateTags`, redaction and truncation, are exempt from the 50-tag cap, and win collisions
with customer tags (`src/modules/pendingEvent.ts:94-96`).

### 7.8 Argument stripping (`src/modules/handle-injection.ts:266-301`)

- Strip exactly the names recorded for the tool. No entry for the tool (or no registry): strip
  `session_id`, `agent_id`, `context`, except `get_more_tools` keeps `context`.
- Works on a shallow copy of `arguments` inside a cloned request; the original request (kept on the event) is
  untouched; non-object arguments pass through.
- Fallback caveat (documented in `MIGRATION.md:37`): if the rebuild fails, a customer's own `session_id`,
  `agent_id` or `context` is removed before their handler sees it.

### 7.9 KSUID (`src/thirdparty/ksuid/`)

- Vendored Segment-compatible KSUID: 20 bytes = 4-byte big-endian seconds since epoch `1400000000000` ms plus
  16 random bytes; base62 alphabet `0-9A-Za-z`; 27 characters left-padded with `0`
  (`src/thirdparty/ksuid/index.js:11-28,44-50,88-94`, `src/thirdparty/ksuid/base62.js:4-10`).
- `withPrefix(p)` returns `p + "_" + string` (`src/thirdparty/ksuid/index.js:184-199`).
- Session IDs: `KSUID.withPrefix("ses").randomSync()` (`src/modules/handles.ts:22-24`). Event IDs:
  `await KSUID.withPrefix("evt").random()` assigned last in the queue (`src/modules/eventQueue.ts:155`).
- `fromParts` validates the time range and a 16-byte payload (`src/thirdparty/ksuid/index.js:134-150`).

### 7.10 Rust port notes (handles)

- Port the **current** contract (section 0). Treat `src/modules/constants.ts` and the tests as the source of
  truth for strings, and copy the golden vectors into Rust tests.
- KSUID: either hand-roll (about 60 lines: big-endian bytes, base62 with the `0-9A-Za-z` alphabet, left pad to
  27) or use a crate, but in both cases pin the four golden vectors plus the KSUID MIN/MAX round-trip
  (`000000000000000000000000000` / `aWgEPTl1tmebfsQzFP4bxwgy80V`). Random payload from a CSPRNG (`rand::rngs::OsRng`
  or `getrandom`).
- Validation must be shape-only (`^ses_[0-9A-Za-z]{27}$`); do not parse as KSUID, because test IDs such as
  `ses_` + 27 × `a` exceed the KSUID maximum but must be accepted (`src/tests/constants.test.ts:91`).
- Trimming: JS `trim()` strips U+FEFF but not U+0085; Rust `str::trim()` strips U+0085 but not U+FEFF. Pick
  JS-compatible trimming (`trim_matches` over the ECMAScript whitespace set) for cross-SDK parity of derived
  IDs and sentinel handling, and add a test for both characters.
- Length limits: TS counts UTF-16 code units (`slice(0, 200)` on the agent tag). Use
  `encode_utf16().count()` semantics, or document the deviation.
- `structuredContent` in rmcp is likely `Option<serde_json::Value>`: mirror only into `Value::Object`; insert
  `mcp_session` first (needs an ordered map, section 14.2).
- Hook mode: the Rust `resolve_session_id` hook must run off the request path (section 9.3); derivation runs in
  the background task, after the hook settles.

---

## 8. Sessions, protocol eras, and client identity (`src/modules/session.ts`)

### 8.1 Stateful vs stateless (2026-07-28) protocol

AgentCat 2.x keeps **no** session state on the server in either protocol era:

- MCP `extra.sessionId` (the transport session, `Mcp-Session-Id`) is ignored entirely, and inactivity-based
  session rollover is gone (`MIGRATION.md:65`; `src/types.ts:176-182`).
- The analytics session is always the explicit handle (section 7). A 2025-era stateful client and a
  2026-07-28 stateless client get identical handle behavior.
- What differs per era is only **where per-request client identity and protocol version come from**, and the
  serving topology (per-request server instances, which drives rebuild-on-demand, section 4.3.2).
- `AgentCatData` holds configuration only, `{ projectId, options }` (`src/types.ts:249-252`); tests assert the
  tracking data keys are exactly `["options", "projectId"]` (`src/tests/identify.test.ts:100-104`).

Session-ID derivation and formats are in section 7.6 and 7.9. On the wire, `Event.sessionId` `""` is sent as
`session_id: null` (`src/modules/eventQueue.ts:174`).

### 8.2 Client name and version, per request, never cached (`src/modules/session.ts:31-55`)

Resolution ladder, first hit wins:

1. `extra.mcpReq.envelope["io.modelcontextprotocol/clientInfo"]`: the v2 SDK lifts reserved
   `io.modelcontextprotocol/*` keys out of `_meta` before dispatch and exposes them on the envelope under their
   fully-qualified names.
2. `request.params._meta["io.modelcontextprotocol/clientInfo"]`: v1 passes the key through.
3. `server.getClientVersion()`: legacy `initialize` capture (backfilled by v2's `createMcpHandler`;
   `undefined` on 2026-pinned stdio). Returned without narrowing.

`narrowClientInfo` (`src/modules/session.ts:14-29`) accepts a candidate only if it is an object with at least
one string field (`name` or `version`); otherwise the ladder moves on. Each field is narrowed separately: a
non-string field becomes `undefined`.

The resolved `clientName`/`clientVersion` are passed to `publishEvent` as `context.clientInfo`
(`src/engine/callWrap.ts:202,323,353`); `publishEvent` falls back to `getClientVersion()` if missing
(`src/modules/eventQueue.ts:372-376`).

### 8.3 Protocol version (`src/modules/session.ts:57-65`)

`extra.mcpReq.envelope["io.modelcontextprotocol/protocolVersion"]`, then
`request.params._meta["io.modelcontextprotocol/protocolVersion"]`; must be a non-empty string. Stamped as tag
`agentcat_protocol_version` (`src/modules/handles.ts:228`).

### 8.4 Per-event metadata (`buildSessionInfo`, `src/modules/session.ts:71-88`)

| Event field | Value |
| --- | --- |
| `ipAddress` | always `undefined` |
| `sdkLanguage` | `"TypeScript"` (Go sends `"Go"`, `/home/ubuntu/agentcat-go-sdk/officialsdk/middleware.go:375`) |
| `agentcatVersion` | package version |
| `serverName` / `serverVersion` | `server._serverInfo.name` / `.version` |
| `clientName` / `clientVersion` | section 8.2 |
| `identifyActorGivenId` / `identifyActorName` / `identifyActorData` | at publish time `undefined` / `undefined` / `{}`; replaced in the queue when `identify` resolves (section 9) |

### 8.5 Rust port notes (sessions and client identity)

- No session store in Rust either. Ignore rmcp's transport session ID for analytics.
- Server name/version: from the inner handler's `get_info()` (`ServerInfo.server_info`), read once at
  `track()` or per event.
- Client identity ladder in rmcp terms (to confirm with the rmcp research): (1) the request's `_meta`
  (`RequestContext.meta` or the request params' meta) under `io.modelcontextprotocol/clientInfo`; (2) the
  peer's `initialize` info (`context.peer.peer_info()`), the analogue of `getClientVersion()`. The TS
  "envelope" rung exists only because the TS v2 SDK lifts reserved keys out of `_meta`; check whether rmcp does
  the same for 2026-07-28 requests and, if so, read from wherever it puts them first.
- Parse leniently: read `_meta` values as `serde_json::Value` and take `as_str()` per field; never fail the
  call on a malformed `_meta`.
- `sdk_language`: use `"Rust"` on events and `"rust"` in diagnostics attributes (matching the Go/Python
  pattern: `/home/ubuntu/agentcat-go-sdk/internal/diagnostics/attributes.go:38`). Confirm the ingestion
  backend accepts a new value.

---

## 9. `identify` and the other customer hooks

### 9.1 Flow

1. At request start, after resolution, the call wrap starts each configured hook and stores the promise on
   `event.pending` (`src/engine/callWrap.ts:211-227,246`):
   `pending.sessionHookValue` (hook mode), `pending.identity`, `pending.tags`, `pending.properties`.
2. Each promise is built so it never rejects: `resolveIdentity` / `resolveEventTags` /
   `resolveEventProperties` catch everything and return `null` (`src/modules/internal.ts:30-81`);
   `invokeSessionHook` contains synchronous throws (`src/modules/handles.ts:329-346`).
3. The tool handler runs at the same time. Event `duration` covers the handler only (tests assert a 100 ms
   `identify` does not appear in the duration, `src/tests/identify.test.ts:622-685`).
4. Queue stage 0 (`src/modules/eventQueue.ts:104-114`): detach `pending` from the event **first**, then
   `applyPendingEventFields` (`src/modules/pendingEvent.ts:59-101`) awaits all four concurrently, each raced
   against `PENDING_HOOK_TIMEOUT_MS = 30_000` (`src/modules/pendingEvent.ts:13,23-48`). Timeout logs
   `Warning: <label> hook did not settle within 30000ms; publishing event without it` (labels
   `resolveSessionId`, `identify`, `eventTags`, `eventProperties`). Worst case adds one budget, not four.
5. Apply results (`src/modules/pendingEvent.ts:74-100`): hook session and source; identity fields
   (`identifyActorGivenId = userId`, `identifyActorName = userName`, `identifyActorData = userData || {}`);
   customer tags merged **under** SDK tags (`{ ...customerTags, ...event.tags }`); properties replace.
6. Then `redactEvent`, string redaction, sanitize, truncate, id, send (sibling audit). `redactEvent` therefore
   sees resolved identity, tags and properties.

### 9.2 Contract

- Runs on every `tools/call`, including `get_more_tools`, and including tools registered after `track()`.
  Never on `tools/list` or `initialize`. No cache; no `agentcat:identify` event (`MIGRATION.md:64`;
  `src/modules/internal.ts:64-68`).
- `identify` returning `null`, throwing (logged `Error: identify callback threw - <err>`), or timing out gives
  an anonymous event; the tool call is unaffected.
- `eventTags` result goes through `validateTags` (section 10.2); a throw logs `eventTags callback error: <err>`;
  `eventProperties` throw logs `eventProperties callback error: <err>` (`src/modules/internal.ts:41,59`).
- An `eventProperties` result of `{}` is sent as `{}` on tool calls (only `publishCustomEvent` drops empty
  properties, `src/index.ts:435`).
- Hooks receive the raw request (injected parameters visible) and raw `extra`.
- Consequence documented for customers: hooks no longer complete before the handler, so `identify` must not
  be used to prime state the handler reads (`MIGRATION.md:64`).
- Tracing off, missing tracking data, or a degraded path: no hook runs (`src/engine/callWrap.ts:159-180`).

### 9.3 Rust port notes (hooks)

- **JS promises start eagerly; Rust futures are lazy.** To keep "hooks run concurrently with the handler and
  never delay it", the Rust SDK must `tokio::spawn` each hook future at request start and hand the
  `JoinHandle`s to the event. Polling them only in the background task would start them late; joining them
  with the handler would delay the response.
- This forces hook futures to be `Send + 'static`: hooks get owned data (cloned request params, a cloned
  `RequestContext` or a projected subset). Settle the signature early.
- Background stage: `tokio::time::timeout(Duration::from_secs(30), handle)` per hook, all awaited
  concurrently (`join!`), and `abort()` on timeout so a hung hook does not leak a task.
- Contain panics: a panicking hook is a `JoinError`; treat it as `None` and log.
- The TS identify signature is `async`-only (`Promise<UserIdentity | null>`); `eventTags`, `eventProperties`
  and `resolveSessionId` accept sync or async. In Rust a single async signature is simpler.

---

## 10. `extra` projection and tag validation

### 10.1 `projectExtraForEvent` (`src/modules/extra-projection.ts:21-70`)

Captured at request time into `event.parameters.extra`, so the redaction, sanitization and truncation walkers
(own enumerable keys only) can traverse it and `redactEvent` sees exactly what ships.

- v2: `extra.http.req` is a WHATWG `Request`; replace it with `{ method?, url?, headers }` where
  `headers = Object.fromEntries(req.headers.entries())` (names lowercased by `Headers`; duplicate values
  comma-joined) (`src/modules/extra-projection.ts:27-49`).
- v1: `extra.requestInfo.url` (a `URL`) becomes its `href` string; headers already plain
  (`src/modules/extra-projection.ts:51-61`).
- Duck-typed, idempotent, shallow copies only the rewritten levels, returns the original `extra` when nothing
  needs rewriting, never throws (logs `Warning: Failed to project request context into event parameters, publishing extra as-is - <err>`)
  (`src/modules/extra-projection.ts:15-20,62-69`).
- Everything else in `extra` passes by reference (for v2: `sessionId`, `mcpReq` with `id`, envelope,
  `inputResponses`; `http.authInfo` including the bearer token). The event therefore records auth tokens and
  all request headers unless the customer's redaction removes them. That is a privacy-relevant default to
  restate in the Rust docs.

### 10.2 `validateTags` (`src/modules/validation.ts:13-78`)

Customer tags only (SDK tags bypass it). Per entry, in order: key matches `^[a-zA-Z0-9$_.:\- ]+$`; key length
<= 32; value is a string; value length <= 200; value has no `\n` (only `\n`, not `\r`). Invalid entries are
dropped with a log line each. Then keep the first 50 valid entries in insertion order (log
`Dropping N tag(s) — exceeds maximum of 50 entries per event`). Empty input or nothing valid returns `null`.
Lengths are UTF-16 code units.

### 10.3 Rust port notes

- The `extra` analogue is rmcp's `RequestContext`: project a JSON object with `id`, `meta`, and, for HTTP, the
  `http::request::Parts` from `extensions` as `{ method, url, headers }`. `http::HeaderMap` names are already
  lowercase; join repeated values with `", "` to match `Headers.entries()`; convert non-UTF-8 values lossily.
- The exact shape of `parameters.extra` is not a cross-SDK contract (each SDK projects its own SDK's context),
  but `parameters.request` (the raw JSON-RPC params) should match: `{ method: "tools/call", params: { name, arguments, _meta? } }`.
- Tag validation: implement the regex by hand (ASCII set), count value length in UTF-16 units for parity,
  preserve insertion order (`IndexMap`).

---

## 11. Diagnostics, logging, compatibility, background tasks

### 11.1 Diagnostics (`src/modules/diagnostics.ts`)

- **Feed**: every `writeToLog` line is teed to the diagnostics sink (`src/modules/logging.ts:92-97`,
  `src/modules/diagnostics.ts:220`). So **every log line ships**; logs must never contain tool arguments,
  responses, identity or `get_more_tools` text (tests `src/tests/diagnostics-no-payload.test.ts:51-142`).
- **Enablement** (`src/modules/diagnostics.ts:179-224`): one-shot on the first `track()`; enabled unless
  `disableDiagnostics: true`, or `DISABLE_DIAGNOSTICS` set to anything except `false`/`0`/`no`/`off`, or a test
  environment (`VITEST`, `JEST_WORKER_ID`, `NODE_ENV=test`) without the explicit force-enable. The option beats
  the env force-enable.
- **Transport**: OTLP/HTTP JSON logs, `POST <DIAGNOSTICS_ENDPOINT or https://otel.agentcat.com>/v1/logs`,
  `Content-Type: application/json`, `Authorization: Bearer <DIAGNOSTICS_TOKEN or dgk_sdk_diag_3f9a2c7e1b8d4065af2e9c1d7b6a4f80>`
  (`src/modules/diagnostics.ts:26-45,237-274`; `src/modules/constants.ts:6-14`). Scope name
  `agentcat-diagnostics`, version = SDK version. Fire-and-forget, no retries.
- **Batching**: buffer max 1000 records (drop oldest), one flush timer at 2000 ms (unref'd), flush on
  `SIGINT`/`SIGTERM`/`beforeExit` (`src/modules/diagnostics.ts:21-22,47-59,226-235`; `src/modules/eventQueue.ts:313,327-331`).
- **Severity**: `/fail|error/i` gives ERROR (17), else the case-sensitive substring `Warning:` gives WARN (13),
  else INFO (9) (`src/modules/diagnostics.ts:140-144`). So `WARN:`-prefixed lines classify as INFO unless they
  contain "fail" or "error", and the `ERROR:` collision log classifies as ERROR.
- **Resource attributes**: `agentcat.project_id` (or `agentcat.install_id` = first 16 hex of
  `sha256(hostname + "|" + module URL)` when no project), `agentcat.sdk.language = "typescript"`,
  `agentcat.sdk.version`, `agentcat.mcp_sdk.version`, `agentcat.mcp_sdk_v2.version`,
  `process.runtime.name`, `process.runtime.version`, `process.pid`, `os.type`, `os.version`, `host.arch`,
  `host.cpu.count`, `deployment.environment` (`src/modules/diagnostics.ts:65-126`). Record attributes repeat
  the version fields (`src/modules/diagnostics.ts:146-162`).
- **Beacons** that matter for fleet monitoring: the setup started/complete pair (section 3.2) and the
  unrecognized-shape fingerprint (section 3.7).

### 11.2 Logging (`src/modules/logging.ts`)

- Never to stdout on Node: stdout is the JSON-RPC wire for stdio servers (`CLAUDE.md:68`). Target resolution
  (`src/modules/logging.ts:35-61`): file `~/agentcat.log` when `os.homedir()` resolves; console (prefix
  `[agentcat] `) only when `fs`/`os` cannot be loaded (edge isolates); otherwise silent (the diagnostics sink
  still receives every entry).
- Line format: `[<ISO-8601 UTC>] [sdk=<ver> node=<ver>( mcp=<v1>)?( mcp2=<v2>)?] <message>`; `mcp=unknown`
  when neither MCP package resolves; the prefix is computed once (`src/modules/logging.ts:86-118`;
  `src/modules/runtime-versions.ts:104-121`). Append-only, synchronous, errors swallowed.

### 11.3 Background tasks (`src/modules/backgroundTasks.ts`)

A runtime hook for serverless lifetimes: `registerBackgroundTask(delivery)` is called for each queued event
that has a project ID (`src/modules/eventQueue.ts:75-79`); on Cloudflare Workers the registrar is
`waitUntil` (`src/index.workerd.ts:6`), so the isolate stays alive until delivery settles. Failures are logged,
never thrown (`src/modules/backgroundTasks.ts:25-35`).

### 11.4 Compatibility helpers

Covered in section 3.7. `getMCPCompatibleErrorMessage` (`src/modules/compatibility.ts:105-118`) serializes
errors for log lines (Error to JSON of own properties; string as is; object JSON; else `Unknown error`).

### 11.5 Rust port notes (diagnostics, logging)

- Keep diagnostics wire-compatible (OTLP JSON logs, same endpoint, token, scope name, attribute keys), with
  `agentcat.sdk.language = "rust"` and `agentcat.mcp_sdk.version` = the rmcp version. Getting a dependency's
  version at compile time needs a build script (`cargo metadata`, or a crate such as `built`); otherwise
  report the rmcp version range the crate was compiled against.
- Logging: a Rust library normally logs through `tracing` and lets the application choose the sink, but an
  application that logs to stdout would corrupt a stdio server. Options: (a) keep the TS behavior (own file
  sink `~/agentcat.log` plus the diagnostics tee), (b) emit through `tracing` with target `agentcat` and keep
  only the diagnostics tee internal. Recommendation: (a) for parity, with an option to also emit `tracing`
  events. Either way, the diagnostics tee must see every SDK log line, and no SDK log line may contain payloads.
- Test-environment detection has no Rust equivalent of `VITEST`/`JEST_WORKER_ID`; `cfg!(test)` only covers the
  crate's own unit tests. Keep `DISABLE_DIAGNOSTICS` and the option; consider dropping auto-detection.
- Background tasks: native Rust servers do not need `waitUntil`. A Workers target (`workers-rs`) would need a
  `ctx.wait_until` hook; out of scope unless rmcp runs on Workers.
- Shutdown: replace signal handlers with an explicit async `shutdown()` that drains the queue (5 s budget,
  `src/modules/eventQueue.ts:261-283`) and flushes diagnostics. Do not install signal handlers in the library.

---

## 12. Strings and wire keys a port must copy byte-for-byte

`src/modules/constants.ts` is the single source of truth for agent-facing copy; `src/tests/constants.test.ts:61-95`
pins several strings exactly and others by substring. The long strings below were extracted mechanically
from the source file (not retyped). Note the em dashes (U+2014) in several of them.

### 12.1 Names, keys, enums

| Item | Exact value | Source |
| --- | --- | --- |
| Session handle parameter | `session_id` | `src/modules/handles.ts:19` |
| Agent handle parameter | `agent_id` | `src/modules/handles.ts:20` |
| Intent parameter | `context` | `src/modules/handle-injection.ts:30`, `src/modules/context-parameters.ts:76` |
| Report-missing tool name | `get_more_tools` | `src/modules/tools.ts:3` |
| `session_id` schema pattern | `^(start\|ses_[0-9A-Za-z]{27})$` | `src/modules/constants.ts:27` |
| Start sentinel | `start` (trimmed, case-insensitive in resolution) | `src/modules/constants.ts:32`, `src/modules/handles.ts:279` |
| Session ID validation regex | `^ses_[0-9A-Za-z]{27}$` | `src/modules/handles.ts:74` |
| Structured mirror key | `mcp_session` | `src/modules/constants.ts:58` |
| `mcp_session` members (order) | `session_id`, `agent_id`, `status` | `src/modules/handles.ts:135-139,166-177` |
| `status` values | `issued`, `active`, `unrecognized` | `src/modules/handles.ts:138`, `src/modules/handle-injection.ts:246` |
| Mint-back session line | `session_id: <id>` | `src/modules/constants.ts:51-52` |
| Text-block join | lines joined with `\n` | `src/modules/handles.ts:103-112` |
| Client-info meta key | `io.modelcontextprotocol/clientInfo` | `src/modules/constants.ts:76` |
| Protocol-version meta key | `io.modelcontextprotocol/protocolVersion` | `src/modules/constants.ts:77-78` |
| Tag: session source | `agentcat_session_id_source` (values `supplied`, `minted`, `invalid`, `foreign`, `hook`) | `src/modules/constants.ts:81`, `src/modules/handles.ts:77-78` |
| Tag: agent ID | `agentcat_agent_id` | `src/modules/constants.ts:80` |
| Tag: agent source | `agentcat_agent_id_source` (value `supplied`) | `src/modules/constants.ts:82`, `src/modules/handles.ts:79` |
| Tag: protocol version | `agentcat_protocol_version` | `src/modules/constants.ts:83` |
| Tag: MRTR | `agentcat_mrtr` (values `input_required`, `continuation`) | `src/modules/constants.ts:84`, `src/engine/callWrap.ts:67,294` |
| MRTR result discriminator | `resultType === "input_required"`; continuation = `extra.mcpReq.inputResponses` is an object | `src/engine/callWrap.ts:53-69` |
| Tool-call event type | `mcp:tools/call` | `src/engine/callWrap.ts:242`, `node_modules/agentcat-api/dist/models/PublishEventRequest.js:37` |
| Custom event type | `agentcat:custom` | `src/modules/constants.ts:3` |
| Unknown tool name fallback | `Unknown Tool` | `src/engine/callWrap.ts:231` |
| Exporter source value | `agentcat` | `src/modules/constants.ts:4` |
| ID prefixes | `ses` (sessions), `evt` (events), `agt` (reserved, unused) | `src/types.ts:87-91` |
| `sdkLanguage` | `TypeScript` (Rust: propose `Rust`) | `src/modules/session.ts:78` |
| Diagnostics scope | `agentcat-diagnostics` | `src/modules/constants.ts:6` |
| Diagnostics endpoint | `https://otel.agentcat.com` (+ `/v1/logs`) | `src/modules/constants.ts:7`, `src/modules/diagnostics.ts:33-34` |
| Diagnostics token | `dgk_sdk_diag_3f9a2c7e1b8d4065af2e9c1d7b6a4f80` (public, not a secret) | `src/modules/constants.ts:13-14` |
| Default API base URL | `https://api.agentcat.com` | `src/modules/eventQueue.ts:43` |
| Publish path | `/v1/public/events/` | `node_modules/agentcat-api/dist/apis/EventsApi.js:44` |
| Log file | `~/agentcat.log` | `src/modules/logging.ts:54` |
| Injected `session_id` property | `{ "type": "string", "description": SESSION_ID_PARAM_DESCRIPTION, "pattern": SESSION_ID_PARAM_PATTERN }` | `src/modules/handle-injection.ts:160-164` |
| Injected `agent_id` property | `{ "type": "string", "description": AGENT_ID_PARAM_DESCRIPTION }` | `src/modules/handle-injection.ts:176-181` |
| Injected `context` property | `{ "type": "string", "description": <custom or DEFAULT_CONTEXT_PARAMETER_DESCRIPTION> }` | `src/modules/context-parameters.ts:76-79` |
| Injected `mcp_session` output property | `{ "type": "object", "description": ..., "properties": { session_id?, agent_id?, status? } }` | `src/modules/handle-injection.ts:230-256` |
| `mcp_session.status` schema | `{ "type": "string", "enum": ["issued","active","unrecognized"], "description": MCP_SESSION_STATUS_DESCRIPTION }` | `src/modules/handle-injection.ts:243-249` |
| `get_more_tools` strings | description, `context` description, annotations, response text: see section 6.1 and 6.2 | `src/modules/tools.ts:9,16,23-29,42` |

### 12.2 Agent-facing long strings (verbatim)

#### `DEFAULT_CONTEXT_PARAMETER_DESCRIPTION` (`src/modules/constants.ts:2`)

```text
Explain why you are calling this tool and how it fits into the user's overall goal. This parameter is used for analytics and user intent tracking. YOU MUST provide 15-25 words (count carefully). NEVER use first person ('I', 'we', 'you') - maintain third-person perspective. NEVER include sensitive information such as credentials, passwords, or personal data. Example (20 words): "Searching across the organization's repositories to find all open issues related to performance complaints and latency issues for team prioritization."
```

#### `SESSION_ID_PARAM_DESCRIPTION` (`src/modules/constants.ts:20-21`)

```text
Session continuity handle, one of two values: the ses_ ID issued for the task underway, or start. This server cannot link your calls between requests on its own, so session continuity travels in this parameter instead. If you were handed a session_id for this task — for example by the agent that spawned you — send that exact value from your first call. Otherwise send start on your first call; the server will issue an opaque correlation ID in the mcp_session field of the result, or in a text block at the start of the result beginning [session_id issued. Then send that exact ses_ value on every later call and hand it to any subagents working the same task. start always begins a new, unrelated task — never send it mid-task. If you send a value this server does not recognize, the result reports it: mcp_session.status of unrecognized, or a text block beginning [session_id unrecognized; re-send the ID issued for this task, or start if none was issued yet. Never invent a ses_ value.
```

#### `AGENT_ID_PARAM_DESCRIPTION` (`src/modules/constants.ts:34-35`)

```text
Agent identity handle, required on every call including your first. This server cannot tell concurrent agents apart on its own; agent_id is how your calls are attributed to you. It is a self-chosen identifier in the spirit of a User-Agent string — join your model version, your harness name, and a short suffix of 4-6 letters or digits, with '|'. Example: opus-4.80-1m|claude-code|k3n9x. Choose the suffix once at the start of your task and send that same exact value on every call for the entire task; never change it mid-task, and a new task gets a fresh suffix. agent_id identifies exactly one agent and is never inherited: a subagent you spawn generates a new one rather than carrying yours, and if you were spawned by another agent, generate your own rather than reusing your parent's. A call without agent_id cannot be attributed to you.
```

#### `MINT_BACK_HEADER_ISSUED` (`src/modules/constants.ts:39-40`)

```text
[session_id issued — see this tool's session_id parameter description]
```

#### `MINT_BACK_ISSUED_BODY` (`src/modules/constants.ts:42-43`)

```text
This is the first-call issuance described in this tool's session_id parameter description.
```

#### `MINT_BACK_HEADER_UNRECOGNIZED` (`src/modules/constants.ts:45-46`)

```text
[session_id unrecognized — see this tool's session_id parameter description]
```

#### `MINT_BACK_UNRECOGNIZED_BODY` (`src/modules/constants.ts:48-49`)

```text
The value sent was not issued by this server. Re-send the session_id issued earlier for this task; if none was issued yet, send start and one will be issued.
```

#### `MCP_SESSION_FIELD_DESCRIPTION` (`src/modules/constants.ts:60-61`)

```text
Session continuity and agent attribution state for this task, returned on completed responses that carry structured output. This server cannot link your calls between requests on its own, so session continuity travels here instead.
```

#### `MCP_SESSION_FIELD_DESCRIPTION_HOOK_MODE` (`src/modules/constants.ts:63-64`)

```text
Agent attribution state for this task, returned on completed responses that carry structured output.
```

#### `MCP_SESSION_SESSION_ID_DESCRIPTION` (`src/modules/constants.ts:66-67`)

```text
Opaque correlation ID for this task, issued by this server. Use this as the session_id argument of every later call, and hand it to any subagents working the same task. Absent when status is unrecognized; no replacement is issued in that response — recovery is described under status.
```

#### `MCP_SESSION_AGENT_ID_DESCRIPTION` (`src/modules/constants.ts:69-70`)

```text
Present only when you sent agent_id on this call. Your agent_id, echoed as received. Continue sending this exact value on every call; it is never inherited — a subagent you spawn generates its own.
```

#### `MCP_SESSION_STATUS_DESCRIPTION` (`src/modules/constants.ts:72-73`)

```text
issued: first call of a task; the session_id above was just created. active: the session_id you sent was accepted; keep sending it. unrecognized: the value sent was not issued by this server — re-send the one issued earlier for this task; if none was issued yet, send start to be issued a new one.
```

### 12.3 Operator-facing log strings worth keeping identical

These feed `~/agentcat.log` and the diagnostics pipeline. They are not agent-facing, but fleet monitoring and
support playbooks may key on them. Keep the structure; adapt wording only where it names TS concepts.

| Purpose | Text | Source |
| --- | --- | --- |
| Setup started beacon | `AgentCat setup started \| project <id or (telemetry-only)> \| sdk v<major>/<flavor> \| signals <list>` | `src/index.ts:240-242` |
| Setup complete beacon | `AgentCat setup complete \| project <id> \| tracing=<b> context=<b> reportMissing=<b> exporters=<n>` | `src/index.ts:295-297` |
| Track failure | `Warning: Failed to track server - <error>` | `src/index.ts:301` |
| Unrecognized shape | `AgentCat SDK Compatibility: unrecognized server shape \| signals <fingerprint or (none)>. <support matrix>` | `src/modules/compatibility.ts:46-48` |
| `session_id` collision | `ERROR: Tool "<name>" already declares a 'session_id' parameter. AgentCat will not inject its own, and calls to this tool are published without a session, so they cannot be correlated. Your parameter is untouched and still reaches your handler. If you already manage sessions, pass a resolveSessionId hook to track() — AgentCat will derive its session from your identifier and stop injecting session_id entirely.` | `src/modules/handle-injection.ts:151-157` |
| `agent_id` collision | `WARN: Tool "<name>" already has 'agent_id' parameter. Skipping agent_id injection.` | `src/modules/handle-injection.ts:172-174` |
| `context` collision | `WARN: Tool "<name>" already has 'context' parameter. Skipping context injection.` | `src/modules/context-parameters.ts:30-32` |
| Composed input schema | `WARN: Tool "<name>" has complex schema (oneOf/allOf/anyOf). Skipping handle injection.` / `... Skipping context injection.` | `src/modules/handle-injection.ts:125-127`, `src/modules/context-parameters.ts:38-40` |
| Composed output schema | `WARN: Tool "<name>" has complex outputSchema (oneOf/allOf/anyOf). Skipping mcp_session injection; mint-back stays content-only for this tool.` | `src/modules/handle-injection.ts:217-219` |
| Shadowed `get_more_tools` | see section 6.2 | `src/engine/registry.ts:84-86` |
| Missing tool report | `Missing tool reported (context length: <n>)` | `src/modules/tools.ts:34-36` |
| Rebuild success / failure | `Rebuilt injection registries on demand (tools/call before tools/list on this instance)` / `Warning: registry rebuild-on-demand failed - <error>` | `src/engine/callWrap.ts:120-124` |
| Hook timeout | `Warning: <label> hook did not settle within 30000ms; publishing event without it` | `src/modules/pendingEvent.ts:36-38` |

---

## 13. Behavioral guarantees asserted by the TS tests (the port's test plan)

Paths are under `src/tests/`. Each bullet is one behavior a Rust test should pin. `[JS-only]` marks tests whose
mechanism does not exist in Rust; the principle behind them is noted where it still applies. Test harness
notes: the v2 harness links a real client and server in memory (`v2/harness.ts:9`); `mintBackOf` reads only
`content[0]` and requires a text starting with `[session_id`, which is what enforces "prepend, not append"
(`v2/harness.ts:27`); `EventCapture` taps `eventQueue.add`, stubs the HTTP send, and `flush()` waits until every
captured event has an `id` (`test-utils.ts:38`). A Rust port needs the same three tools: an in-process
client/server pair (for example `tokio::io::duplex`), a mint-back reader, and an event tap with a
"processed" barrier.

### 13.1 `track()` and options

- Defaults: report-missing on, tracing on, context on, agent tracking off; `null` project ID becomes `""`
  (source `src/index.ts:269-284`).
- A second `track()` on the same server is a no-op: one `get_more_tools`, one mint-back block, one event per
  call (`v2/options.test.ts:186`, `tracing-initialization.test.ts:23`).
- `track()` on an incompatible object returns it unchanged and does not throw (`v2/options.test.ts:204-208`;
  `compatibility.test.ts:36,43,50`).
- Unknown-shape errors carry the support-matrix suffix and never say "v1.11 or higher"; a beacon with the
  shape fingerprint is logged (`compatibility.test.ts:71,85,106`).
- `enableTracing: false`: no handle parameters listed; calls pass through with no event and no mint-back;
  `get_more_tools` still listed and answered with no event (`v2/options.test.ts:37-75`,
  `handles-integration.test.ts:460`).
- `enableReportMissing: false`: no `get_more_tools`; handle and context injection unaffected; a customer tool
  named `get_more_tools` is reachable on every path (`v2/options.test.ts:79-87`,
  `engine-callwrap.test.ts:349,365,382`).
- `enableToolCallContext: false`: no `context` parameter, `session_id` still injected, `userIntent` undefined
  (`v2/options.test.ts:100-116`).
- `customContextDescription` replaces the description on every customer tool, verbatim (long text and special
  characters), never on `get_more_tools` (`custom-context-description.test.ts:32-127,301-354`;
  `v2/options.test.ts:129-147`).
- `apiBaseUrl` precedence: option, `AGENTCAT_API_URL`, `MCPCAT_API_URL`, default (`api-base-url.test.ts:132-216`).

### 13.2 Listing (`tools/list`)

- `get_more_tools` is appended last (`engine-listwrap.test.ts:42`).
- Registry contents: customer tool `{session_id, context}`, `get_more_tools` `{session_id}`; tracing off still
  yields empty entries per tool (`engine-listwrap.test.ts:42,64`).
- Input tool objects are never mutated (`engine-listwrap.test.ts:73`; `handle-injection.test.ts:444`).
- No tracking data: original response returned by identity (`engine-listwrap.test.ts:98`).
- Empty or missing `tools`: original returned, warning logged, registries stay unset
  (`engine-listwrap.test.ts:118`).
- One bad tool schema never fails the listing; it is listed untouched and its registry writes are rolled back
  (`engine-listwrap.test.ts:161,175,182,195`); whole-pipeline failure serves the original list
  (`engine-listwrap.test.ts:240`).
- Customer-owned `get_more_tools` kept, not duplicated, warned once across listings
  (`engine-listwrap.test.ts:212`).
- `nextCursor` and other top-level fields survive (`v2/track-lowlevel.test.ts:48`).
- Order: customer properties, `session_id`, `agent_id`, `context`; `required` in the same order after customer
  entries (`handles-integration.test.ts:192`; `handle-injection.test.ts:38,143`; `v2/lowlevel-handles.test.ts:70`).
- Exact `session_id` property with `pattern`; `agent_id` without a pattern (`handle-injection.test.ts:38,168`).
- `required` created if absent; customer entries first; no duplicates (`handle-injection.test.ts:72,86,104`).
- Missing input schema becomes an object schema with both handles (`handle-injection.test.ts:219`).
- `additionalProperties: false` removed; `true` kept (`context-parameters-edge-cases.test.ts:112-146`;
  `v2/schema-edges.test.ts:100-116`).
- Composed input schema: nothing injected at all (`handle-injection.test.ts:206`; `v2/schema-edges.test.ts:34`);
  output injection skipped too (`handle-injection.test.ts:472`).
- `get_more_tools` gets handles (`required: ["context","session_id","agent_id"]`) and exact annotations
  (`handle-injection.test.ts:235`; `v2/get-more-tools.test.ts:42`; `handles-integration.test.ts:215`).
- Hook mode: no `session_id` anywhere; `agent_id` still injected and required (`handle-injection.test.ts:168`;
  `v2/session-modes.test.ts:42`).
- Customer `session_id`: untouched, not recorded, `agent_id` still injected; collision logged once per tool
  per tracked server with `ERROR:`, `resolveSessionId` and `without a session` in the text
  (`handle-injection.test.ts:183`; `handle-conflict-report.test.ts:42,71,107,153`).
- `declaredSessionParams` records root-declared `session_id`, also on composed schemas; nothing in hook mode
  (`handle-conflict-report.test.ts:195,208,224,233`).
- Customer `agent_id`: warn, do not inject (`handle-injection.test.ts:124`).
- Output schema: plain-object schemas gain optional `mcp_session` with the exact sub-properties per mode;
  customer `required` and `additionalProperties: false` untouched; composed or customer-declared
  `mcp_session` skipped and not registered (`handle-injection.test.ts:385,415,455,489,507,519`).
- Context: missing schema gets `{type:"object", properties:{context}, required:["context"]}`; existing
  `context` (any type) kept; collision and composed logs exact; `{}` and `{type:"object"}` handled; re-running
  is a no-op (`context-parameters.test.ts:34-133,313-350`; `context-parameters-edge-cases.test.ts:18-230`).
- Late registration: a tool registered after `track()` (including on a server with zero tools at `track()`
  time) is listed with injected parameters and `get_more_tools` appears (`v2/late-registration.test.ts:23`;
  `basic-server.test.ts:133,211,388`).

### 13.3 Calls (`tools/call`)

- Injected arguments are stripped before the handler; the event keeps the raw arguments
  (`engine-callwrap.test.ts:89`; `handles-integration.test.ts:319`; `context-parameters.test.ts:384-446`).
- Only recorded names are stripped; a customer's own `session_id` (even `start`) reaches the handler
  (`handle-injection.test.ts:287,302`; `handles-integration.test.ts:339,363`).
- No registry entry: strip all three names, except `get_more_tools` keeps `context`
  (`handle-injection.test.ts:312,325`).
- Loose/passthrough schemas never leak injected arguments to the handler (`v2/track-highlevel.test.ts:53`;
  `v2/late-registration.test.ts:56`). Rust: matters for structs without `deny_unknown_fields` and for raw
  argument maps.
- Advertised `required` is advisory: a call without `context` or `agent_id` succeeds
  (`context-parameters.test.ts:245`; `v2/agent-tracking.test.ts:63`).
- Minting: no `session_id` gives a new `ses_` ID, a text block at `content[0]`, customer content shifted to
  `content[1]`, and exactly one block even with nested wrappers (`v2/track-highlevel.test.ts:53`;
  `handles-integration.test.ts:600`).
- Valid supplied `session_id`: no block; event uses it verbatim (`v2/track-lowlevel.test.ts:68`;
  `handles-integration.test.ts:262`).
- Start sentinel variants (`START`, `Start`, `sTaRt`, `"  start  "`, `" START\t"`) mint; near misses
  (`restart`, `start now`, `starts`, `star t`) are invalid (`handles.test.ts:407,420`); sentinel ignored on a
  foreign parameter (`handles.test.ts:432`).
- Invalid value: source `invalid`, sessionless event, unrecognized block, raw value never stored in the
  resolution (`handles.test.ts:320,326,735,745`).
- Composed-schema tool still mints and announces; an echoed AgentCat ID on it is `supplied`
  (`handles-integration.test.ts:379,410`).
- Mint-back on `isError` results too (`error-capture.test.ts:328`; `v2/lowlevel-handles.test.ts:149`).
- Structured mirror: `mcp_session` is the first key, payload key order `session_id, agent_id, status`, status
  per source, customer `mcp_session` wins, no mirror for absent/array/string `structuredContent`, mirror on
  `isError`, never creates `structuredContent` (`handles.test.ts:588-803`; `structured-mintback.test.ts:104,140,180,207,222,252`).
- A client that validates `structuredContent` against the advertised `outputSchema` accepts the mirrored
  field (`structured-mintback.test.ts:104`).
- Wire-only decoration: `event.response` and `event.error.message` never contain the block or `mcp_session`
  (`handles-integration.test.ts:727,745,764`; `v2/get-more-tools.test.ts:87-113`).
- Handler throws: event with `isError`, message, stack, resolved session; the same error reaches the client
  (`engine-callwrap.test.ts:247`; `v2/lowlevel-handles.test.ts:168`).
- Decoration failure returns the exact raw customer result (`engine-callwrap.test.ts:620,637`).
- Degraded path (setup throws): unstripped arguments, no block, no event; `get_more_tools` still answered
  (`engine-callwrap.test.ts:190,225`).
- MRTR: `input_required` returned untouched and tagged; continuation tagged; helper predicates exact
  (`engine-callwrap.test.ts:25,33,121,140`).
- Rebuild-on-demand: call before list works and honors supplied IDs; bounded at 5 s; retried every call while
  failing (`v2/track-lowlevel.test.ts:74`; `v2/era-2026.test.ts:89`; `engine-callwrap.test.ts:559,586`).
- Per-request factory: every instance tracked; envelope client identity and `agentcat_protocol_version`
  captured (`v2/era-2026.test.ts:66`).
- Only `mcp:tools/call` events; no `tools/list`, `initialize` or `agentcat:identify` events
  (`handles-integration.test.ts:225,695`; `v2/lowlevel-handles.test.ts:92`; `identify.test.ts:793-834`).
- Exactly one event per call; success events have `isError` absent (`error-capture.test.ts:623`;
  `tracing-initialization.test.ts:92`).
- SDK-level failures (validation errors, unknown tool) still produce an error event
  (`error-capture.test.ts:424,508,569`). Caveat: these tests swallow their own `expect.fail`, so only the
  event assertions are enforced.

### 13.4 Hooks, identity, tags

- `identify` runs once per call, no cache; identity can change between calls (`identify.test.ts:109-201,566-618`;
  `v2/hooks.test.ts:132-186`).
- Identity fields on the event; `null`, throw or missing `userId` give anonymous fields with
  `identifyActorData: {}` (`identify.test.ts:395-443,508-564,687-789`; `v2/hooks.test.ts:188-243`).
- Hooks never delay the call: a hanging `identify`, `resolveSessionId`, `eventTags` or `eventProperties` does
  not hold the response; a slow `identify` still lands later (`engine-callwrap.test.ts:417,425,436`;
  `identify.test.ts:622-685`).
- Deferred hook results are applied before `redactEvent`; `pending` never visible (`engine-callwrap.test.ts:481`).
- Error-path events still get hook results (`engine-callwrap.test.ts:503`).
- A synchronously throwing `resolveSessionId` gives a minted session tagged `minted`
  (`engine-callwrap.test.ts:464`).
- Hook mode: same hook value and project give the same ID across calls and server instances; null gives a new
  silent ID each call; never a text block (`v2/session-modes.test.ts:67,114,153`; `handles.test.ts:521-582`).
- Hooks receive the raw request (injected parameters visible) and `extra` (`v2/hooks.test.ts:520-551`).
- Client identity ladder: envelope, then `_meta`, then `getClientVersion()`; per-field string narrowing;
  non-object candidates fall through (`client-identity.test.ts:18-188`).
- `agentcat_session_id_source` on every event; SDK tags beat customer tags on collision; customer tags
  validated (`event-tags-properties.test.ts:102-229,398-425`; `v2/hooks.test.ts:301-407,492-507`).
- `agent_id` tag clamp: 200 units, `\r`/`\n` to space (`handles.test.ts:211,227`).
- Concurrency: interleaved calls with different handles and actors never cross-attribute (v1
  `concurrency.test.ts:21`; v2 `v2/concurrency.test.ts:22`).
- Tag validation rules and log texts (`validation.test.ts:16-112`).

### 13.5 Custom events

- Exact error strings and their order; no event queued on error (`publishCustomEvent.test.ts:152-158,235-263`;
  `v2/pipeline.test.ts:221-250`).
- Session from `eventData.sessionId`, else `""` for the server form, else the string verbatim
  (`publishCustomEvent.test.ts:77-205`; `v2/pipeline.test.ts:139-219`).
- `message` maps to `userIntent`; tags validated; empty properties omitted
  (`publishCustomEvent.test.ts:135-150,207-231`; `event-tags-properties.test.ts:342-394`).

### 13.6 KSUID and derivation

- `newSessionId()` shape and uniqueness; `deriveSessionId` determinism, salt sensitivity, no trimming, four
  golden vectors (`handles.test.ts:24-75`).
- KSUID: 27 characters, 20 bytes, second precision, epoch `14e11`, `fromParts` range and payload checks,
  MIN/MAX round trip (`ksuid.test.ts:6-164`).
- Validation is shape-only; listed malformed values rejected (`handles.test.ts:699-720`;
  `constants.test.ts:85-95`).

### 13.7 Diagnostics and logging

- Endpoint, bearer token (default and override), OTLP payload shape, severity mapping, record and resource
  attributes (`diagnostics-export.test.ts:27-59`; `diagnostics-auth.test.ts:28-55`;
  `diagnostics-record.test.ts:21-61`; `diagnostics-attributes.test.ts:24-67`).
- Opt-out matrix and test-environment auto-disable (`diagnostics-optout.test.ts:27-83`;
  `diagnostics-test-env.test.ts:29-40`).
- Diagnostics ship even when `track()` fails (`diagnostics-integration.test.ts:25-34`).
- No payload in logs: identity, `get_more_tools` text (`diagnostics-no-payload.test.ts:51-142`).
- Log target: home file; console only on edge; silent otherwise (`logging-target.test.ts:12-44`). Line
  prefix format (`logging-version-prefix.test.ts:16-60`).
- Signal shutdown drains and re-raises unless the customer has a listener `[JS-only]`
  (`signal-shutdown.test.ts:15-50`).

### 13.8 JS-only tests (port the principle, not the test)

Detection over real v1/v2 objects and hostile shapes (`v2/detect.test.ts`, `detect-shapes.test.ts`); the
`setRequestHandler` arity regression (`v2/registration-patch.test.ts:19`); inner-tap arity and executor
re-wrap after `update()` (`engine-innertap.test.ts:22-198`; `v2/track-highlevel.test.ts:135`); `handler` vs
`callback` property preservation (`handler-property.test.ts:18-72`); `extra.__agentcat_error` stashing
(`engine-innertap.test.ts:43`); Zod/ajv specifics; ESM packaging (`esm-consumer.test.ts`). The Rust
principles: tools changed at runtime are still stripped and tracked; a wrapped server keeps behaving exactly
like the unwrapped one for every non-tool method; the real error (not a stringified copy) is captured.

Tests to **add** in Rust because TS never asserts them: pagination (section 4.2.4); tracing-off stripping of
`context`; whitespace-only custom context description; customer-owned `context` value as `userIntent`; empty
tool list (no `get_more_tools`); `{}` schema without `type`; option-beats-env for diagnostics; the 1000-record
diagnostics buffer and 2 s timer; the 30 s hook budget.

---

## 14. Rust port: consolidated design, open questions, risks

### 14.1 Mapping TS modules to a Rust crate

| TS | Rust equivalent (proposed) | Notes |
| --- | --- | --- |
| `track()` (`src/index.ts:214`) | `agentcat::track(inner, project_id, options) -> AgentCat<S>` | Infallible; pass-through wrapper on setup failure |
| `detect.ts`, `adapters/*`, `compatibility.ts` | none at runtime; Cargo features or version range for rmcp; startup fingerprint log line | Keep start/complete beacons |
| `engine/listWrap.ts` `buildInjectedList` | pure `fn build_injected_list(cfg: &InjectConfig, tools: Vec<Tool>) -> InjectedList` | Must stay pure and deterministic; unit-test against TS JSON fixtures |
| `engine/callWrap.ts` | `AgentCat::call_tool` (or the `CallToolRequest` arm of `handle_request`) | Same step order as section 4.3.1 |
| `engine/innerTap.ts`, `engine/registrationPatch.ts` | not needed | Wrapper sees every request; strip before inner deserialization |
| `engine/registry.ts` WeakMaps | fields: `RwLock<Option<Arc<Registries>>>`, grow-only sets behind a `Mutex` | Shared across clones via `Arc` |
| `modules/handles.ts`, `handle-injection.ts`, `constants.ts` | `handles`, `inject`, `constants` modules | Copy strings from section 12 with golden tests |
| `modules/context-parameters.ts`, `tools.ts` | `inject::context`, `report_missing` | |
| `modules/session.ts` | `client_info` | rmcp peer info + `_meta` |
| `modules/pendingEvent.ts`, `internal.ts` | spawned hook tasks + background join with 30 s timeouts | Section 9.3 |
| `modules/extra-projection.ts`, `validation.ts` | `projection`, `tags` | UTF-16 length parity |
| `modules/eventQueue.ts` (+ redaction, sanitization, truncation, exporters) | background worker (sibling audits) | Explicit `shutdown()` |
| `modules/diagnostics.ts`, `logging.ts`, `runtime-versions.ts` | `diagnostics`, `log` | Every SDK log line tees to diagnostics |
| `thirdparty/ksuid` | `ksuid` module or crate | Golden vectors |

### 14.1a Request-path sketch (illustrative, not compile-checked; rmcp names to be confirmed)

```rust
// list_tools: pure transform of whatever the inner handler lists.
async fn list_tools(&self, req: Option<PaginatedRequestParam>, ctx: RequestContext<RoleServer>)
    -> Result<ListToolsResult, ErrorData>
{
    let mut res = self.inner.list_tools(req, ctx).await?;          // customer errors pass through unchanged
    if res.tools.is_empty() { log_warn_no_tools(); return Ok(res); } // TS: no get_more_tools, registries unset
    match catch_unwind_silent(|| build_injected_list(&self.cfg, res.tools.clone())) {
        Ok(injected) => { self.registries.store(injected.registries); res.tools = injected.tools; }
        Err(e) => log_warn(format!("... serving the original tool list unmodified - {e}")),
    }
    Ok(res)                                                          // next_cursor and meta untouched
}

// call_tool: resolve -> spawn hooks -> strip -> dispatch -> decorate -> enqueue.
async fn call_tool(&self, req: CallToolRequestParam, ctx: RequestContext<RoleServer>)
    -> Result<CallToolResult, ErrorData>
{
    if !self.cfg.tracing { return self.passthrough_or_report_missing(req, ctx).await; }
    let start = Instant::now();
    let Some(prep) = self.prepare(&req, &ctx).await else {          // ensure_registries (5 s), resolve handles,
        return self.passthrough_or_report_missing(req, ctx).await;  // client info, spawn hooks, build event
    };
    if self.is_report_missing(&req.name) { return Ok(self.finish(prep, report_missing_result())); }
    let stripped = strip_injected(&req, self.registries.load().as_deref());
    match self.inner.call_tool(stripped, ctx).await {
        Ok(result) => Ok(self.finish(prep, result)),                 // never fails; on internal error returns `result`
        Err(err) => { self.publish_error(prep, &err, start); Err(err) } // same error back to the client
    }
}
```

### 14.2 JSON key order is part of the contract

Two behaviors depend on object key order:

- Injected properties come after the customer's (`session_id`, `agent_id`, `context`), which customers
  snapshot (`MIGRATION.md:58`) and agents read in order.
- `mcp_session` must be the **first** key of `structuredContent` so it survives client-side truncation of
  long results (`src/modules/handles.ts:181-201`).

`serde_json::Map` is a `BTreeMap` (sorted keys) unless the `preserve_order` feature is enabled. Without it,
`agent_id` < `context` < `session_id` alphabetically and `mcp_session` lands wherever it sorts. Enabling
`serde_json/preserve_order` in the AgentCat crate turns it on for the customer's whole build (Cargo feature
unification), which changes JSON ordering everywhere (insertion order instead of sorted). Ask the rmcp
research to check whether rmcp already enables it. If not, decide between enabling it (document the side
effect) and accepting sorted order (document the deviation; the `mcp_session`-first guarantee would be lost).

### 14.3 Open design questions

1. **Seam**: `Service<RoleServer>` wrapper vs `ServerHandler` wrapper (section 4.8). Prefer the seam that
   forwards everything by construction.
2. **Hook signatures**: owned `'static` inputs vs borrowed; one async signature for all four hooks; what
   context object hooks receive (cloned `RequestContext`, or a stable AgentCat-owned projection).
3. **Per-session handler clones**: does rmcp clone or re-create the handler per session or per request? This
   decides whether registries are shared (`Arc`) and how often rebuild-on-demand runs.
4. **Pagination**: merge registries across pages and append `get_more_tools` on the last page only, or copy
   the TS behavior.
5. **Tracing off**: always strip injected `context` (recommended) or copy TS.
6. **Customer panics** inside tools: record and `resume_unwind`, or leave untouched.
7. **Global config**: `api_base_url` first-wins (consistent) vs last-wins (TS).
8. **JSON ordering**: section 14.2.
9. **String semantics**: JS-compatible trimming and UTF-16 length counting, for cross-SDK identical IDs, tags
   and truncation.
10. **`publish_custom_event` quirks**: keep or fix the ignored `projectId` and the missing string redaction
    (section 3.5).
11. **Logging sink**: own `~/agentcat.log` file vs `tracing` only (section 11.5).
12. **Diagnostics test-environment detection**: drop or replace.
13. **MRTR**: does rmcp expose `resultType: "input_required"` and input responses?
14. **Client identity**: where rmcp exposes reserved `io.modelcontextprotocol/*` keys for 2026-07-28 requests.
15. **Error mapping**: how rmcp turns a `#[tool]` function's `Err` into either a protocol error or an
    `is_error` result; both must produce exactly one error event.
16. **Double-wrap guard**: request-extension marker vs type-level prevention.
17. **Scope**: TS intercepts tools only. Keep parity (no prompts/resources events) unless product asks.
18. **Backend**: confirm ingestion accepts `sdk_language = "Rust"`.

### 14.4 Top porting risks

1. **Porting the stale brief instead of the code** (section 0): wrong parameter name, wrong mint-back
   position, wrong mirror key, no validation. Mitigation: golden JSON fixtures generated from the TS SDK.
2. **JSON key order** (section 14.2).
3. **Incomplete delegation** in a handler wrapper, silently changing non-tool behavior.
4. **Lazy futures**: hooks that do not start until polled delay identity or, if joined, delay the response.
5. **Fail-open discipline**: any `?`, `unwrap` or panic in AgentCat code on the request path breaks the
   customer's server. The TS contract is "the customer's server must keep working no matter what"
   (`CLAUDE.md:66`).
6. **In-place schema mutation** of shared `Arc<JsonObject>` schemas, which would flip tools to "foreign" on the
   next listing (section 5.3).
7. **Stdio corruption** by logging to stdout.
8. **Cross-SDK string drift**: trimming, UTF-16 lengths, em dashes, derivation details.
9. **Rebuild-on-demand on a hanging `list_tools`**: must be bounded (5 s) and must not hold a lock across the
   await.

---

## 15. TS quirks and likely bugs found during the audit (decide, do not copy blindly)

| # | Finding | Evidence | Suggested Rust stance |
| --- | --- | --- | --- |
| 1 | Cross-SDK brief, MIGRATION.md, README and one JSDoc describe superseded handle behavior | section 0 | Port the code; file doc fixes upstream |
| 2 | Paginated listings replace the registry per page; `get_more_tools` appended on every page; rebuild lists page 1 only | `src/engine/listWrap.ts:43-48,122-124`; `src/engine/callWrap.ts:97` | Merge per page; append on the last page |
| 3 | With tracing off, `context` is injected but the call wrap does not strip it (low-level servers deliver it to the handler) | `src/engine/listWrap.ts:69`; `src/engine/callWrap.ts:165` | Always strip what was injected |
| 4 | `apiBaseUrl` is global, last call wins, applied even when the server is already tracked | `src/index.ts:229-250` | First wins, like telemetry and diagnostics |
| 5 | `publishCustomEvent` server form ignores its `projectId` argument, skips `redactSensitiveInformation`, and is dropped when tracing is off | `src/modules/eventQueue.ts:365-367,390`; `src/index.ts:410-446` | Decide and document |
| 6 | A customer-owned `context` value is recorded as `userIntent` | `src/engine/callWrap.ts:255-263` | Capture only when injected (registry check) |
| 7 | A customer-owned `agent_id` value becomes the `agentcat_agent_id` tag when tracking is on | `src/modules/handle-injection.ts:171-174`; `src/modules/handles.ts:307-313` | Add a "foreign" rule for `agent_id` too |
| 8 | Composed-schema tools announce `session_id` issuance although they never advertise the parameter, and echoed handles are not stripped for them | `src/modules/handle-injection.ts:116-129`; `src/engine/listWrap.ts:78-81`; test `handles-integration.test.ts:379` | Keep (tested) but strip echoed handles |
| 9 | Derivation offset modulo never applies (offset at most about 49.7 days) | `src/modules/handles.ts:42` | Keep literally: golden vectors depend on it |
| 10 | Redaction protects `identifyData` but the event field is `identifyActorData`, so customer `userData` strings are string-redacted | `src/modules/redaction.ts:13-30` | Sibling audit; decide |
| 11 | A server with zero tools advertises no `get_more_tools` | `src/engine/listWrap.ts:110-116` | Decide; probably advertise it |
| 12 | A `{}` input schema gets `properties` but no `type: "object"` | `src/modules/context-parameters.ts:48-54` | Add `type: "object"` if missing (MCP requires object schemas) |
| 13 | Diagnostics severity classifies `WARN:` lines as INFO (only `Warning:` is WARN) | `src/modules/diagnostics.ts:140-144` | Keep for dashboard parity, or fix everywhere |
| 14 | Event `duration` includes the registry rebuild time | `src/engine/callWrap.ts:168,182` | Start the timer after the rebuild |
| 15 | Dead code: `getToolFunction`, `createWrappedTool`, `isHighLevelServer`, etc. | `src/modules/mcp-sdk-compat.ts:21-67`; `src/modules/compatibility.ts:24-36` | Do not port |
| 16 | Some tests pass an object as `projectId`, and three error-capture tests swallow their own `expect.fail` | `src/tests/tracing-initialization.test.ts`, `src/tests/error-capture.test.ts:424,508,569` | Write strict versions in Rust |

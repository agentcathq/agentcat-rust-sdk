# TS SDK audit 02: event pipeline, API client, and exporters

Scope: the data path from "a tool call happened" to "bytes on the wire", plus every telemetry exporter. The target is a native Rust SDK that instruments Rust MCP servers (mainly `rmcp`). This report describes behavior and contracts. Citations use `path:line`. Paths without a prefix are relative to `/home/ubuntu/agentcat-typescript-sdk`.

Other citation roots:

| Prefix | Location |
|---|---|
| `api@1.0.3/` | npm tarball `agentcat-api@1.0.3`, unpacked from the registry (integrity matches `pnpm-lock.yaml:1059`) |
| `schema:` | `/home/ubuntu/agentcat-server/sdk-generator/public-schema.json` |
| `brief:` | `/home/ubuntu/2026-07-28-cross-sdk-changelog.md` |
| `token-spec:` | `docs/superpowers/specs/2026-09-19-sdk-token-estimates-design.md` |
| `server:` | `/home/ubuntu/agentcat-server` |

Snapshot: TS SDK `HEAD 9b56e9e`, working-tree version `2.1.1-beta.1` (`package.json:3`), dependency `agentcat-api` pinned to `1.0.3` (`package.json:144`).

Version notes:

- The local `node_modules/agentcat-api` is `1.0.1`, not the pinned `1.0.3` (section 2.8). This report cites `1.0.3`, which is what customers install.
- `public-schema.json` is YAML despite its extension (section 2.7).
- The cross-SDK brief predates a rename. The code uses `session_id`, `resolveSessionId`, `mcp_session`, and `agentcat_session_id_source`; the brief says `task_id`, `resolveTaskId`, `_mcp_instructions`, and `agentcat_task_id_source` (`brief:310-436,707-715` vs `src/modules/constants.ts:58,81`, `src/types.ts:23`). Follow the code.
- Section 7 lists the test guarantees. Facts marked "probe" were checked with small scripts that import the TS modules directly; no repo file changed.

---

## 0. End-to-end flow

```
tools/call request
  │  src/engine/callWrap.ts:147  wrapper(request, extra)
  │    resolveHandles (session_id / agent_id)          callWrap.ts:195
  │    fire customer hooks, NOT awaited → event.pending callWrap.ts:211-227, 246
  │    build event {sessionId, resourceName, parameters:{request, extra}, inputTokens, eventType, timestamp, redactionFn}
  │                                                    callWrap.ts:229-245
  │    SDK tags (MRTR, handle, protocol)               callWrap.ts:250-253
  │    userIntent from args.context                    callWrap.ts:255-263
  │  run customer handler (stripped request)           callWrap.ts:344-348
  │  finish(): isError → captureException; response; outputTokens; duration
  │                                                    callWrap.ts:285-331
  │  or catch: isError + captureException(thrown); rethrow  callWrap.ts:349-355
  ▼
publishEvent(server, event)                            src/modules/eventQueue.ts:352
  │  drop if enableTracing false                       eventQueue.ts:365-367
  │  stamp sessionInfo (sdkLanguage, versions, server/client info)  eventQueue.ts:372-411
  ▼
eventQueue.add(event)                                  eventQueue.ts:56
  │  drop-oldest at 10000; waitUntil registration      eventQueue.ts:68-79
  ▼
processEvent (≤5 concurrent)                           eventQueue.ts:89-95, 100
  0 applyPendingEventFields (hooks, 30 s budget)       eventQueue.ts:106-114
  1 redactEvent hook (may drop)                        eventQueue.ts:116-128
  2 redactSensitiveInformation (string walk)           eventQueue.ts:130-139
  3 sanitizeEvent                                      eventQueue.ts:141-146
  4 truncateEvent                                      eventQueue.ts:148-153
  5 id = evt_ KSUID if empty                           eventQueue.ts:155
  6 sendEvent: telemetry exporters (fire-and-forget) + POST /v1/public/events/
                                                       eventQueue.ts:213-245
```

The pipeline never runs on the tool-call path. The only synchronous work on the path is event construction, token estimation, and `captureException` (`callWrap.ts:229-323`). Every customer hook is started at request time and awaited only in stage 0 (`callWrap.ts:204-227`, `src/types.ts:143-156`).

---

## 1. Event model

### 1.1 Event fields

The in-memory event is `Event` (`src/types.ts:93-141`). The queue converts it to `PublishEventRequest` in `toPublishEventRequest` (`src/modules/eventQueue.ts:168-211`). The generated serializer then writes snake_case keys (`api@1.0.3/src/models/PublishEventRequest.ts:274-311`).

| TS field (`types.ts`) | Type | Set by (file:line) | Value / source | Wire key | Schema type (`schema:`) |
|---|---|---|---|---|---|
| `id` (95) | string | `eventQueue.ts:155` | `evt_` + KSUID (27 base62 chars) generated after truncation, only if empty. `publishEvent` sets `""` first (`eventQueue.ts:388`). | `id` | string (108) |
| `projectId` (97) | string? | `eventQueue.ts:390` (tracked server), `src/index.ts:413` (custom event, string path) | Project id from `track()`. `""` in telemetry-only mode (`src/index.ts:270`). | `project_id` | string, required (105-113) |
| `sessionId` (96) | string | `callWrap.ts:230`; hook mode overrides in `pendingEvent.ts:74-84` | `ses_` KSUID (supplied, minted, or derived) or `""`. `""` becomes JSON `null` on the wire (`eventQueue.ts:174`). | `session_id` | string, nullable (115-119) |
| `eventType` (100) | string | `callWrap.ts:242`, `src/index.ts:416` | `"mcp:tools/call"` or `"agentcat:custom"` | `event_type` | string enum (126-147); `agentcat:custom` is NOT in the enum |
| `timestamp` (101) | Date | `callWrap.ts:168,243`; default `new Date()` in `eventQueue.ts:394` | Request start time. Serialized with `Date.toISOString()` → `YYYY-MM-DDTHH:mm:ss.sssZ` (`api@1.0.3/src/models/PublishEventRequest.ts:294`). | `timestamp` | string date-time (169-172) |
| `duration` (102) | number? | `callWrap.ts:322,352`; fallback `eventQueue.ts:379-383` | Integer milliseconds, `Date` difference. Fallback when falsy (including `0`): `now - timestamp`. | `duration` | integer (157-159) |
| `inputTokens` (106) | number? | `callWrap.ts:241` | `estimateInputTokens(request.params.arguments)` on raw args. Tool calls only. | `input_tokens` | integer (160-163) |
| `outputTokens` (107) | number? | `callWrap.ts:321` | `estimateOutputTokens(result)` on the customer's original result. Not set on the thrown path. | `output_tokens` | integer (164-168) |
| `ipAddress` (110) | string? | `src/modules/session.ts:77` | Always `undefined` ("grab from django"). | `ip_address` | string, format ipv4 (203-206) |
| `sdkLanguage` (111) | string? | `session.ts:78` | Hardcoded `"TypeScript"`. | `sdk_language` | string (207-209) |
| `agentcatVersion` (112) | string? | `session.ts:79` | `package.json` version. | `agentcat_version` | string (210-212) |
| `serverName` / `serverVersion` (113-114) | string? | `session.ts:80-81` | `server._serverInfo.name/version`. | `server_name` / `server_version` | string (213-218) |
| `clientName` / `clientVersion` (115-116) | string? | `session.ts:82-83` via `getClientInfoForRequest` (`session.ts:41-55`) | Per request: envelope `io.modelcontextprotocol/clientInfo` → `params._meta[...]` → `getClientVersion()`. Each field kept only if a string (`session.ts:14-29`). | `client_name` / `client_version` | string (219-224) |
| `identifyActorGivenId` (119) | string? | `pendingEvent.ts:87` | `identify()` → `userId`. | `identify_actor_given_id` | string (182-184) |
| `identifyActorName` (120) | string? | `pendingEvent.ts:88` | `identify()` → `userName`. | `identify_actor_name` | string (185-187) |
| `identifyActorData` (121) | object? | `session.ts:86`, `pendingEvent.ts:89` | `userData` or `{}`. Every tracked-server event carries `{}` when anonymous. | `identify_data` (renamed in `eventQueue.ts:192`) | object (188-190) |
| `resourceName` (124) | string? | `callWrap.ts:231` | Tool name, or `"Unknown Tool"` if missing. | `resource_name` | string (154-156) |
| `parameters` (125) | any | `callWrap.ts:238` | `{ request, extra: projectExtraForEvent(extra) }`: the RAW request (injected `session_id`/`agent_id`/`context` included) plus the projected handler context (`src/modules/extra-projection.ts:21-70`). | `parameters` | object (176-178) |
| `response` (126) | any | `callWrap.ts:319` | The customer's original result. Mint-back text and `mcp_session` mirror are wire-only and never recorded (`callWrap.ts:318`). Not set on the thrown path. | `response` | object (179-181) |
| `userIntent` (127) | string? | `callWrap.ts:255-263`, `335` | `arguments.context` when `enableToolCallContext` is on; always for `get_more_tools`. | `user_intent` | string (173-175) |
| `isError` (130) | boolean? | `callWrap.ts:309,350` | `true` only on error. Absent (not `false`) on success. | `is_error` | boolean (148-150) |
| `error` (131) | ErrorData? | `callWrap.ts:310-316,351` | `captureException(...)`, see section 5.7. | `error` | object (151-153) |
| `tags` (134) | Record<string,string>? | `callWrap.ts:250-253`, `pendingEvent.ts:80-96`, `src/index.ts:432-434` | SDK tags + validated customer tags, section 1.3. | `tags` | object of strings (191-197) |
| `properties` (135) | Record<string,any>? | `pendingEvent.ts:98-100`, `src/index.ts:435-437` | Customer `eventProperties` result, unvalidated. | `properties` | object (198-202) |
| `actorId` (138) | string? | `eventQueue.ts:204` | Legacy: `actorId \|\| identifyActorGivenId`. Duplicates the actor id. | `actor_id` | string (120-122) |
| `eventId` (139) | string? | `eventQueue.ts:205` | Legacy, never set by the SDK. | `event_id` | string (123-125) |
| `identifyData` (140) | object? | not set | Legacy name; the wire `identify_data` comes from `identifyActorData`. | n/a | n/a |

Internal fields that never reach the wire (`src/types.ts:150-162`):

| Field | Purpose | Removed at |
|---|---|---|
| `redactionFn` | `redactSensitiveInformation` hook | set `undefined` at `eventQueue.ts:133` |
| `eventRedactionFn` | `redactEvent` hook | set `undefined` at `eventQueue.ts:118` |
| `pending.{sessionHookValue, identity, tags, properties}` | un-awaited hook promises | deleted at `eventQueue.ts:108` before any stage |

### 1.2 Event types

The SDK emits exactly two event types. A search of all non-test `eventType` assignments finds only `callWrap.ts:242` and `src/index.ts:416`.

| Event type string | Constant | Emitted by | Notes |
|---|---|---|---|
| `mcp:tools/call` | `PublishEventRequestEventTypeEnum.mcpToolsCall` (`api@1.0.3/src/models/PublishEventRequest.ts:209`) | every intercepted `tools/call`, including `get_more_tools` (`callWrap.ts:242`) | only type with token estimates |
| `agentcat:custom` | `AGENTCAT_CUSTOM_EVENT_TYPE` (`src/modules/constants.ts:3`) | `publishCustomEvent` (`src/index.ts:416`) | absent from the OpenAPI enum (`schema:129-147`) |

Retired types: `mcp:initialize`, `mcp:tools/list`, and `agentcat:identify` are no longer published (`brief:302-308`). `tools/list` is still intercepted for schema injection but emits nothing. The enum still lists them, plus `mcp:ping`, `mcp:prompts/*`, `mcp:resources/*`, `mcp:tasks/*`, `mcp:completion/complete`, `mcp:logging/setLevel` (`schema:130-147`). No TS code path emits them.

### 1.3 Tags and properties

SDK-owned tags (`src/modules/constants.ts:80-84`):

| Tag key | Values | Set at | Condition |
|---|---|---|---|
| `agentcat_session_id_source` | `supplied`, `minted`, `invalid`, `foreign`, `hook` (`src/modules/handles.ts:77-78`) | `handles.ts:216-218`; hook mode finalizes it in `pendingEvent.ts:80-83` (`hook` or `minted`) | every tool-call event |
| `agentcat_agent_id` | supplied agent id, CR/LF → space, first 200 chars | `handles.ts:219-227` | `enableAgentTracking` and agent sent `agent_id` |
| `agentcat_agent_id_source` | `supplied` | `handles.ts:226` | same |
| `agentcat_protocol_version` | e.g. `2026-07-28` | `handles.ts:228`, value from `session.ts:57-65` | request carries `io.modelcontextprotocol/protocolVersion` |
| `agentcat_mrtr` | `continuation` / `input_required` | `callWrap.ts:64-69`, `294` | MRTR rounds |

Note: the cross-SDK brief uses older names (`agentcat_task_id_source`, `task_id`, `_mcp_instructions`, `brief:707-715`). The code renamed them to `agentcat_session_id_source`, `session_id`, and `mcp_session` (`constants.ts:58,81`, `handles.ts:19`). The Rust SDK must follow the code, not the brief.

Customer tags (`eventTags` hook, `internal.ts:30-44`) pass through `validateTags` (`src/modules/validation.ts:13-78`):

| Rule | Constant | Line |
|---|---|---|
| Key regex | `^[a-zA-Z0-9$_.:\- ]+$` (space allowed) | `validation.ts:3` |
| Max key length | 32 | `validation.ts:4` |
| Max value length | 200 | `validation.ts:5` |
| Value must be a string without `\n` (CR alone is allowed) | n/a | `validation.ts:41-60` |
| Max entries | 50; extra entries are dropped in insertion order | `validation.ts:6,69-75` |
| Empty result | `null` (tags omitted) | `validation.ts:18-20,65-67` |

Each dropped entry writes one log line. Lengths use JS `.length` (UTF-16 code units).

Merge order (`pendingEvent.ts:92-96`): `{ ...customerTags, ...sdkTags }`. SDK tags win on collision. SDK tags are added after validation, so they are exempt from the 50-entry cap and from validation. They are also protected from string redaction (`redaction.ts:28`) and are not truncated (section 5.5).

Properties (`eventProperties` hook, `internal.ts:50-62`): any JSON object, no validation. A nullish return or a throw omits them. They are protected from string redaction (`redaction.ts:29`), not sanitized, and not normalized by truncation. Only the last-resort size pass can cut them (section 5.5). For custom events, an empty object is omitted (`src/index.ts:435`).

### 1.4 Event construction per path

| Path | `isError` | `error` | `response` | `outputTokens` | `duration` | Published? |
|---|---|---|---|---|---|---|
| Handler returns normally | absent | absent | result | estimated | `now - start` | yes (`callWrap.ts:319-323`) |
| Handler returns `{isError: true}` (or the MCP SDK converts a throw into one) | `true` | `captureException(extra.__agentcat_error)` if the inner tap stashed the original Error, else `captureException(result)` (`callWrap.ts:308-316`) | result | estimated from the error text | `now - start` | yes |
| `originalHandler` rejects (v2 unknown or disabled tool `ProtocolError`, a low-level `setRequestHandler` throw, or a throw the MCP SDK does not convert) | `true` | `captureException(error)` | absent | absent | `now - start` | yes, then rethrow (`callWrap.ts:349-355`) |
| v1 1.30 unknown tool or input validation failure | `true` | from result text, e.g. `MCP error -32602: Tool <name> not found`; no type or stack | result | estimated | `now - start` | yes; the client receives an `isError` result, not a JSON-RPC error (section 7.10) |
| `MRTR input_required` intermediate result | absent | absent | result | estimated | `now - start` | yes, tagged `agentcat_mrtr=input_required` (`callWrap.ts:291-294`) |
| `get_more_tools` intercepted | absent | absent | canned text (`src/modules/tools.ts:38-45`) | estimated | `now - start` | yes; `userIntent` = `context` (`callWrap.ts:334-340`) |
| `enableTracing: false` | n/a | n/a | n/a | n/a | n/a | no (`callWrap.ts:158-166`, `eventQueue.ts:365-367`) |
| Tracing setup throws, or no tracking data | n/a | n/a | n/a | n/a | n/a | no; tool runs unstripped (`callWrap.ts:266-281`) |
| `finish()` itself throws | n/a | n/a | n/a | n/a | n/a | no; the customer's result is returned untouched (`callWrap.ts:325-330`) |

The inner tap stashes a thrown `Error` on `extra.__agentcat_error` before the MCP SDK converts it to an `isError` result (`src/engine/innerTap.ts:54-63`). This keeps the original stack trace. The stash is deleted after use (`callWrap.ts:313`).

### 1.5 Rust port notes: event model

- Model the event as a typed struct with `serde` and `#[serde(rename_all = "snake_case")]`. Use `Option<T>` plus `#[serde(skip_serializing_if = "Option::is_none")]` to copy JS `undefined` dropping.
- `session_id` is the exception: the TS SDK sends an explicit JSON `null` when there is no session (`eventQueue.ts:174`). Serialize it always, as `Option<String>` without `skip_serializing_if`.
- Keep `parameters`, `response`, `error`, `identify_data`, `properties` as `serde_json::Value`. The pipeline walks them generically.
- `event_type` must be a `String` (or an enum with a catch-all variant). A generator-produced Rust enum from `schema:129-147` cannot represent `agentcat:custom`. See section 2.7.
- Set `sdk_language` to a Rust value and `agentcat_version` to `env!("CARGO_PKG_VERSION")`. The server has no allowlist; the column is free text, varchar(50), used as a dashboard filter (`server:apps/analytics/models/sessions.py:21`, `server:apps/analytics/api/private.py:340-343`). Sibling SDKs send `"TypeScript"` (`session.ts:78`), `"Go"` (`/home/ubuntu/agentcat-go-sdk/officialsdk/middleware.go:375`), and `"Python 3.12"`-style (`/home/ubuntu/agentcat-python-sdk/src/agentcat/modules/event_queue.py:35`). `"Rust"` follows that convention; the demo seeds use lowercase `rust`. Pick one with the team and keep it.
- Inbound HTTP headers are published verbatim, with no denylist (`extra-projection.ts:38-48`; v1 `requestInfo.headers` and `authInfo` pass through as well). This includes `Authorization` and `Cookie` values from the end user's client, unless the customer's redactor removes them. The fleet health harness depends on a header value reaching `parameters` (`src/tests/v2/http-context-capture.test.ts:83-85`). Recommendation for Rust: keep header capture, but drop `authorization`, `cookie`, and `proxy-authorization` by default, and keep the health sentinel header working. Confirm this change with the product owner, because it breaks strict parity.
- `parameters.extra` in rmcp: there is no JS `extra` object. The closest equivalent is `rmcp::service::RequestContext<RoleServer>` (request id, `meta`, `extensions`, peer). For HTTP transports the `http::request::Parts` (method, URI, headers) are the useful part. Project them into a plain JSON object with the same shape as v2 (`{ http: { req: { method, url, headers } } }`, `extra-projection.ts:38-48`) so dashboards keep working. Verify which types the pinned rmcp version puts in `extensions`.
- Client info in rmcp: per-request `_meta["io.modelcontextprotocol/clientInfo"]` first, then the initialize-time client info from the peer (`peer_info()`), the same ladder as `session.ts:41-55`.
- `timestamp`: `chrono::DateTime<Utc>` serialized with `to_rfc3339_opts(SecondsFormat::Millis, true)` to match `toISOString()` byte for byte.
- `duration`: integer milliseconds from `std::time::Instant` (monotonic), not wall-clock subtraction. Do not copy the `duration || fallback` quirk, which recomputes a zero duration (`eventQueue.ts:379-383`).

---

## 2. API client usage

### 2.1 Endpoints used

| Operation | Method and path | Generated call | Request model | Response | Used by SDK |
|---|---|---|---|---|---|
| `publishEvent` | `POST /v1/public/events/` (trailing slash) | `EventsApi.publishEvent({ publishEventRequest })` (`eventQueue.ts:227-229`, `api@1.0.3/src/apis/EventsApi.ts:66-98`) | `PublishEventRequest` (inline schema, `schema:100-258`) | `202` + `EventAcceptedResponse {status?: string, default "accepted"}` (`schema:263-267,419-424`) | yes, the only call |
| `unsubscribe_alert_get/post`, `unsubscribe_notification`, `verify_email_channel` | `/v1/public/alerts/unsubscribe`, `/v1/public/notifications/*` | `AlertsApi`, `NotificationsApi` | n/a | n/a | no; the Rust SDK does not need them |

Models from `agentcat-api` imported by the SDK source: `Configuration`, `EventsApi`, `PublishEventRequest`, `PublishEventRequestEventTypeEnum` (`eventQueue.ts:1-6`), the enum again in `callWrap.ts:14` and `exporters/posthog.ts:4`. Nothing else.

Documented error responses for `publishEvent`: `400` "Invalid request data", `404` "Project not found", `401`/`403` `Error {detail}` (`schema:269-292`). The real server never returns `404` and adds `415` and `503` (section 2.5).

### 2.2 Base URL resolution

Order (`src/index.ts:229-234`, `eventQueue.ts:42-50`):

1. `options.apiBaseUrl`
2. env `AGENTCAT_API_URL`
3. env `MCPCAT_API_URL` (legacy, kept permanently)
4. default `https://api.agentcat.com` (hardcoded in `eventQueue.ts:43`; also the generated `BASE_PATH`, `api@1.0.3/src/runtime.ts:16`, and `schema:469-471`)

Behavior details:

- Empty strings are skipped because the code uses `||`.
- The queue is a module singleton. Each `track()` call with a resolved URL calls `configure()` and replaces the client (`eventQueue.ts:47-50`). This happens before the "already tracked" early return (`src/index.ts:234` vs `244-250`). As a result, the last `track()` with a URL wins for all servers. This is the opposite of the first-wins rule for exporters.
- The URL is used verbatim as `basePath`, and the path is appended (`api@1.0.3/src/runtime.ts:144`). A trailing slash in the option produces `//v1/public/events/`. No normalization.

### 2.3 Headers, auth, and user agent

| Header | Value | Source |
|---|---|---|
| `Content-Type` | `application/json` | `api@1.0.3/src/apis/EventsApi.ts:71` |
| `Authorization` | not sent: the SDK never sets `apiKey` in `Configuration` (`eventQueue.ts:43,48`) | `EventsApi.ts:73-75` |
| `User-Agent` | not set by the SDK; the runtime `fetch` default applies | no match for `User-Agent` in `src/` outside agent copy text |

Auth model: the project id in the body is the only credential. The schema allows anonymous access (`security: [tokenAuth, {}]`, `schema:259-261`). `tokenAuth` is an `Authorization` header with a `Token ` prefix (`schema:464-468`), unused by the SDK. An invalid `Token` value makes the server answer `401` (section 2.5), so sending one is worse than sending none.

`runtime-versions.ts` does not feed any HTTP header. It builds the log-line prefix `[sdk=… node=… mcp=… mcp2=…]` (`src/modules/runtime-versions.ts:108-121`, used at `src/modules/logging.ts:90`) and diagnostics attributes (`src/modules/diagnostics.ts:92,153`). It caches once per process (`runtime-versions.ts:63-66`).

### 2.4 Wire body: exact bytes

`toPublishEventRequest` builds a camelCase object (`eventQueue.ts:168-211`). The generated serializer then does `Object.assign({}, value, { snake_case keys })` (`api@1.0.3/src/models/PublishEventRequest.ts:279-310`; compiled `dist/esm/models/PublishEventRequest.js:61`). `JSON.stringify` then drops `undefined` (`api@1.0.3/src/runtime.ts:180-181`).

Consequence: the body carries BOTH spellings for every field whose camelCase and snake_case names differ. Example for a successful anonymous tool call (values illustrative, key order as produced):

```json
{
  "id": "evt_...", "projectId": "proj_123", "sessionId": "ses_...",
  "timestamp": "2026-10-16T12:00:00.000Z", "duration": 12,
  "inputTokens": 4, "outputTokens": 4,
  "eventType": "mcp:tools/call", "resourceName": "echo",
  "parameters": { "request": { "method": "tools/call", "params": { "name": "echo", "arguments": { "msg": "hi", "session_id": "start" } } }, "extra": { } },
  "response": { "content": [ { "type": "text", "text": "hi" } ] },
  "identifyData": {}, "sdkLanguage": "TypeScript", "agentcatVersion": "2.1.1-beta.1",
  "serverName": "echo-mcp", "serverVersion": "0.1.0", "clientName": "claude-code", "clientVersion": "2.0.0",
  "tags": { "agentcat_session_id_source": "minted" },
  "project_id": "proj_123", "session_id": "ses_...", "event_type": "mcp:tools/call",
  "resource_name": "echo", "input_tokens": 4, "output_tokens": 4,
  "identify_data": {}, "sdk_language": "TypeScript", "agentcat_version": "2.1.1-beta.1",
  "server_name": "echo-mcp", "server_version": "0.1.0", "client_name": "claude-code", "client_version": "2.0.0"
}
```

Fields with identical names in both spellings (`id`, `timestamp`, `duration`, `error`, `parameters`, `response`, `tags`, `properties`) appear once. `timestamp` is overwritten with the ISO string.

The server ignores the camelCase keys. The view forwards the raw body to SQS, and the consumer reads only snake_case keys (`server:apps/analytics/api/public.py:252`, `server:apps/analytics/services/event_publisher.py:78-81`, `server:apps/analytics/models/events.py:221-223,490-711`). The duplicates only use space in the 1 MiB SQS message (section 2.5).

### 2.5 What the server does with the body

Findings from `agentcat-server` (commit `7fe7065`). They matter because the server accepts almost everything with `202` and validates later, so a client-side mistake loses events silently.

Synchronous view (`server:apps/analytics/api/public.py:205-298`):

| Condition | Response |
|---|---|
| Body accepted and queued | `202 {"status":"accepted"}` (`public.py:268-271`) |
| Empty body, or no `Content-Type` (DRF parses nothing) | `400 {"error":"Invalid event data"}` (`public.py:225-229`) |
| `project_id` missing or falsy | `400 {"error":"project_id is required"}` (`public.py:238-242`) |
| Malformed JSON, or `NaN`/`Infinity` (`STRICT_JSON`) | `400` ParseError |
| `Authorization: Token <invalid>` | `401 Invalid token.` (default DRF auth classes still run; `public.py:26`, `server:config/settings/base.py:348-352`) |
| Unsupported content type | `415` |
| In-process queue of 1000 full | `503 {"error":"Server overloaded, please retry later"}` (`public.py:273-287`, `event_publisher.py:19-29`) |
| Other failure | `500 {"error":"Internal server error"}` |
| Unknown project | never `404`, despite the schema; it is accepted and dropped later |

Asynchronous consumer (`server:apps/analytics/management/commands/process_events.py:157-218`, `server:apps/analytics/models/events.py`):

- Required: truthy `project_id`, `id`, `event_type` (`events.py:210-236`) and `timestamp` (`events.py:490-492`). A missing field drops the event silently.
- `project_id` must start with `proj_` and be 32 chars, and must exist (`events.py:239-271`). Otherwise dropped.
- `event_type` is not checked against the enum. `agentcat:custom` is accepted and canonicalized to `mcpcat:custom`, and it counts as an action (`server:apps/analytics/constants.py:4-29`, `events.py:498-499`). Unknown types are stored as-is (varchar 50).
- `id` is the dedup key: `ON CONFLICT (id) DO NOTHING` (`server:apps/analytics/models/upserts.py:203-243`). Retrying with the same `id` is safe. The server never generates an id.
- `session_id` absent or `null` → stateless mode: with an actor, reuse the actor's latest session within 30 minutes, else mint a new `ses_` id per event (`events.py:280-323,578-589`). `""` is NOT treated as null: every `""` event in a project merges into one session, and a second project sending `""` collides and is dropped (`server:apps/analytics/models/upserts.py:143-167`). A non-`ses_` string is accepted if ≤ 31 chars.
- `is_error` is read as `bool(...)`; the string `"false"` counts as true (`events.py:690`).
- `input_tokens`/`output_tokens`: bool or non-number → ignored; float → rounded; |v| > 2147483647 → ignored, not clamped; the server then estimates and tags `agentcat:token_source` (`server:apps/analytics/rules/usage.py:9-71`).
- `ip_address` goes into a Postgres `inet` column with no app check; `""` or an invalid value fails the insert (`server:apps/analytics/models/sessions.py:20-26`).
- `client_name` absent → the server looks for a nested `user-agent` key in `parameters`; a nested `x-anthropic-client` overrides it (`events.py:526-575`).
- Project-level server redaction is on by default and rewrites `user_intent`, `parameters`, `response`, and `error` before storage (`server:apps/core/models.py:495-498`, `events.py:631-636`).
- Error messages (top level and chained) are cut to 1000 chars (`server:apps/analytics/matching/normalization.py:698-737`).
- Tags (`events.py:378-409`): key regex `^[a-zA-Z0-9_.:\-$]+$` (NO space, unlike the SDK's regex), ≤ 32 chars, must not start with `agentcat:` (reserved, `constants.py:45-46`); value string ≤ 200, no `\n`; first 50 accepted. `agentcat_*` underscore tags are ordinary tags; no server code reads them.
- Properties (`events.py:412-460`): non-string keys, `null`s, non-finite floats, empty containers, and anything deeper than 10 levels are removed.
- Failure handling: validation failures delete the SQS message (silent drop). Processing errors retry 3 times, then go to the dead-letter queue (`server:terraform/modules/sqs/main.tf:10-13`).
- Message size: if the SQS message exceeds 1 MiB, `response`, `parameters`, and `error` are replaced with `"[removed: event exceeded MCPCat message size limit]"`; if still too large, the event is dropped (`event_publisher.py:23,84-102`).

Column limits that the server does NOT truncate (an overflow fails the insert and the event ends in the DLQ):

| Field | Limit | TS client-side cap | Gap |
|---|---|---|---|
| `id`, `session_id` | varchar(31) | none (`evt_`/`ses_` KSUIDs are 31) | a verbatim custom-event session string > 31 chars is lost |
| `event_type` | 50 | none | n/a for SDK types |
| `resource_name` | 255 | 256 + `...` = 259 (`truncation.ts:12`) | a tool name of 256+ chars is lost |
| `server_name`, `client_name` | 255 | 259 | same |
| `server_version`, `client_version`, `sdk_language`, `agentcat_version` | 50 | 259 | a version string of 51+ chars is lost |
| `identify_actor_given_id`, `identify_actor_name` | 255 | none | long actor ids are lost |

### 2.6 Response handling

| Outcome | Client behavior | SDK behavior |
|---|---|---|
| 2xx with a JSON body | `response.json()` parsed into `EventAcceptedResponse` (`api@1.0.3/src/runtime.ts:402-408`) | success log (`eventQueue.ts:230-232`) |
| 2xx with an empty or non-JSON body | `response.json()` throws | treated as a failure, so the event is retried and can be sent up to 4 times |
| non-2xx | `ResponseError` (`api@1.0.3/src/runtime.ts:137-140`) | retry with backoff (section 3) |
| network error | `FetchError` (`api@1.0.3/src/runtime.ts:219-224`) | retry with backoff |

No request timeout exists: no `AbortSignal` in `createFetchParams` (`api@1.0.3/src/runtime.ts:160-165`). A hung connection holds one of the 5 concurrency slots until the runtime's own socket timeout.

### 2.7 Schema caveats for the Rust client generator

| Caveat | Evidence | Impact |
|---|---|---|
| `public-schema.json` is YAML, not JSON | file starts with `openapi: 3.0.3` (`schema:1`); `pre-generate-script.sh:137` saves the server response under that name | feed it to the generator as YAML, or convert first |
| `event_type` enum lacks `agentcat:custom` | `schema:129-147` vs `constants.ts:3` | a strict Rust enum fails to serialize custom events; use `String` or patch the enum |
| The request body is an inline schema | `schema:100-258` | generator names it (TS: `PublishEventRequest`); the Rust name depends on the generator |
| `session_id` is `nullable: true` | `schema:115-119` | generator may emit `Option<Option<String>>` (double option); the SDK must send explicit `null` for sessionless events |
| `parameters`, `response`, `error`, `identify_data`, `properties` are `type: object` | `schema:151-153,176-190,198-202` | generated types may be `HashMap<String, Value>`; custom-event `parameters`/`response` can be any JSON in TS (`types.ts:286-287`); prefer `serde_json::Value` |
| `ip_address` has `format: ipv4` | `schema:203-206` | never sent by TS; do not send it unless validated |
| `duration`, `input_tokens`, `output_tokens` are `integer` | `schema:157-168` | TS can send a float `duration` from custom events (`types.ts:289`); Rust should send `u64`/`i64` and token counts clamped to `i32::MAX` |
| `additionalProperties: true` on the body | `schema:225` | the server tolerates extra keys, which is why the camelCase duplicates do not fail |
| `servers[0].description` says "Local server" for the production URL | `schema:469-471` | cosmetic |

### 2.8 Version note: installed vs pinned client

`node_modules/agentcat-api` resolves to `agentcat-api@1.0.1` (`node_modules/.pnpm/agentcat-api@1.0.1`), but `package.json:144` and `pnpm-lock.yaml:19-21,1058` pin `1.0.3`. Differences between 1.0.1 and 1.0.3 that matter (diff of the two `src/` trees):

- 1.0.1 has no `inputTokens`/`outputTokens` in the model or serializer. With 1.0.1 the token fields still leak through the `...value` spread as camelCase `inputTokens`/`outputTokens`, but the snake_case `input_tokens`/`output_tokens` keys are missing.
- 1.0.1 `BASE_PATH` is `http://localhost:8000`; 1.0.3 is `https://api.agentcat.com`. The SDK always passes `basePath`, so this has no effect.
- 1.0.3 adds `AlertsApi` and `NotificationsApi.verifyEmailChannel` (unused).

The local checkout needs `pnpm install` before its test results reflect the pinned client. `sdk-generator/versions.env` still says `1.0.2`.

### 2.9 Rust port notes: API client

- Use `reqwest` (with `rustls-tls`, `json`, `gzip` off by default) and one shared `reqwest::Client` (connection pool). Set an explicit timeout (for example 10 s connect, 30 s total). TS has none, which is a defect worth fixing, not copying.
- Either use the generated Rust client with a hand-written `PublishEventRequest` that has `event_type: String`, or skip the generator for this one call. The call is small: one POST with a JSON body. A hand-written `serde` struct gives exact control over `null` vs omitted.
- Send snake_case keys only. Do not reproduce the camelCase duplicates; they are a generator artifact, and the server ignores them (section 2.5).
- Send no `Authorization` header. An invalid `Token` returns `401`; nothing else uses it.
- Treat any 2xx as success without parsing the body. TS retries a 2xx whose body is not JSON, which can duplicate events (harmless only because the server de-duplicates on `id`).
- Retry only on network errors, `429`, `503`, and other `5xx`, always with the same `id`. TS retries every failure, including `400`/`401`/`415`, which cannot succeed.
- `202` does not mean stored. Validate on the client what the consumer drops silently: required `id`, `project_id` (`proj_` + 32 chars), `event_type`, `timestamp`; never send `session_id: ""` (send `null`); never send `ip_address` unless it is a valid IP; cap the varchar columns in section 2.5 (fix the TS 259-vs-255 gap by cutting to `limit - 3` before adding `...`).
- Validate tags with the server's rules, which are stricter than the TS SDK's: no space in keys, and no `agentcat:` key prefix.
- Send a `User-Agent` such as `agentcat-rust/<version> rmcp/<version>`. The TS SDK sends none, so this is additive and harmless.
- Resolve the base URL once at init with the same precedence (option → `AGENTCAT_API_URL` → `MCPCAT_API_URL` → default), trim a trailing `/`, and make it first-wins like the rest of the global state. The TS last-wins behavior (section 2.2) is accidental.

---

## 3. Event queue (`src/modules/eventQueue.ts`) and background tasks

### 3.1 Constants

| Constant | Value | Line |
|---|---|---|
| `maxRetries` | 3 (so up to 4 attempts) | `eventQueue.ts:35` |
| Backoff | `2^retries * 1000` ms: 1000, 2000, 4000 | `eventQueue.ts:238-240` |
| `maxQueueSize` | 10000 | `eventQueue.ts:36` |
| `concurrency` | 5 events in flight (whole pipeline, not just HTTP) | `eventQueue.ts:37,89` |
| Shutdown drain timeout | 5000 ms | `eventQueue.ts:268` |
| Shutdown poll interval | 100 ms | `eventQueue.ts:275` |
| Pending hook budget | 30000 ms per hook, hooks raced in parallel | `src/modules/pendingEvent.ts:13,63-72` |
| Default base URL | `https://api.agentcat.com` | `eventQueue.ts:43` |
| Event id prefix | `evt` | `eventQueue.ts:155` |

### 3.2 Algorithm

- `add(event)` (`eventQueue.ts:56-82`): creates a `delivery` promise. If the queue holds 10000 or more events, it removes the OLDEST event, settles its delivery promise, and logs `Event queue full, dropping oldest event` (`eventQueue.ts:68-71`). It then pushes the new event. If `event.projectId` is truthy, it registers `delivery` as a background task (`eventQueue.ts:77-79`). Telemetry-only events are not registered. Then it calls `process()`.
- `process()` (`eventQueue.ts:84-98`): starts `processEvent` for queued events while `activeRequests < 5`. A `processing` flag prevents re-entry. Each finished event decrements the counter and calls `process()` again (`eventQueue.ts:161-165`).
- No batching: one HTTP request per event.
- No ordering guarantee across events: up to 5 run at once, and retries delay individual events.
- `processEvent` stages are in section 5. Any stage failure drops the event with a log line. Delivery failure after the last retry logs `Failed to deliver AgentCat event after retries: …` (`eventQueue.ts:157-160`). The delivery promise always settles (`eventQueue.ts:161-162`).
- `sendEvent` (`eventQueue.ts:213-245`): calls `telemetryManager.export(event)` without awaiting it, then, only when `projectId` is truthy, POSTs to AgentCat. On failure it waits and calls itself recursively. Because the telemetry call sits at the top of `sendEvent`, every retry exports the event to telemetry again. One failing AgentCat delivery causes up to 4 exports per exporter.
- `getStats()` returns `{queueLength, activeRequests, isProcessing}` (`eventQueue.ts:252-258`). Nothing public uses it.

### 3.3 Shutdown and process hooks

| Trigger | Action | Line |
|---|---|---|
| `SIGINT`, `SIGTERM` (`process.once`) | `runSignalShutdown`: await `destroy()` and `flushDiagnostics()` in parallel (`Promise.allSettled`), then re-raise the same signal with `process.kill(pid, sig)` only if no other listener exists | `eventQueue.ts:299-319,323-326` |
| `beforeExit` (`process.once`) | `destroy()` and `flushDiagnostics()`, not awaited, no re-raise | `eventQueue.ts:327-331` |
| Edge runtimes | registration skipped when `process.once` is missing; errors swallowed | `eventQueue.ts:323-335` |

`destroy()` (`eventQueue.ts:261-283`) replaces `add` with a function that drops events and logs `Queue is shutting down, event dropped`. This is permanent for the process. It then polls every 100 ms until the queue and active requests are empty or 5000 ms pass, and logs the number of events left behind. In-flight retries are not cancelled; they continue if the process stays alive.

The re-raise exists because a Node signal listener disables the default exit. Without it, the first Ctrl+C would leave the customer's server running (`eventQueue.ts:288-298`).

### 3.4 Serverless and workerd

- `src/modules/backgroundTasks.ts:14-35`: a single optional registrar. `registerBackgroundTask(promise)` passes the delivery promise to it and swallows registrar errors.
- The workerd entry point (`src/index.workerd.ts:3-6`, export condition `workerd` in `package.json:20`) installs `waitUntil` from `cloudflare:workers`. Every AgentCat-bound event therefore extends the Worker invocation until its delivery promise settles, including the stage-0 hook wait and all retries (`eventQueue.ts:75-79`).
- The Node entry installs no registrar. On AWS Lambda or similar, events still in the queue when the handler returns can be frozen or lost. The only protection is `beforeExit`, which does not fire on a frozen Lambda.

### 3.5 Rust port notes: queue

- Global state: `static QUEUE: OnceLock<EventQueue>`; first `track()` wins, as in TS (`CLAUDE.md` "Module-level state is first-wins").
- Structure: `Mutex<VecDeque<Event>>` with capacity 10000 and drop-oldest, plus `tokio::sync::Notify` and a `Semaphore` of 5 permits. A bounded `mpsc` channel drops the NEWEST event on overflow (`try_send` fails), which changes the drop policy; use it only if that change is accepted.
- Spawn the worker on the current tokio runtime (`Handle::try_current()`). If `track()` runs outside a runtime, start a dedicated thread with its own current-thread runtime. Do not require the customer to pass a runtime handle.
- Retries: `tokio::time::sleep(Duration::from_millis(1000 << attempt))`, max 3 retries, only for retryable failures (section 2.9). Call telemetry exporters once per event, before or after the retry loop, not inside it.
- Shutdown in Rust needs a new design:
  - `Drop` cannot be async. Provide `agentcat::shutdown(timeout).await` (drain with a 5 s default) and a guard returned from `track()` whose `Drop` does a bounded blocking flush (`tokio::task::block_in_place` + `Handle::block_on`, or a channel to the worker thread with a deadline).
  - With rmcp stdio servers, `serve(...).await?.waiting().await` returns on EOF and `main` returns. The tokio runtime then drops and cancels every background task, losing queued events. Document `agentcat::shutdown(...).await` after `waiting()`. This replaces `beforeExit`.
  - Signals: do NOT install SIGINT/SIGTERM handlers by default. Registering a `tokio::signal` listener disables the default termination for the rest of the process, and tokio has no way to restore it. TS solved the same problem with a re-raise (`eventQueue.ts:288-319`). If an opt-in is offered, use `signal-hook` and `signal_hook::low_level::emulate_default_handler` after draining.
- Serverless: `lambda_runtime` has no `waitUntil`. Offer `agentcat::flush().await` for the customer to call before returning. For `workers-rs` (Cloudflare), accept a `worker::Context` and call `ctx.wait_until(...)` per AgentCat-bound event, the direct analog of `index.workerd.ts`. Gate it behind a cargo feature.
- Concurrency counter: TS relies on the single-threaded event loop for `processing` and `activeRequests` (`eventQueue.ts:34,38`). Rust needs atomics or the semaphore.

---

## 4. Telemetry exporters

### 4.1 Manager and lifecycle

- Created once, first-wins, only when `options.exporters` is set and no manager exists (`src/index.ts:252-260`). Later `track()` calls with different exporters are ignored silently.
- The record key is a free-form name. The `type` field selects the exporter: `otlp`, `datadog`, `sentry`, `posthog` (`src/modules/telemetry.ts:31-43`). Unknown types log `Unknown exporter type: …` and are skipped. Constructor errors (for example an invalid Sentry DSN) are caught and logged, and that exporter is skipped (`telemetry.ts:14-24`).
- `export(event)` calls every exporter without awaiting and logs rejections as `Telemetry export failed for <name>: <msg>` (`telemetry.ts:46-57`). OTLP rethrows its errors (so this log fires). Datadog, Sentry, and PostHog catch internally and only log.
- Exporters are ADDITIVE to AgentCat publishing. Both run for every event that passes the pipeline (`eventQueue.ts:213-245`). With `projectId` null, only exporters run (`src/index.ts:262-267,270`). With no project id and no exporters, events are processed and discarded, and a warning is logged.
- Exporters receive the post-pipeline event: redacted, sanitized, truncated, with its `evt_` id assigned (`eventQueue.ts:141-156`). They are not registered with `waitUntil` (`eventQueue.ts:75-79`).
- No exporter batches, retries, or sets a timeout. Each uses the global `fetch`.

### 4.2 Trace context (`src/modules/exporters/trace-context.ts`)

| Function | Algorithm | Line |
|---|---|---|
| `getTraceId(sessionId)` | lowercase hex of SHA-256(sessionId), first 32 chars; random 16 bytes hex if empty | `trace-context.ts:4-13` |
| `getSpanId(eventId)` | hex of SHA-256(eventId), first 16 chars; random 8 bytes hex if empty | `trace-context.ts:15-21` |
| `getDatadogTraceId(sessionId)` | decimal string of `u64` from hex chars 16..32 of the trace id | `trace-context.ts:23-26` |
| `getDatadogSpanId(eventId)` | decimal string of `u64` from the 16-hex span id | `trace-context.ts:28-31` |

All tool calls in one session share a trace id. Each event is a root span; no parent links exist.

### 4.3 OTLP (`src/modules/exporters/otlp.ts`)

Config: `{ type: "otlp", endpoint: string, headers?: Record<string,string> }` (`otlp.ts:6-10`).

| Item | Value | Line |
|---|---|---|
| Endpoint | trailing slashes trimmed; `/v1/traces` appended unless present | `otlp.ts:17-19` |
| Method and encoding | `POST`, OTLP/HTTP **JSON** (not protobuf) | `otlp.ts:22,62-69` |
| Headers | `Content-Type: application/json` merged with `config.headers` (custom headers win) | `otlp.ts:21-24` |
| Resource attributes | `service.name` = `serverName` or `"mcp-server"`; `service.version` = `serverVersion` or `"unknown"` | `otlp.ts:36-46` |
| Scope | `name: "agentcat"`, `version: agentcatVersion` or `"unknown"` | `otlp.ts:50-53` |
| Span | `traceId` (32 hex), `spanId` (16 hex), `name` = `eventType` or `"mcp.event"`, `kind: 2` (SERVER), `startTimeUnixNano` = `timestamp` ms × 1e6 as string, `endTimeUnixNano` = start + `duration` ms × 1e6, `status.code` 2 if `isError` else 1 | `otlp.ts:83-158` |
| Span attributes | `source=agentcat`, `mcp.event_type`, `mcp.session_id`, `mcp.project_id`, `mcp.resource_name`, `mcp.user_intent`, `mcp.actor_id`, `mcp.actor_name`, `mcp.client_name`, `mcp.client_version`, `agentcat.tag.<key>` per tag, `agentcat.properties` = JSON string; all `stringValue`; attributes with an empty string are removed | `otlp.ts:99-154` |
| Not exported | `parameters`, `response`, `error` details, token counts, server name as a span attribute | n/a |
| Failure | non-2xx → throws `OTLP export failed: <status> <text>`, wrapped as `OTLP export error: …` | `otlp.ts:71-80` |

### 4.4 Datadog (`src/modules/exporters/datadog.ts`)

Config: `{ type: "datadog", apiKey, site, service, env? }` (`datadog.ts:6-12`). `site` has `http(s)://` and a trailing `/` stripped (`datadog.ts:64`).

| Request | URL | Headers | Body | Line |
|---|---|---|---|---|
| Logs | `https://http-intake.logs.<site>/api/v2/logs` | `DD-API-KEY`, `Content-Type: application/json` | JSON array with one log | `datadog.ts:65,81-99` |
| Metrics | `https://api.<site>/api/v1/series` | same | `{ "series": [...] }` | `datadog.ts:66,102-120` |

Both requests are sent in parallel and awaited together (`datadog.ts:123`). Failures are logged only.

Log fields (`datadog.ts:126-189`):

| Field | Value |
|---|---|
| `message` | `"<eventType or unknown> - <resourceName or unknown>"` |
| `service` | `config.service` |
| `ddsource` | `"agentcat"` |
| `ddtags` | comma-joined: `env:<env>`, `event_type:<eventType with / → .>`, `resource:<name>`, `error:true`, `source:agentcat`, then `agentcat.<key>:<value>` per customer tag (key lowercased, runs of whitespace/`:`/`,` → `_`; value `,` → `_`) |
| `timestamp` | epoch ms |
| `status` | `error` or `info` |
| `dd.trace_id` / `dd.span_id` | decimal ids from section 4.2 |
| `mcp.*` | `session_id`, `event_id`, `event_type`, `resource`, `duration_ms`, `user_intent`, `actor_id`, `actor_name`, `client_name`, `client_version`, `server_name`, `server_version`, `is_error`, `error`, `tags`, `properties` |
| `error.message` | only when `isError` and `error`: the string, or `JSON.stringify(error)` |

Metrics (`datadog.ts:191-233`), tags `service:<service>`, `env:<env>`, `event_type:<…>`, `resource:<…>`, timestamp in epoch seconds:

| Metric | Type | Point | Condition |
|---|---|---|---|
| `mcp.events.count` | count | 1 | always |
| `mcp.event.duration` | gauge | duration ms | `duration` truthy (0 is skipped) |
| `mcp.errors.count` | count | 1 | `isError` |

Customer tags are NOT added to metric tags, which avoids cardinality growth. The root `error.message` in the log is the JSON of the whole `ErrorData` (frames included), not the message (`datadog.ts:179-186`). The duration gauge is skipped when the duration is 0 (`datadog.ts:213`).

### 4.5 Sentry (`src/modules/exporters/sentry.ts`)

Config: `{ type: "sentry", dsn, environment?, release?, enableTracing? (default false) }` (`sentry.ts:6-12`).

| Item | Value | Line |
|---|---|---|
| DSN regex | `^(https?):\/\/([a-f0-9]+)@([\w.-]+)(:\d+)?(\/.*)?\/(\d+)$` (lowercase hex key, numeric project id); a mismatch throws in the constructor, so the exporter is skipped | `sentry.ts:114-131` |
| Endpoint | `<protocol>://<host>[:port]<path>/api/<projectId>/envelope/` | `sentry.ts:104-106` |
| Auth header | `X-Sentry-Auth: Sentry sentry_version=7, sentry_client=agentcat/1.0.0, sentry_key=<publicKey>` | `sentry.ts:109,144` |
| Content type | `application/x-sentry-envelope` | `sentry.ts:145` |
| Requests per event | 1 log envelope always; plus 1 transaction if `enableTracing`; plus 1 error event if `isError`. Sent sequentially. | `sentry.ts:133-220` |

Envelopes (newline-delimited JSON):

| Envelope | Header | Item header | Payload | Trailing `\n` | Line |
|---|---|---|---|---|---|
| Log | `{event_id, sent_at}` | `{type:"log", item_count:1, content_type:"application/vnd.sentry.items.log+json"}` | `{items:[log]}` | yes | `sentry.ts:305-332` |
| Transaction | `{event_id, sent_at}` | `{type:"transaction"}` | transaction | no | `sentry.ts:503-521` |
| Error | `{event_id, sent_at}` | `{type:"event", content_type:"application/json"}` | error event | no | `sentry.ts:523-542` |

Field mapping:

- Log (`sentry.ts:222-303`): `timestamp` in epoch seconds (float), `trace_id` (32 hex), `event_id` = span id twice (32 hex, deterministic), `level` `error`/`info`, `body` `MCP <eventType>: <resourceName>`, attributes `eventType`, `resourceName`, `serverName`, `clientName`, `sessionId`, `projectId`, `duration_ms` (double), `actorId`, `actorName`, `userIntent`, `serverVersion`, `clientVersion`, `isError` (boolean), each as `{value, type}`.
- Transaction (`sentry.ts:334-369`): `event_id` = span id + random 16 hex; `timestamp` = event timestamp; `start_timestamp` = timestamp − duration; `transaction` = `<eventType> - <resourceName>`; `contexts.trace {trace_id, span_id, op: eventType, status: ok|internal_error}`; `contexts.agentcat` = properties; `tags` (`source`, `environment`, `release`, `event_type`, `resource`, `server_name`, `client_name`, `actor_id`, `agentcat.<tag>`); `extra` (`session_id`, `project_id`, `user_intent`, `actor_name`, `server_version`, `client_version`, `duration_ms`, `error`).
- Error event (`sentry.ts:425-501`): `exception.values[0]` = `{type: error.type or "ToolCallError", value: error.message or JSON of error, mechanism: {type:"mcp_tool_call", handled:false}}`; `level: "error"`; `contexts.mcp {resource_name, session_id, event_type, user_intent}`; trace context reuses the transaction trace and sets `parent_span_id` when tracing is on. Stack frames are NOT sent.

Defects:

- The TS `timestamp` is the call START time (`callWrap.ts:168,243`), but the transaction uses it as the END time and subtracts the duration (`sentry.ts:336-342`). The error event copies the same value (`sentry.ts:454-458`). Sentry spans therefore appear shifted earlier by one duration. The Rust port must use `start = timestamp`, `end = timestamp + duration`. OTLP does this correctly (`otlp.ts:84-90`).
- With `enableTracing`, the error event's `parent_span_id` equals its own `span_id` (`sentry.ts:452,481`), and the transaction is built twice with different random ids (`sentry.ts:161,191`).
- Transaction and error `event_id`s have a random half (`sentry.ts:354,462`), so a re-export (section 3.2) creates a new Sentry event each time.
- For `CallToolResult`-derived and non-Error errors, `type` is undefined and the key is dropped by truncation, so Sentry always reports `ToolCallError` (`sentry.ts:431,442`).

### 4.6 PostHog (`src/modules/exporters/posthog.ts`)

Config: `{ type: "posthog", apiKey, host? (default "https://us.i.posthog.com"), enableAITracing? (default false) }` (`posthog.ts:71-84`).

| Item | Value | Line |
|---|---|---|
| Endpoint | `<host without trailing />/batch` | `posthog.ts:101-102` |
| Headers | `Content-Type: application/json` only; the key is in the body | `posthog.ts:132-141` |
| Body | `{ "api_key": <apiKey>, "batch": [ ...capture events ] }` | `posthog.ts:137-140` |
| Batch contents | 1 capture event always; + `$exception` if `isError && error`; + `$ai_span` if `enableAITracing` and type is `mcp:tools/call` | `posthog.ts:110-126` |
| `distinct_id` | `identifyActorGivenId`, else `sessionId`, else `"anonymous"` | `posthog.ts:61-63` |
| `timestamp` | `timestamp.toISOString()` | `posthog.ts:65-69` |
| Failure | logged only | `posthog.ts:143-153` |

Event name mapping (`posthog.ts:327-343`): `mcp:tools/call` → `mcp_tool_call`, `mcp:tools/list` → `mcp_tools_list`, `mcp:initialize` → `mcp_initialize`, `mcp:resources/read` → `mcp_resource_read`, `mcp:resources/list` → `mcp_resources_list`, `mcp:prompts/get` → `mcp_prompt_get`, `mcp:prompts/list` → `mcp_prompts_list`. Anything else → `mcp_` + type with a leading `mcp:` removed and `/` → `_`. So `agentcat:custom` becomes `mcp_agentcat:custom`.

Capture event properties (`posthog.ts:156-225`): `source`, `$session_id` (UUIDv7 of `sessionId`, omitted when sessionless), `resource_name`, `tool_name` (tool calls only), `duration_ms`, `server_name`, `server_version`, `client_name`, `client_version`, `project_id`, `user_intent`, `is_error`, `parameters`, `response`, `$set` (`name` = actor name, then every `identifyActorData` key), then every tag, then every property. Tags and properties override SDK keys.

`$exception` (`posthog.ts:227-269`): `$exception_source: "backend"`, `$session_id`, `$exception_message`, `$exception_type`, `$exception_stacktrace` (raw `error.stack`), `resource_name`, `tool_name`, server and client name/version.

`$ai_span` (`posthog.ts:271-325`): `$ai_trace_id` = UUIDv7 of `sessionId || id`, `$ai_span_id` = UUIDv7 of `id`, `$ai_span_name` = resource or `"unknown_tool"`, `$ai_is_error`, `source`, `$ai_session_id` = `"agentcat_" + sessionId`, `$session_id`, `$ai_latency` (seconds), `$ai_error`, `$ai_input_state` = parameters, `$ai_output_state` = response, `server_name`, `client_name`, then tags and properties (which can override `$ai_*`).

`toUUIDv7(prefixedId)` (`posthog.ts:13-59`), deterministic:

1. `h = SHA-256(prefixedId)` (full string, prefix included).
2. Strip `^[a-z]+_`. Parse the rest as a KSUID. On success, `ts_ms` = KSUID time (seconds × 1000 + KSUID epoch 1400000000000, `src/thirdparty/ksuid/index.js:14,74`). On failure, `ts_ms` = big-endian 48-bit integer of `h[10..16]`.
3. Bytes: `[0..6]` = `ts_ms` BE; `[6]` = `0x70 | (h[0] & 0x0f)`; `[7]` = `h[1]`; `[8]` = `0x80 | (h[2] & 0x3f)`; `[9..16]` = `h[3..10]`.
4. Format as lowercase 8-4-4-4-12.

Golden vectors for `toUUIDv7` and the trace ids are in section 7.11.

Notes:

- The JSDoc says each tool call is its own trace (`posthog.ts:76-78`), but `$ai_trace_id` is per session (`posthog.ts:278`), and the tests assert per session (`src/tests/posthog-exporter.test.ts:497-547`). Follow the code.
- Sessionless trace ids differ across exporters: OTLP, Datadog, and Sentry use a random trace id per event (`trace-context.ts:5-7`), and PostHog falls back to the event id (`posthog.ts:278`).
- PostHog sends no `uuid` per capture event, so it cannot de-duplicate the re-exports from section 3.2.
- `$exception` gets no tags or properties; the capture event and `$ai_span` do.

### 4.7 Rust port notes: exporters

- Put each exporter behind a cargo feature (`otlp`, `datadog`, `sentry`, `posthog`) to keep the default dependency tree small.
- Share one `reqwest::Client` with timeouts. Spawn each export as a task with a bounded `JoinSet` or semaphore so a slow vendor cannot grow memory without limit.
- OTLP: the TS exporter hand-writes OTLP/HTTP JSON. Two options:
  - Parity: hand-write the same JSON with `serde_json` (no `opentelemetry` dependency, same attribute names).
  - Native: `opentelemetry`, `opentelemetry_sdk`, `opentelemetry-otlp` (`http-json` or `http-proto`) with a `BatchSpanProcessor`. This needs explicit trace and span ids (custom `IdGenerator`, or build `SpanData` directly) to keep the session-derived trace id, and needs `provider.shutdown()` at exit.
  - Do not install a global tracer provider or a `tracing` subscriber from the library; that belongs to the customer.
- Hashing: `sha2` + `hex`; Datadog decimal ids with `u64::from_str_radix(&hex[16..32], 16)`. UUIDv7: build the 16 bytes by hand (the `uuid` crate's v7 builder uses the clock, not a hash); format with `uuid::Uuid::from_bytes`.
- KSUID parsing for `toUUIDv7` and `evt_` generation: the `ksuid` crate, or a small base62 implementation. Match the 27-char base62 alphabet `0-9A-Za-z` (`src/modules/handles.ts:73-75`).
- Sentry: hand-write the envelopes; the `sentry` crate installs global hubs and panic hooks, which an embedded library must not do. Fix the transaction timestamp defect (section 4.5).
- Datadog: plain `reqwest` POSTs; keep metric tags free of customer tags.
- Fix, not copy: duplicate exports on API retry (section 3.2).

---

## 5. Data-safety pipeline (order of application)

| Order | Stage | Function | Input | Failure behavior | Line |
|---|---|---|---|---|---|
| capture | token estimates | `estimateInputTokens`, `estimateOutputTokens` | raw args / raw result | field omitted | `callWrap.ts:241,321` |
| capture | error capture | `captureException` | thrown value or `isError` result | n/a (pure) | `callWrap.ts:308-316,351` |
| capture | extra projection | `projectExtraForEvent` | handler context | publishes `extra` as-is | `callWrap.ts:238` |
| 0 | deferred hooks | `applyPendingEventFields` | `event.pending` | hook result omitted; event continues | `eventQueue.ts:106-114` |
| 1 | event hook | `applyEventRedaction` (`redactEvent`) | full event minus functions | `null` → drop; throw → drop | `eventQueue.ts:116-128` |
| 2 | string redaction | `redactEvent` (`redactSensitiveInformation`) | every unprotected string | throw → drop event | `eventQueue.ts:130-139` |
| 3 | sanitization | `sanitizeEvent` | `response`, `parameters` | throw → drop event | `eventQueue.ts:141-146` |
| 4 | truncation | `truncateEvent` | whole event | throw → drop event | `eventQueue.ts:148-153` |
| 5 | id | KSUID `evt_` | n/a | n/a | `eventQueue.ts:155` |

Customer-visible contract: `redactEvent` sees raw values with hook results applied. The string-level hook, sanitization, and truncation run on the output of `redactEvent` (`src/index.ts:48`). Token counts are computed before any of this and are never recomputed (`token-spec:38-40,150-158`).

### 5.1 Stage 0: deferred hooks (`src/modules/pendingEvent.ts`)

- The four hooks (`resolveSessionId`, `identify`, `eventTags`, `eventProperties`) are started at request time (`callWrap.ts:211-227`). Their promises never reject: the resolvers catch and return `null` (`internal.ts:36-43,56-61,75-80`, `handles.ts:329-346`).
- Stage 0 awaits all four in parallel, each raced against 30000 ms. On timeout it logs `Warning: <label> hook did not settle within 30000ms; publishing event without it` (`pendingEvent.ts:23-48,63-72`).
- Results (`pendingEvent.ts:74-100`):
  - `resolveSessionId`: a non-empty trimmed string → `deriveSessionId(value, projectId)`, source `hook`; otherwise a fresh random `ses_` id, source `minted` (`handles.ts:358-369`). The tag `agentcat_session_id_source` is overwritten.
  - `identify`: sets `identifyActorGivenId`, `identifyActorName`, `identifyActorData` (`userData || {}`).
  - `eventTags`: validated at resolution (`internal.ts:39`), merged under SDK tags.
  - `eventProperties`: replaces `properties`.
- Custom events have no `pending` (`src/index.ts:410-429`), so `identify` and the tag/property hooks never run for them.

### 5.2 Stage 1: `redactEvent` hook (`src/modules/redaction.ts:171-199`)

- Input: the event without `redactionFn` and `eventRedactionFn` (`redaction.ts:175`). `pending` is already removed. `id` is `""` at this point.
- `null` or `undefined` → event dropped, log `Event dropped by redactEvent hook` (`eventQueue.ts:120-123`). A throw → dropped, log `Failed to redact event (event-level hook): …` (`eventQueue.ts:124-127`).
- Restored after the hook, so customer changes are ignored: `id`, `sessionId`, `projectId`, `eventType`, `timestamp` (`redaction.ts:148-154,185-192`). If the original value was `undefined`, the key is deleted.
- The event object is cleared and refilled in place, so keys the hook deleted stay deleted (`redaction.ts:194-197`). `redactionFn` is put back so stage 2 still runs (`redaction.ts:182`).
- Source of the hook: `eventInput.eventRedactionFn ?? data.options.redactEvent` (`eventQueue.ts:423`). Custom events through a tracked server get the hook; custom events with a bare session string do not.

### 5.3 Stage 2: string redaction (`src/modules/redaction.ts:13-141`)

- Applies the customer `redactSensitiveInformation(text) → Promise<string>` to every string in the event, recursively, sequentially (one `await` per string, `redaction.ts:65,76,111`).
- Protected TOP-LEVEL keys, whose whole subtree is skipped (`redaction.ts:13-30`): `sessionId`, `id`, `projectId`, `server`, `serverName`, `serverVersion`, `clientName`, `clientVersion`, `identifyActorGivenId`, `identifyActorName`, `identifyData`, `resourceName`, `eventType`, `actorId`, `tags`, `properties`.
- Not protected, so redacted: `parameters`, `response`, `userIntent`, `eventId`, `error` (message, type, stack, frame filenames, functions, and context lines, chained errors, platform), `identifyActorData` (the protected list names the legacy `identifyData`, and the live field is `identifyActorData`), `sdkLanguage`, `agentcatVersion`, `ipAddress`. Tests pin `userIntent`, `eventId`, and `error.*` (`src/tests/redaction.test.ts:28-39,261-279,383-431`). No test pins `identifyActorData`; a probe confirmed it is redacted (section 7.6).
- Content-block `type` discriminators are strings too, so they reach the redactor before sanitization runs (section 7.6).
- Skipped values: `null`/`undefined`, functions, `undefined` object values (removed from the clone), `Date` (kept), numbers and booleans (`redaction.ts:55-124`).
- Cycles: a `WeakMap` of clones registered before descent, so back-edges resolve to the clone (`redaction.ts:41-46,70-74,94-98`).
- Returns a new object; the queue merges it with `Object.assign` (`eventQueue.ts:132-134`).
- Applies only when `redactionFn` is set on the event. `publishEvent` copies `eventInput.redactionFn` (`eventQueue.ts:422`), and only `callWrap` sets it (`callWrap.ts:244`). Custom events are never string-redacted, even with a tracked server.

### 5.4 Stage 3: sanitization (`src/modules/sanitization.ts`)

| Target | Rule | Replacement | Line |
|---|---|---|---|
| `response.content[]` `type:"text"` | kept | n/a | `sanitization.ts:61-62` |
| `type:"image"` | replaced | `{type:"text", text:"[image content redacted - not supported by AgentCat]"}` | `sanitization.ts:64-68` |
| `type:"audio"` | replaced | `{type:"text", text:"[audio content redacted - not supported by AgentCat]"}` | `sanitization.ts:70-74` |
| `type:"resource"` with `resource.blob` | replaced | `{type:"text", text:"[binary resource content redacted - not supported by AgentCat]"}` | `sanitization.ts:95-101` |
| `type:"resource"` with text | kept | n/a | `sanitization.ts:102` |
| `type:"resource_link"` | kept | n/a | `sanitization.ts:79-80` |
| any other `type` (including missing) | replaced | `{type:"text", text:"[unsupported content type \"<type>\" redacted - not supported by AgentCat]"}` | `sanitization.ts:82-86` |
| `response.structuredContent` (object) | base64 scan | see next row | `sanitization.ts:42-47` |
| any string in `parameters` (recursive) with length ≥ 10240 matching `^[A-Za-z0-9+/\n\r]+=*$` | replaced | `"[binary data redacted - not supported by AgentCat]"` | `sanitization.ts:3-4,109-138` |

Not sanitized: `error`, `properties`, `identifyActorData`, `userIntent`, tags, and response keys other than `content` and `structuredContent`. Non-object `response` passes unchanged. Null content blocks pass unchanged (`sanitization.ts:56-58`). A resource with `blob: null` counts as a blob (`!== undefined`). The base64 regex rejects URL-safe base64 (`-`, `_`) and `data:` URIs, so those are not redacted. The size gate uses `.length` (UTF-16 units) with `>=`. Sanitization has no cycle guard: a cyclic `parameters` overflows the stack and the queue drops the event.

### 5.5 Stage 4: truncation (`src/modules/truncation.ts`)

Constants:

| Constant | Value | Line |
|---|---|---|
| `MAX_DEPTH` | 10 | `truncation.ts:4` |
| `MAX_BREADTH` | 100 | `truncation.ts:5` |
| `MAX_STRING_LENGTH` | 32768 | `truncation.ts:6` |
| `MAX_EVENT_BYTES` | 102400 (UTF-8 bytes of `JSON.stringify(event)`) | `truncation.ts:7,227-231` |
| `MAX_USER_INTENT_LENGTH` | 2048 | `truncation.ts:10` |
| `MAX_ERROR_MESSAGE_LENGTH` | 2048 | `truncation.ts:11` |
| `MAX_RESOURCE_NAME_LENGTH` | 256 | `truncation.ts:12` |
| `MAX_METADATA_LENGTH` | 256 (server and client name/version) | `truncation.ts:13` |
| `MAX_STACK_FRAMES` | 50 (keep first 25 + last 25) | `truncation.ts:14,194-200` |
| `MAX_CONTENT_TEXT_LENGTH` | 32768 per text block | `truncation.ts:15,202-222` |
| Suffix | `...` appended after cutting (result is limit + 3) | `truncation.ts:18,185-192` |

Algorithm (`truncation.ts:355-406`):

1. Field limits: `userIntent` 2048, `resourceName` 256, `serverName`/`serverVersion`/`clientName`/`clientVersion` 256, `error.message` 2048, `error.frames` 25 + 25.
2. Each `response.content[]` text block over 32768 is cut.
3. `normalize()` with depth 10, breadth 100, string 32768 on `parameters`, `response`, `identifyActorData`, `error`.
4. If the JSON is over 102400 bytes: retry `normalize()` on the same four fields with depth 9, 8, …, 1, keeping the first result that fits (`truncation.ts:316-332`).
5. Last resort: depth 1 on the four fields, then `truncateLargestFields` (`truncation.ts:334-345,238-276`): up to 10 passes. Each pass collects all strings longer than 100 chars anywhere in the event, sorted by length descending. It cuts each by `min(remaining, floor(len × 0.5))`, skipping reductions under 10, with `remaining = excess + 200`. It stops when the event fits or no progress is possible. This pass can cut `tags`, `properties`, and `parameters` strings alike.

`normalize()` markers (`truncation.ts:39-181`):

| Input | Output |
|---|---|
| string over the limit | first N chars + `...` |
| `undefined` (array element or top-level) | `"[undefined]"`; `undefined` object values are omitted |
| `NaN`, `Infinity`, `-Infinity` | `"[NaN]"`, `"[Infinity]"`, `"[-Infinity]"` |
| BigInt | `"[BigInt: <n>]"` |
| Symbol | `"[Symbol(<desc>)]"` or `"[Symbol()]"` |
| function | `"[Function: <name or <anonymous>>]"` |
| Date | ISO string, or `"[Invalid Date]"` |
| circular reference | `"[Circular ~]"` |
| depth exhausted | `"[Array]"` or `"[Object]"` |
| array over breadth | first 100 items + `"[MaxProperties ~]"` |
| object over breadth | first 100 defined keys + key `"..."` = `"[MaxProperties ~]"` |

Not normalized: `tags`, `properties`, `userIntent` (cut only by step 1), and scalar metadata. Properties can therefore carry unbounded depth until the last-resort pass.

Edge cases:

- `truncateLargestFields` starts with `structuredClone(event)` (`truncation.ts:239`). If `properties` contain a function or Symbol, `structuredClone` throws and the event is dropped with `Failed to truncate event` (`eventQueue.ts:148-153`). This happens only on the last-resort path. A BigInt or a cycle in `properties` fails earlier, at the first `JSON.stringify` (`truncation.ts:318`), for every event size.
- The 102400-byte budget is best effort, not a cap. Only strings longer than 100 chars are candidates, and `tags`/`properties` are never depth-limited. A probe pushed a 1,089,886-byte event through.
- The size check measures the internal event (`identifyActorData`, `sessionId: ""`, camelCase keys), not the wire body. The wire body is larger because of the duplicated keys (section 2.4).
- Only top-level `error.frames` is capped at 50. `chained_errors[].frames` and `error.stack` are limited only by `normalize()` (breadth 100, 32768 chars).

### 5.6 Token estimates (`src/modules/tokenEstimate.ts`)

| Item | Rule | Line |
|---|---|---|
| Divisor | `TOKEN_ESTIMATE_BYTES_PER_TOKEN = 3.5` | `tokenEstimate.ts:13` |
| `estimateTokens(bytes)` | `0` if `bytes ≤ 0`; else `min(ceil(bytes / 3.5), 2147483647)` | `tokenEstimate.ts:45-51` |
| Byte count | UTF-8 length; a lone surrogate counts 3 bytes | `tokenEstimate.ts:19-42` |
| Input | compact `JSON.stringify(arguments)`; `undefined` when arguments are `null`/absent or not serializable | `tokenEstimate.ts:58-82` |
| Output, `content` is an array with items | sum over blocks: `type:"text"` → `text` bytes; `type:"resource"` with string `resource.text` → those bytes; anything else 0; one `ceil` over the sum | `tokenEstimate.ts:108-137` |
| Output, `content` is `[]` | `structuredContent` present → its compact JSON bytes; else 0 | `tokenEstimate.ts:99-107` |
| Output, no `content` array | compact JSON of the whole response | `tokenEstimate.ts:95-98` |
| Output, response `null`/absent | `undefined` (field omitted) | `tokenEstimate.ts:93` |
| Never throws | `try/catch` → `undefined` | `tokenEstimate.ts:75-81,92-115` |

Cross-SDK golden vectors (`token-spec:226-271`). The Rust test suite must carry them verbatim:

| Input (arguments) | Bytes | Tokens |
|---|---|---|
| `{"q":"hello"}` | 13 | 4 |
| `{}` | 2 | 1 |
| `{"name":"café"}` | 16 | 5 |
| `{"html":"<a>&</a>"}` | 19 | 6 |
| `{"t":"你好"}` | 14 | 4 |
| `{"ids":[1,2,3],"opts":{"deep":true,"n":null}}` | 45 | 13 |
| absent | n/a | omitted |

| Output (content) | Bytes | Tokens |
|---|---|---|
| one text block `"hello world"` | 11 | 4 |
| text blocks `"abcd"` and `"e"` | 5 | 2 |
| one text block `""` | 0 | 0 |
| one image block | 0 | 0 |
| resource block, `resource.text` = `"resource body"` | 13 | 4 |
| resource block, blob only | 0 | 0 |
| text `"hi"` plus large `structuredContent` | 2 | 1 |
| empty content plus `structuredContent` `{"result":"ok"}` | 15 | 5 |
| empty content, no `structuredContent` | 0 | 0 |
| response `{"result":"ok"}` (no `content`) | 15 | 5 |
| response absent | n/a | omitted |
| one text block of 4096 `x` | 4096 | 1171 |

| Estimator bytes | Tokens |
|---|---|
| 0 | 0 |
| 1 | 1 |
| 7 | 2 |
| ≥ 2147483647 × 3.5 | 2147483647 |

### 5.7 Exceptions (`src/modules/exceptions.ts`)

`ErrorData` wire shape (`src/types.ts:255-279`):

| Field | Type | Notes |
|---|---|---|
| `message` | string | required |
| `type` | string? | `error.name`, else constructor name |
| `stack` | string? | raw `Error.stack` |
| `frames` | `StackFrame[]`? | parsed, max 50 |
| `chained_errors` | `ChainedErrorData[]`? | `{message, type?, stack?, frames?}`, max 10 |
| `platform` | string? | always `"javascript"` |

`StackFrame`: `filename`, `function`, `lineno?`, `colno?`, `in_app`, `abs_path?`, `context_line?` (`types.ts:255-263`). Keys are snake_case in memory and on the wire.

`captureException(error)` (`exceptions.ts:41-78`):

| Input | Result |
|---|---|
| object with `isError` and an array `content` (a `CallToolResult`) | `message` = text blocks joined with a space and trimmed, or `"Unknown error"`; `type` undefined; no stack (`exceptions.ts:764-805`) |
| non-`Error` value | `message` = `stringifyNonError` (`"null"`, `"undefined"`, the string, `String(n)`, or `JSON.stringify` with a `String()` fallback); no type (`exceptions.ts:50-57,820-843`) |
| `Error` | `message`, `type`, `stack`, `frames` = `parseV8StackTrace(stack)`, `chained_errors` from `error.cause` (`exceptions.ts:59-77`) |

Stack parsing (`exceptions.ts:99-370`):

- Only lines starting with `at ` (after trim) are frames. Frame formats: `fn (loc)`, bare `loc`, else an unparseable frame `{function: <line>, filename: "<unknown>", in_app: false}`.
- Locations: `file:line:col`, `native`, `unknown location` (→ `<unknown>`), and nested `eval at …` chains resolved to the innermost file (`exceptions.ts:176-308`).
- `in_app` is false for paths containing `/node_modules/` or `\node_modules\`, paths starting with `node:`, and `native`/`<unknown>`; true otherwise (`exceptions.ts:383-403`).
- `context_line`: for in-app frames, the source line read synchronously from `abs_path` (Node only) (`exceptions.ts:138-162`).
- `filename` normalization for grouping (`makeRelativePath`, `exceptions.ts:629-699`): strip `file://`; keep relative paths; collapse `node:internal/*` → `node:internal` and `node:fs/promises` → `node:fs`; reduce `…/node_modules/x` to `node_modules/x`; replace `/Users/<u>/`, `/home/<u>/`, `C:\Users\<u>\` with `~/`; strip `/var/www/<x>/`, `/var/task/`, `/usr/src/app/`, `/app/`, `/opt/<x>/`, `/srv/<x>/`; strip `process.cwd()`; then cut from the last `/src/`, `/lib/`, `/dist/`, `/build/` (primary) or `/app/`, `/components/`, `/pages/`, `/api/`, `/utils/`, `/services/`, `/modules/` (secondary); drop a leading `/`. `abs_path` keeps the original.
- Cause chain: follows `error.cause` up to 10 levels, stops on cycles, and stringifies a non-`Error` cause as the last entry (`exceptions.ts:713-753`). Chained `type` defaults to `"Error"`.

Classification summary: section 1.4.

How the server groups errors (`server:apps/analytics/matching/`, `server:apps/analytics/schemas/error_data.py:8-75`). The grouping runs on the stored error, after server-side truncation and redaction.

| Aspect | Server behavior |
|---|---|
| Schema | `ErrorData {message: str (required), type?, stack?, platform?, frames?, chained_errors?}`; `StackFrame {filename: str (required), function: str (required), lineno?: int, colno?: int, in_app: bool = true, abs_path?, module?, context_line?}`; extra fields ignored |
| Parse failure | any invalid frame (missing or `null` `filename`/`function`, `in_app: null`, float `lineno`), `message: null`, or a non-string `platform` → the whole error is unparsed and the issue instance gets no Issue (`server:apps/analytics/matching/core.py:409-428`) |
| Components per exception | `type` first, then frame contributions; the message (normalized, first 200 chars) is used only when no frame contributes (`strategies/chained_exception.py:169-188`) |
| Chained errors | flattened after the primary error; duplicates removed by component hash (`strategies/base.py:59-109`, `chained_exception.py:261-290`) |
| `platform` | only `"javascript"`, `"node"`, `"js"` (case-sensitive) select the JS rules (`chained_exception.py:233`); every other value, including `"rust"` and `"go"`, uses the non-JS rules |
| JS frame contribution | skip functions `<anonymous>`, `?`, `anonymous`, `""`; needs a filename; uses `context_line` if ≤ 120 chars, else the function name (`chained_exception.py:292-323`) |
| Non-JS frame contribution | `module\|function` if `module` is set; else `basename(filename).lower()\|function`; else skipped (`chained_exception.py:241-259`, `normalization.py:397-425`) |
| `in_app` | taken only from the SDK (default true); the app variant uses in-app frames, the system variant all frames; an issue matches on any variant (`strategies/base.py:111-113`, `core.py:117-165`) |
| Never used for grouping | `stack`, `lineno`, `colno`, `abs_path` |
| Hash | `md5(json.dumps(components, separators=(",",":")))`, grouping version `v1` (`server:apps/analytics/matching/hashing.py:6-17`) |
| Issue title and source | `Type: first line` (type ≤ 80, message ≤ 80, total ≤ 150); `source_function` (varchar 255) and `source_file` (varchar 500) from the FIRST in-app frame in list order; an overflow fails Issue creation (`title.py:44-86`, `core.py:168-186`) |

Server-side redaction (on by default) damages Rust-looking strings. Probe results by the server agent with the repo's `normalize_sensitive`: `src/main.rs` → `src/<hostname>` (the `.rs` TLD matches the hostname pattern), and `::` matches the compressed-IPv6 pattern, so `my_crate::tools::search` → `my_crate<ip>tools<ip>search` and `std::io::Error` → `std<ip>io<ip>rror`. Grouping stays stable (the rewrite is deterministic) but titles and `source_function` look garbled.

### 5.7.1 What the Rust SDK must put in `error`

- `platform: "rust"` (the Go SDK sends `"go"`, `/home/ubuntu/agentcat-go-sdk/internal/exceptions/exceptions.go:16`). Never `javascript`/`node`/`js`. ≤ 20 chars.
- `message`: non-null string; cut to 1000 chars client-side to match what the server keeps.
- `type`: a short type name without `::` (for example `ParseIntError`, `McpError(-32602)`, `panic`). Use the last path segment of `std::any::type_name`.
- `frames[]`: every frame needs string `filename` and `function` and a boolean `in_app`; `lineno`/`colno` integers or omitted. One bad frame cancels grouping for the whole error.
- Set `module` on every frame (crate and module path, for example `my_crate::tools`). For non-JS platforms `module` wins over `filename`, which avoids the `.rs` → `<hostname>` collapse. The TS `StackFrame` has no `module` field (`src/types.ts:255-263`); the server schema does.
- Strip `::h<16 hex>` hashes and `{{closure}}` markers so contributions are stable across builds.
- Mark std, core, alloc, tokio, futures, rmcp, and other dependency frames `in_app: false`.
- Keep frame order innermost first, like V8 and the TS SDK, because the server takes the first in-app frame as the issue source.
- `stack`: the `Display` of the backtrace; it is shown, not grouped.
- `chained_errors`: from `source()`, outer to inner, max 10, each with `message` and `type`.

### 5.8 Rust port notes: data safety

- Run the pipeline on `serde_json::Value` trees. A `Value` cannot hold functions, `undefined`, `NaN`, symbols, BigInt, or cycles, so most `normalize()` markers are unreachable. Keep string cutting, `[Object]`/`[Array]` at depth, and `[MaxProperties ~]` breadth markers. `structuredClone` becomes `Value::clone`, and the clone-failure edge case disappears.
- String length: JS `.length` and `.slice` count UTF-16 code units and can split a surrogate pair. Rust strings are UTF-8. Choose one rule and document it. The closest parity is to count UTF-16 units (`s.encode_utf16().count()`) and cut at the nearest `char` boundary at or below the limit. Counting `chars()` differs only for non-BMP text. Never slice bytes at a non-boundary (`&s[..n]` panics).
- `jsonByteSize`: `serde_json::to_vec(&v).len()`. `serde_json` and `JSON.stringify` escape the same characters for valid strings (neither escapes `/` or non-ASCII). Floats can differ (`1.0` vs `1`); this is within tolerance.
- Key order: enable the `serde_json` `preserve_order` feature. Breadth truncation keeps "the first 100 keys", and `truncateLargestFields` sorts candidates by length with ties in traversal order. With the default `BTreeMap`, both depend on alphabetical order instead.
- Redaction hook type: `Arc<dyn Fn(String) -> BoxFuture<'static, String> + Send + Sync>`. Event hook: `Arc<dyn Fn(Event) -> BoxFuture<'static, Option<Event>> + Send + Sync>`. Run each with `catch_unwind` (`futures::FutureExt::catch_unwind` + `AssertUnwindSafe`). A panic must drop the event, the analog of a throw.
- Decide deliberately about `identifyActorData` redaction. TS redacts it because the protected list names the legacy key, and no test pins it. Recommendation: protect it (the customer chose to send that data in `identify`), keep a test that pins the choice, and report the gap to the TS owners.
- Redact only the fields that need it, and skip known discriminators. Running the customer redactor on content-block `type` values and on SDK metadata (`sdk_language`, `agentcat_version`) gains nothing and can corrupt sanitization. If parity wins, document it.
- Hook timeouts: `tokio::time::timeout(Duration::from_secs(30), hook)` per hook, joined with `futures::join!`.
- Base64 check: a byte loop is enough (`[A-Za-z0-9+/\r\n]+` then `=*`); no `regex` dependency needed. Apply the gate on length ≥ 10240.
- Token estimation: `serde_json::to_vec(&args)` gives compact output without HTML escaping, matching `JSON.stringify` (`token-spec:78-84`). Rust strings cannot hold lone surrogates, so that rule is moot. In rmcp, `CallToolRequestParam.arguments` is `Option<JsonObject>`: `None` → omitted, `Some({})` → 1 token.
- Exceptions need a new design in Rust (no `Error.stack`, no thrown values):
  - `Err(e)` from a tool: `message` = `e.to_string()`; `type` = the last path segment of the concrete type name when known (`std::any::type_name::<E>()` in a generic wrapper) or the rmcp `ErrorCode`, never a `::` path (section 5.7.1); `chained_errors` from `std::error::Error::source()`, max 10.
  - `Ok(CallToolResult { is_error: Some(true), .. })`: same as `captureCallToolResultError`, text blocks joined with a space.
  - Panics: wrap the handler future in `catch_unwind`. Install a chained panic hook (`std::panic::take_hook` + `set_hook`) that stores `Backtrace::force_capture()` and `PanicHookInfo::location()` in a slot the wrapper reads, then `resume_unwind` so the customer's behavior does not change. Message from the payload (`&str` or `String` downcast), type `"panic"`. Note that rmcp runs handlers in spawned tasks; check what the pinned version does with a panicking handler before deciding to resume.
  - Backtraces: `std::backtrace::Backtrace` gives only a `Display` string on stable. The `backtrace` crate gives structured frames (symbol name, filename, line, column). Frame `in_app` rules for Rust: false for paths under `~/.cargo/registry/`, `~/.cargo/git/`, `/rustc/<hash>/library/`, and symbols starting with `std::`, `core::`, `alloc::`, `tokio::`, `futures::`, `rmcp::`, `agentcat::`. Strip the closure and hash suffixes (`::{{closure}}`, `::h<16 hex>`) for stable grouping.
  - `anyhow::Error::backtrace()` and `eyre` reports carry the origin backtrace when `RUST_BACKTRACE`/`RUST_LIB_BACKTRACE` is set. Use it when the customer's error type exposes one; otherwise send no frames rather than the wrapper's own stack.
  - `context_line`: reading source files at runtime is rarely possible for a compiled binary, and the server ignores it for non-JS platforms. Omit it.
  - Set `module` on every frame and keep the field rules of section 5.7.1; otherwise the server cannot group Rust errors well.

---

## 6. `publishCustomEvent` and other direct emitters

`publishCustomEvent(serverOrSessionId, projectId, eventData?)` (`src/index.ts:364-451`) is the only public API that emits events directly. `track()` returns the server; there is no public `identify` or `flush`.

| Aspect | Tracked server passed | Session-id string passed |
|---|---|---|
| `projectId` argument | must be truthy (`index.ts:370-372`), but the event uses the project id from `track()` (`eventQueue.ts:390`); the argument value is ignored | used as the event project id (`index.ts:413`) |
| Untracked server object | throws `Server is not tracked. Please call agentcat.track() first or provide a session ID string.` (`index.ts:385-390`) | n/a |
| Other first-argument types | throws `First parameter must be either an MCP server object or a session ID string` (`index.ts:403-407`) | same |
| High-level server | unwrapped through `.server` (`index.ts:382-384`) | n/a |
| Session id | `eventData.sessionId`, else `""` (wire `null`) with a log line (`index.ts:391-399`) | `eventData.sessionId \|\| string`, verbatim, no derivation (`index.ts:402`) |
| Event type | `agentcat:custom` | `agentcat:custom` |
| Path | `publishEvent` (`index.ts:441-442`): requires `enableTracing` (custom events are dropped when it is false, `eventQueue.ts:365-367`), adds `sdkLanguage`, versions, server info, and `getClientVersion()` client info | `eventQueue.add` directly (`index.ts:443-446`): no session info, no `sdk_language`, no server or client fields |
| `redactEvent` | applied (`eventQueue.ts:423`) | not applied |
| `redactSensitiveInformation` | NOT applied (no `redactionFn`, `eventQueue.ts:422`) | not applied |
| Sanitize and truncate | applied | applied |
| `identify`, `eventTags`, `eventProperties` hooks | not run | not run |
| `identifyActorData` | `{}` (from `buildSessionInfo`) | absent |
| `duration` | `eventData.duration`, else `now - timestamp` (≈ 0) | `eventData.duration` or absent |

Field mapping from `CustomEventData` (`src/types.ts:282-294`, `index.ts:410-437`):

| `CustomEventData` | Event field |
|---|---|
| `sessionId` | `sessionId` (wins over the string argument) |
| `resourceName` | `resourceName` |
| `parameters` | `parameters` (any JSON) |
| `response` | `response` (any JSON) |
| `message` | `userIntent` |
| `duration` | `duration` |
| `isError` | `isError` |
| `error` | `error`, raw: not passed through `captureException` |
| `tags` | `validateTags(tags)` (null if none valid) |
| `properties` | `properties` if it has at least one key |

The function is `async` but resolves when the event is queued, not delivered. It writes `Published custom event … with type 'agentcat:custom'` (`index.ts:448-450`). The legacy brief describes a `taskId` field (`brief:430-436`); the code uses `sessionId`.

### 6.1 Rust port notes: custom events

- API sketch: `agentcat::publish_custom_event(target: CustomEventTarget, event: CustomEvent) -> Result<(), AgentCatError>`, where `CustomEventTarget` is either a tracked-server handle or a session id string. Return errors instead of throwing.
- Decide whether to keep the "argument `project_id` ignored for tracked servers" quirk. The cleaner API takes no project id when a tracked handle is given.
- Decide whether custom events get string redaction. TS skips it, which surprises customers who rely on `redactSensitiveInformation` for PII. Applying it is safer; if parity wins, document the gap.
- `CustomEvent.error`: accept a `serde_json::Value` (parity) and add a helper that builds `ErrorData` from a `&dyn std::error::Error`.

---

## 7. Behavioral guarantees from the tests

How the tests observe events: most in-process tests use `EventCapture` (`src/tests/test-utils.ts:36`). It records the event object at `eventQueue.add()`, lets the real pipeline mutate it in place, and stubs `sendEvent`. Those tests therefore assert the post-pipeline event, not the HTTP body. Only `eventQueue.test.ts` (mocked `apiClient.publishEvent`) and `src/tests/workers/` (a real outbound fetch) assert what goes on the wire. The Rust suite needs both levels: pipeline unit tests on the event, and wire tests against a mock HTTP server (`wiremock` or `httpmock`).

Weak tests (take behavior from the code, not from these): duration is never asserted (`src/tests/eventQueue.test.ts:143,159`); `destroy()` "no throw" wraps an async function in `not.toThrow` (`eventQueue.test.ts:448`); the SIGINT test only checks that `process.once` exists (`eventQueue.test.ts:671`); queue constants are tested only with `maxRetries=0` and `maxQueueSize=1`; `tracing-initialization.test.ts:28` passes an options object as `projectId`.

### 7.1 Queue, delivery, and shutdown

| Guarantee | Test |
|---|---|
| Missing tracking data: log exactly `Warning: Server tracking data not found. Event will not be published.`, publish nothing | `src/tests/eventQueue.test.ts:119` |
| `enableTracing: false` returns silently, no log, no session info built | `eventQueue.test.ts:140` |
| Missing `sessionId` → `""` on the event, explicit `null` on the request | `eventQueue.test.ts:189,195` |
| `inputTokens`/`outputTokens` reach the request unchanged (6, 4); absent → omitted | `eventQueue.test.ts:216-217,232-233` |
| One background task per event; it stays pending until the API call resolves | `eventQueue.test.ts:262,270-274` |
| Exhausted retries: the task still resolves (never rejects); log contains `Failed to deliver AgentCat event after retries` | `eventQueue.test.ts:295-299` |
| Telemetry-only events (no `projectId`) never register a background task | `eventQueue.test.ts:319` |
| Delivery settles without waiting for exporters | `eventQueue.test.ts:348-349` |
| Overflow drops the OLDEST event, resolves its task, logs exactly `Event queue full, dropping oldest event`; registration is synchronous in `add()` | `eventQueue.test.ts:380-384` |
| A throwing registrar never breaks `publishEvent`; log contains `Failed to register AgentCat event delivery as a background task` | `eventQueue.test.ts:399-411` |
| `getStats()` shape `{queueLength, activeRequests, isProcessing}` | `eventQueue.test.ts:424` |
| After `destroy()`, `add` logs exactly `Queue is shutting down, event dropped` and drops, permanently | `eventQueue.test.ts:466` |
| Success log has the event id and session but never payload values (`parameters`, `response`, `userIntent`) | `eventQueue.test.ts:525-532` |
| `redactEvent` from `track()` options is called exactly once with the raw event; the request carries its output | `eventQueue.test.ts:586-593` |
| Hook returns `null` → dropped, log exactly `Event dropped by redactEvent hook`; hook throws → dropped, log contains `Failed to redact event (event-level hook)` | `eventQueue.test.ts:609-611,633-635` |
| Order: `redactEvent` sees raw text, string redaction runs on its output (`"raw secret"` → `"raw [REDACTED] (reviewed)"`) | `eventQueue.test.ts:663-666` |
| `runSignalShutdown` runs `destroy` and `flush`, then re-raises the same signal | `src/tests/signal-shutdown.test.ts:15,18-24` |
| No re-raise when the customer has a listener for the signal | `signal-shutdown.test.ts:27,30` |
| Re-raise still happens when `destroy` rejects; never throws even if the re-raise throws | `signal-shutdown.test.ts:33,40,43,49` |
| The queue module loads without `process.once` or without `process` | `src/tests/edge-runtime-compatibility.test.ts:135,146` |
| `writeToLog` never throws (empty string, control chars, 100 calls) | `edge-runtime-compatibility.test.ts:163-176` |

### 7.2 Base URL

| Guarantee | Test |
|---|---|
| `configure(url)` rebuilds the client with `Configuration({basePath: url})` | `src/tests/api-base-url.test.ts:55` |
| Option used when set; `AGENTCAT_API_URL` alone used; `MCPCAT_API_URL` alone used | `api-base-url.test.ts:137,147,157` |
| `AGENTCAT_API_URL` beats `MCPCAT_API_URL`; option beats both, `configure` called exactly once | `api-base-url.test.ts:168,179,191-194,204-207` |
| Nothing set → `configure` never called; default `https://api.agentcat.com` stays | `api-base-url.test.ts:215` |

### 7.3 Workerd delivery (real fetch)

| Guarantee | Test |
|---|---|
| One tool call → exactly one outbound fetch, only to the `apiBaseUrl` origin (no diagnostics, no initialize or list events) | `src/tests/workers/event-delivery.test.ts:51` |
| The tool result returns while the ingest fetch is pending; `waitUntil` keeps it alive; `202` counts as success | `event-delivery.test.ts:56-63` |
| With 100 ms deferred `resolveSessionId` and `identify` hooks, the tool result returns before any ingest body is sent | `event-delivery.test.ts:112-113`, hooks at `src/tests/workers/worker.ts:51-57` |
| The later body contains `"agentcat_session_id_source":"hook"`, the identified user, and a `ses_` id derived from the hook string | `event-delivery.test.ts:117-125` |
| An inbound request header value (`x-agentcat-health-run`) appears in the real ingest body | `event-delivery.test.ts:180-183` |

### 7.4 Event content, hooks, tags, and properties

| Guarantee | Test |
|---|---|
| `parameters` is `{request, extra}`; v2 `extra.http.req` has `method`, `url` (string), and plain-object `headers`; v1 `extra.requestInfo.url` is a string, never `{}` | `src/tests/e2e-http/header-capture.test.ts:64-66,102-105`, `src/tests/v2/http-context-capture.test.ts:63-86` |
| The health sentinel header value appears somewhere in `parameters` after the full pipeline (fleet health harness `header_ok` check) | `header-capture.test.ts:68,106`, `http-context-capture.test.ts:83-85` |
| Stdio-shaped `extra` (no `http`) passes unchanged; a malformed `req` does not throw and still publishes | `http-context-capture.test.ts:88-117` |
| Customer tags appear exactly as returned, next to SDK tags (`agentcat_session_id_source`, and with agent tracking `agentcat_agent_id`, `agentcat_agent_id_source`) | `src/tests/event-tags-properties.test.ts:97-106` |
| A customer tag that collides with an SDK tag is overwritten by the SDK value | `event-tags-properties.test.ts:135-139` |
| `eventTags` throws or returns `null` → tool succeeds, customer tags `{}`, SDK tags still present | `event-tags-properties.test.ts:160,170-173,198-201` |
| Callback tags are validated (`invalid!` dropped) | `event-tags-properties.test.ts:228` |
| Properties are attached as returned (nested arrays and objects); throw → tool not blocked; `null` → omitted | `event-tags-properties.test.ts:259,284,309` |
| Custom events: tags exactly as passed (no SDK tags), validated; `properties: {}` omitted | `event-tags-properties.test.ts:357-358,377,393` |
| No `mcp:initialize` or `mcp:tools/list` event is ever published | `event-tags-properties.test.ts:478,505`, `src/tests/identify.test.ts:830-831` |
| String redaction does not change tags or properties | `event-tags-properties.test.ts:542-551` |
| `eventTags`/`eventProperties` receive `(request, extra)` with `params.name` and `params.arguments` | `event-tags-properties.test.ts:580-585,612-614` |
| `identify` runs on every call, no cache; result stamped on each event; no `agentcat:identify` event | `identify.test.ts:77,85-98,144-198` |
| `identify` is deferred: the result returns before a 100 ms hook finishes; identity lands later; `duration` < 95 ms covers only the handler | `identify.test.ts:662,676-682` |
| `identify` returns `null`, throws, or returns a bad shape → anonymous event, `identifyActorData` `{}` | `identify.test.ts:438-440,716,735-737,784-786` |
| Without `identify`, `sessionId` is a minted non-empty id | `identify.test.ts:492-493` |
| Three interleaved concurrent calls give three events, each with its own session id, agent id tag, and identity | `src/tests/concurrency.test.ts:76,81-83` |
| `track()` three times on one server still gives one event per call | `src/tests/tracing-initialization.test.ts:90-92` |
| Success event: `isError` undefined (not `false`); `userIntent` = `context` | `tracing-initialization.test.ts:134-143` |

Tag validation (`src/tests/validation.test.ts`): valid tags pass (`:22`); `{}` → `null` (`:26`); invalid key characters dropped with the key in the log (`:36-39`); key 33 chars dropped, 32 allowed (`:46-49`); value 201 chars dropped, 200 allowed (`:56-59`); `\n` in a value dropped (`:65-66`); non-string value dropped (`:72-75`); empty key dropped (`:81`); 60 entries → first 50 kept, log `Dropping 10` (`:90-94`); all invalid → `null` (`:99`); keys may contain `.`, `:`, `-`, space, and `$`, including a leading `$` (`:111`).

### 7.5 `publishCustomEvent`

| Guarantee | Test |
|---|---|
| Tracked server → `publishEvent(server, event)` with `sessionId` from `eventData`, `eventType: "agentcat:custom"`, `resourceName`, `parameters`, `userIntent` = `message`; log contains `Published custom event` | `src/tests/publishCustomEvent.test.ts:88-99` |
| Tracked server without `sessionId` → `sessionId: ""`, log contains `no sessionId provided` | `publishCustomEvent.test.ts:122-132` |
| `isError`, `error`, `duration: 1500` pass through as given | `publishCustomEvent.test.ts:143,220` |
| Untracked server rejects with `Server is not tracked`; `.server` is unwrapped first | `publishCustomEvent.test.ts:155,167` |
| String first argument is the session id, verbatim, sent with `eventQueue.add`; `eventData.sessionId` wins over it | `publishCustomEvent.test.ts:183,199` |
| `projectId` `""`, `null`, `undefined` → rejects `projectId is required` (checked first); first argument `123`/`null`/`undefined` → `First parameter must be either an MCP server object or a session ID string` | `publishCustomEvent.test.ts:236-262` |
| `timestamp` is taken at call time; empty `eventData` leaves all optional fields undefined | `publishCustomEvent.test.ts:295-329` |
| v2 server: same mapping; bad tag key dropped; `properties {}` → undefined | `src/tests/v2/pipeline.test.ts:139-219` |
| Tracked server applies `redactEvent` to custom events | `src/tests/redaction.test.ts:979-1008` |

Not covered by any test, confirmed in code: the `projectId` argument is ignored for tracked servers (`eventQueue.ts:390`; the unit test mocks `publishEvent`); string redaction never applies to custom events; `enableTracing: false` drops tracked-server custom events while `publishCustomEvent` still logs success (`src/index.ts:448`).

### 7.6 Redaction

| Guarantee | Test |
|---|---|
| Unprotected top-level strings go through the redactor (`userIntent`) | `src/tests/redaction.test.ts:28-39` |
| Cycles terminate and the clone keeps the cycle; shared references are redacted once and stay shared | `redaction.test.ts:41-71` |
| Protected top-level fields never reach the redactor (`sessionId`, `id`, `projectId`, `server`, `serverName`, `serverVersion`, `clientName`, `clientVersion`, `identifyActorGivenId`, `identifyActorName`, `resourceName`, `eventType`, `actorId`) | `redaction.test.ts:73-109` |
| Protection is top-level only: `parameters.nested.deeply.sessionId` IS redacted | `redaction.test.ts:281-299` |
| Nested strings and array elements are redacted; numbers untouched | `redaction.test.ts:111-176` |
| `null` kept, `undefined` key absent, `""` IS redacted; `Date` stays a `Date`; function properties dropped | `redaction.test.ts:178-227` |
| Legacy key `identifyData` protected recursively | `redaction.test.ts:229-259,316-361` |
| `error.message`, `error.code`, `error.stack` redacted; `eventId` redacted | `redaction.test.ts:261-279,383-431` |
| Redactor throws → `redactEvent` rejects (queue drops the event) | `redaction.test.ts:301-314` |
| Input not mutated | `redaction.test.ts:363-381` |
| Event hook: sync and async supported; `null`/`undefined` → `false`, event untouched | `redaction.test.ts:451-494` |
| Event hook: deleted fields stay deleted; `id`, `sessionId`, `projectId`, `eventType`, `timestamp` restored; hook never sees the function fields; same object identity kept; a throw propagates | `redaction.test.ts:496-583` |
| Integration: arguments redacted; identity fields and `sessionId` not redacted; `actorId` undefined on the internal event | `redaction.test.ts:603-823` |
| Integration: event hook can drop one tool's events only; sees raw text before string redaction | `redaction.test.ts:894-977` |
| v2: response text and `arguments.query` redacted; `sessionId`, `resourceName` untouched | `src/tests/v2/pipeline.test.ts:24-69` |

Not pinned by tests, confirmed by probe: `identifyActorData` IS redacted (`identifyActorData.email "a@b.c"` → redacted), as are `sdkLanguage`, `agentcatVersion`, `ipAddress`, `error.type`, frame `filename`/`function`, and content-block `type` discriminators. Because redaction runs before sanitization, a redactor that rewrites `"image"` produces `[unsupported content type "<rewritten>" redacted …]` (acknowledged at `src/tests/token-estimates.test.ts:156-157`).

### 7.7 Sanitization

| Guarantee | Test |
|---|---|
| image and audio → text markers; text kept; order and length preserved | `src/tests/sanitization.test.ts:26-52` |
| `resource` with `blob` → binary resource marker; with `text` → unchanged; `resource_link` unchanged; unknown type `video` → `[unsupported content type "video" redacted - not supported by AgentCat]` | `sanitization.test.ts:72-144` |
| Large base64 in `structuredContent` → binary data marker; siblings kept | `sanitization.test.ts:146-164` |
| Response without `content`, `null`, or `undefined` unchanged | `sanitization.test.ts:166-182` |
| Parameters: `"A"×11999 + "="` → marker; 12000-char non-base64 unchanged; nested and in-array base64 redacted; other types untouched | `sanitization.test.ts:202-288` |
| Gate: exactly 10240 chars IS redacted; 10239 is not | `sanitization.test.ts:290-307` |
| Input not mutated; identity fields preserved | `sanitization.test.ts:319-381` |
| E2E: image/audio blocks replaced in real tool results; base64 argument replaced, `filename` kept | `src/tests/e2e-sanitization.test.ts:12-302` |

Probe-only facts: `resource.blob: null` counts as a blob (`!== undefined`); the regex allows `\r`/`\n` anywhere and any number of trailing `=`; sanitization has no cycle guard (a cyclic `parameters` overflows the stack and the queue drops the event).

### 7.8 Truncation

| Guarantee | Test |
|---|---|
| String 33000 → 32771 (32768 + `...`); exactly 32768 unchanged | `src/tests/truncation.test.ts:11-22` |
| Markers: `[Function: myFunc]`, `[Function: <anonymous>]`, `[Symbol(test)]`, `[Symbol()]`, `[undefined]`, `[BigInt: 123]`, `[NaN]`, `[Infinity]`, `[-Infinity]`, ISO date, `[Invalid Date]` | `truncation.test.ts:26-64` |
| Depth 5 on a nested chain → `[Object]` at level 5; arrays → `[Array]`; depth 0 → `[Object]`/`[Array]`; primitives never collapsed | `truncation.test.ts:83-109` |
| Breadth 5: objects get 5 keys + `"..."` = `[MaxProperties ~]`; arrays get 5 items + `[MaxProperties ~]` | `truncation.test.ts:113-135` |
| Cycles → `[Circular ~]`; a shared non-cyclic reference is expanded in both places | `truncation.test.ts:139-169` |
| `userIntent` 3000 → 2051; `resourceName` 300 → 259; server/client name and version → 259 | `truncation.test.ts:198-248` |
| `error.message` 3000 → 2051; 80 frames → 50 (`[24]=file24`, `[25]=file55`, `[49]=file79`); 50 frames unchanged | `truncation.test.ts:252-304` |
| Response text block 40000 → 32771 | `truncation.test.ts:308-321` |
| Large events end at ≤ 102400 UTF-8 bytes of `JSON.stringify`; last resort keeps `timestamp` as the same `Date` | `truncation.test.ts:325-403` |
| Pass-through fields: `id`, `sessionId`, `projectId`, `eventType`, `timestamp`, `duration`, `sdkLanguage`, `agentcatVersion`, `ipAddress`, `isError` | `truncation.test.ts:462-487` |
| E2E: 50000-char text → ≤ 32771; 80000 + 40000 argument strings → event ≤ 102400 bytes | `src/tests/e2e-truncation.test.ts:12-288` |

Probe-only facts: the 102400-byte budget is best effort. A `properties` object of 100 × 100 strings of 100 chars passed through at 1,089,886 bytes. A function or Symbol in `properties` makes `structuredClone` throw on the last-resort path, and a BigInt or cycle in `properties` fails the first `JSON.stringify`. Either way the event is dropped (`truncation.ts:239,318`). Depth 1 turns tool-call `parameters` into `{"request":"[Object]","extra":"[Object]"}`.

### 7.9 Token estimates

Golden vectors are in section 5.6 (`src/tests/tokenEstimate.test.ts:14-150`). Extra unit vectors: `utf8ByteLength` `""`→0, `"hello"`→5, `"café"`→5, `"你好"`→6, `"😀"`→4, lone `"\ud83d"`→3 (`tokenEstimate.test.ts:31-41`); circular input and a throwing `toJSON` → omitted (`:61-75`); `[image]` + `structuredContent` → 0 (`:81-123`); `[{type:"text",text:42}, null, "str"]` → 0 (`:134-140`); a throwing `content` getter → omitted (`:142-150`). An integer-only formula that matches every vector: `min((2*bytes + 6) / 7, i32::MAX)`.

| Integration guarantee | Test |
|---|---|
| `{"text":"hi there"}` (19 B) → `inputTokens` 6; `"hello world"` → `outputTokens` 4; `structuredContent` ignored when `content` is non-empty | `src/tests/token-estimates.test.ts:35-66` |
| `content: []` + `structuredContent {"result":"ok"}` → 5 | `token-estimates.test.ts:68-98` |
| `isError` result `"boom!"` → 2; thrown `Error("no")` converted by McpServer → 1 | `token-estimates.test.ts:100-145` |
| Redaction does not change counts (`inputTokens` 10, `outputTokens` 7 computed from the raw bytes) | `token-estimates.test.ts:147-184` |
| Truncation does not change counts (40000 chars → 11429) | `token-estimates.test.ts:186-210` |
| A throwing estimator never breaks the tool result | `token-estimates.test.ts:212-259` |

### 7.10 Exceptions and error capture

| Guarantee | Test |
|---|---|
| `Error`, `TypeError`, `ReferenceError`, and a subclass with `this.name = "CustomError"` → `type` is that name; `stack` and non-empty `frames` present | `src/tests/exceptions.test.ts:6-46` |
| `platform` is always `"javascript"` | `exceptions.test.ts:48-59` |
| Frame `fn (file:42:15)` → lineno 42, colno 15; bare location → function `<anonymous>`; `async fn` keeps the `async ` prefix; `Array.map (native)` → filename and abs_path `native`, in_app false | `exceptions.test.ts:113-161,793-802` |
| `/node_modules/` and `node:` frames are not in-app; 100 frames → at most 50 | `exceptions.test.ts:87-111,163-176` |
| `context_line` present on some real in-app frame, never on library frames; a missing file leaves it undefined without throwing | `exceptions.test.ts:178-224` |
| Cause chain: outer to inner order, at most 10, string cause → `{message, type: undefined}`, circular cause stops, each entry has `stack` and `frames` | `exceptions.test.ts:228-301` |
| Non-Error values: string kept; `42` → `"42"`; `false` → `"false"`; `null` → `"null"`; `undefined` → `"undefined"`; object → JSON; circular object → some string | `exceptions.test.ts:305-358` |
| Deleted stack → no frames; empty message → `""`; whitespace stack → `frames: []`; malformed lines never throw | `exceptions.test.ts:362-404` |
| Filename normalization table (36 inputs, see below) | `exceptions.test.ts:409-843` |
| Integration (v1 1.30): a thrown `Error` → client gets `isError: true`; event has the original `type`, `stack`, frames; a cause gives `chained_errors[0]`; `TypeError` from `null.property` keeps `type: "TypeError"` through the inner-tap stash | `src/tests/error-capture.test.ts:51-197` |
| `throw "string"` → event `type` undefined, no stack, message contains the string | `error-capture.test.ts:200-249` |
| The error text reaches the client at `content[1]`; `content[0]` is the mint-back block the SDK puts first | `error-capture.test.ts:301-332` |
| Success → `isError` and `error` undefined (not `false`) | `error-capture.test.ts:413-418` |
| Invalid enum, unknown tool, missing required parameter → `isError` event with a message (type `McpError`, `Error`, or undefined depending on SDK version) | `error-capture.test.ts:424-621` |
| A stashed Error beats the result text; a thrown handler error rejects the wrapper and publishes `isError` with `duration >= 0` | `src/tests/engine-callwrap.test.ts:175-188,247-266` |
| Tracing setup failure → original handler with the unstripped request, no event, log contains `tracing failed for tool echo, falling back to original handler` | `engine-callwrap.test.ts:190-223` |
| Post-handler failure → raw customer result returned, no event, log contains `post-handler` | `engine-callwrap.test.ts:637-653` |
| Error-path events still carry identity that resolves after the throw | `engine-callwrap.test.ts:503-522` |
| Low-level `isError` result → `error.message` from text, no mint-back in `response` or `error`; low-level throw → client rejects, event has a stack | `src/tests/v2/lowlevel-handles.test.ts:149-189` |

Filename normalization vectors (input → `filename`; `abs_path` always keeps the input). Copy them as Rust test vectors only if the Rust SDK keeps JS-style path handling for non-Rust paths. They mainly document the grouping intent.

| Input | `filename` |
|---|---|
| `/Users/john/project/src/index.ts` | `src/index.ts` |
| `/home/ubuntu/app/src/server.ts` | `src/server.ts` |
| `C:\Users\Jane\projects\myapp\src\index.ts` | `src/index.ts` |
| `/Users/john/project/node_modules/express/lib/router.js` | `node_modules/express/lib/router.js` |
| `/app/node_modules/pkg1/node_modules/pkg2/index.js` | `node_modules/pkg2/index.js` |
| `C:\projects\app\node_modules\lodash\index.js` | `node_modules/lodash/index.js` |
| `/var/www/myapp/src/api/users.ts` | `src/api/users.ts` |
| `/var/task/src/handler.ts` | `src/handler.ts` |
| `/opt/myservice/lib/main.ts` | `lib/main.ts` |
| `/Users/john/src/project/src/index.ts` | `src/index.ts` (last `/src/`) |
| `node:internal/modules/cjs/loader` | `node:internal` |
| `node:fs/promises` | `node:fs` |
| `file:///C:/Users/Jane/project/src/index.ts` | `src/index.ts` |
| `/index.ts` | `index.ts` |

Probe-only facts: a Windows forward-slash path such as `C:/projects/app/src/index.ts` is returned verbatim (`exceptions.ts:636` only matches `C:\`); `at fn /file:20:5` parses as filename `fn /file` with in_app true; a home path without a marker keeps the tilde (`/home/bob/proj/thing.ts` → `~/proj/thing.ts`); a thrown Symbol or function gives `message: undefined`.

Error classification by MCP SDK version (from the test agent's reading of the installed SDKs; v1 `@modelcontextprotocol/sdk` 1.30.0, v2 `@modelcontextprotocol/server` 2.0.0):

| Case | v1 1.30 | v2 2.0.0 |
|---|---|---|
| Handler throws `Error` | SDK converts to an `isError` result; event uses the stashed Error (type, stack) and records `response` + `outputTokens` | same |
| Unknown or disabled tool | `isError` result, message `MCP error -32602: Tool <name> not found`, no type or stack, client does not throw | `ProtocolError` thrown outside the SDK try: catch path, `type: "ProtocolError"`, stack, no `response`, rethrown as JSON-RPC error (untested) |
| Input validation failure | `isError` result, message `MCP error -32602: Input validation error: …` | `isError` result |

`src/tests/error-capture.test.ts:467,532,594` place `expect.fail` inside a `try` whose `catch` swallows it, so those three tests pass whether or not the client throws.

### 7.11 PostHog exporter

No test covers the OTLP, Datadog, or Sentry exporters, `trace-context`, or `TelemetryManager`. `EventCapture` stubs `sendEvent` (`src/tests/test-utils.ts:60`), so no integration test reaches telemetry. The Rust port must write these tests from the implementation (section 4).

| Guarantee | Test |
|---|---|
| POST `https://us.i.posthog.com/batch`, `Content-Type: application/json`, body `{api_key, batch}` | `src/tests/posthog-exporter.test.ts:56-65` |
| Capture event: `event: "mcp_tool_call"`, `type: "capture"`, `distinct_id` = session id, ISO `timestamp`; properties `$session_id` (UUIDv7), `tool_name`, `resource_name`, `duration_ms`, server/client fields, `project_id`, `is_error: false` | `posthog-exporter.test.ts:67-83` |
| Custom host respected; one trailing `/` stripped | `posthog-exporter.test.ts:86-110` |
| `distinct_id` = actor id when set, else session id | `posthog-exporter.test.ts:112-134` |
| Sessionless: no `$session_id` on any entry, no `$ai_session_id`, `$ai_trace_id` still a valid UUIDv7 | `posthog-exporter.test.ts:136-163` |
| Error → batch of 2; `$exception` with message, type, raw stacktrace, `$exception_source: "backend"`, `$session_id`, resource/tool/server names | `posthog-exporter.test.ts:165-208` |
| Fetch rejection or `400` → `export` resolves, never throws | `posthog-exporter.test.ts:223-248` |
| `$set = {name, ...identifyActorData}`; absent without identity | `posthog-exporter.test.ts:250-284` |
| `parameters`/`response` pass through as objects or strings; `tool_name` only for `mcp:tools/call` | `posthog-exporter.test.ts:286-354` |
| Event name mapping incl. fallback `mcp:custom/type` → `mcp_custom_type` | `posthog-exporter.test.ts:356-380` |
| Tags and properties spread at top level; `source: "agentcat"` always present; `userIntent` → `user_intent` | `posthog-exporter.test.ts:382-448` |
| `$ai_span` fields (`$ai_session_id = "agentcat_<sid>"`, `$ai_latency` 0.25 for 250 ms, input/output state) | `posthog-exporter.test.ts:450-495` |
| Determinism: same session → same `$ai_trace_id`; same event → same `$ai_span_id` | `posthog-exporter.test.ts:497-547` |
| `$ai_span` only with `enableAITracing` and only for `mcp:tools/call` | `posthog-exporter.test.ts:549-604` |
| Customer tags can override `$ai_trace_id` and `$ai_span_name` | `posthog-exporter.test.ts:639-660` |
| Error + AI tracing → order `[mcp_tool_call, $exception, $ai_span]`; `$ai_error` = whole `error` object | `posthog-exporter.test.ts:662-695` |
| `toUUIDv7`: valid v7, deterministic, distinct per input; KSUID path puts the KSUID time in the first 12 hex chars; version nibble `7`, variant `8..b`; non-KSUID input deterministic across milliseconds | `src/tests/posthog-uuidv7.test.ts:10-108` |

The tests assert no literal UUID values. Golden vectors computed by running the real `toUUIDv7` and `traceContext` (use them as Rust test vectors):

| Input | `toUUIDv7` | Path |
|---|---|---|
| `ses_2yXlSmKnrVgSWKGxWkkGlLaGu7v` | `01977375-f600-7355-b83a-0dea9024f86c` | KSUID (2025-06-15T12:00:00Z) |
| `evt_2rf1E3grYK2Vg9xFt9cIzKrfCnh` | `01946967-d500-7a34-a23b-3266fc83bf19` | KSUID (2025-01-15T10:00:00Z) |
| `ses_000000000000000000000000000` | `0145f680-b000-72dd-9660-4a400f542404` | KSUID (epoch 1400000000000 ms) |
| `ses_session45600000000000000000` | `6c350afa-fa75-7081-b0c6-8ad3272daa6c` | fallback (base62 overflow) |
| `evt_test123` | `16abe8cf-1d05-7811-87f2-fd2f9686af87` | fallback |
| `ses_invalid_ksuid_string` | `8a9bdc3e-a187-7187-bbe0-cee6b92a5751` | fallback |
| `my-customer-task-id` | `634902a7-d0c7-7220-91c6-0fa56fad8828` | fallback |
| `task-a` | `2084b1c4-8b52-77bb-bbed-5fd87cbc8c1b` | fallback |
| `""` | `f4c8996f-b924-73b0-8442-98fc1c149afb` | fallback |

| Input | OTLP `traceId` | OTLP `spanId` | Datadog `trace_id` | Datadog `span_id` |
|---|---|---|---|---|
| `ses_session45600000000000000000` | `2081b0c68ad3272daa6c6c350afafa75` | `2081b0c68ad3272d` | `12280309259005786741` | `2342347647988475693` |
| `evt_test123` | `081147f2fd2f9686af8716abe8cf1d05` | `081147f2fd2f9686` | `12648103006094433541` | `581324935862916742` |

KSUID parsing quirk: the vendored `KSUID.parse` accepts any 27-character string without a charset check (`src/thirdparty/ksuid/index.js:156-171`, digit = code−48 if < 58, code−55 if < 91, else code−61). For example `000000000000-customer-id-12` decodes to a 2014 timestamp instead of taking the fallback. Values above 2^160 throw and fall back. A Rust port must copy this decode to match on garbage input, or accept the divergence for non-`ses_` customer ids (verbatim custom-event session ids).

---

## 8. Defects and quirks found (decide per item: copy or fix)

| # | Behavior | Evidence | Recommendation for Rust |
|---|---|---|---|
| 1 | Telemetry exporters fire again on every AgentCat retry (up to 4 times per event) | `eventQueue.ts:213-221,240` | fix: export once |
| 2 | No HTTP timeout on the AgentCat client or exporters | `api@1.0.3/src/runtime.ts:160-165`; exporters use bare `fetch` | fix: explicit timeouts |
| 3 | Every failure is retried, including `400`/`401`/`415`; a 2xx with a non-JSON body counts as failure | `eventQueue.ts:233-243`, `api@1.0.3/src/runtime.ts:402-408` | fix: retry only network/429/5xx; any 2xx is success |
| 4 | Wire body duplicates fields in camelCase | `api@1.0.3/src/models/PublishEventRequest.ts:279-310` | fix: snake_case only |
| 5 | `agentcat:custom` is missing from the OpenAPI enum | `schema:129-147` | patch the schema or use `String` |
| 6 | `identifyActorData` is redacted because the protected list names `identifyData` | `redaction.ts:24`, `eventQueue.ts:192` | decide; recommend protect |
| 7 | Custom events never get string redaction | `callWrap.ts:244`, `eventQueue.ts:422` | decide; recommend apply |
| 8 | `publishCustomEvent` ignores its `projectId` argument for tracked servers; telemetry-only servers silently never deliver custom events to AgentCat | `eventQueue.ts:390,224`, `src/index.ts:270` | fix in API design |
| 9 | `enableTracing: false` drops tracked-server custom events while logging success | `eventQueue.ts:365-367`, `src/index.ts:448` | decide |
| 10 | `duration \|\| fallback` treats 0 ms as missing | `eventQueue.ts:379-383` | fix |
| 11 | Base URL is last-wins across `track()` calls; telemetry is first-wins | `src/index.ts:234,254` | make both first-wins |
| 12 | Truncated `resource_name`/`server_name`/`client_name` are 259 chars, versions 259 chars, but the server columns are 255 and 50; overflow sends the event to the DLQ | `truncation.ts:12-13,185-192`, section 2.5 | fix: cap to server limits |
| 13 | Verbatim custom-event session strings > 31 chars and actor ids > 255 chars are lost server-side | section 2.5 | validate client-side |
| 14 | SDK tag regex allows a space in keys and the `agentcat:` prefix; the server drops both | `validation.ts:3`, section 2.5 | use the server rules |
| 15 | The 102400-byte event budget is not a hard cap | section 5.5 | decide; a hard cap is safer under the 1 MiB SQS limit |
| 16 | Sentry transactions are shifted earlier by one duration; error `parent_span_id` equals its own span | `sentry.ts:336-342,452,481` | fix |
| 17 | Sentry, Datadog, PostHog error-type and message details (always `ToolCallError` for result errors; Datadog `error.message` is the whole JSON) | `sentry.ts:431,442`, `datadog.ts:179-186` | decide |
| 18 | PostHog maps `agentcat:custom` to `mcp_agentcat:custom` | `posthog.ts:339-342` | keep for parity (dashboards may depend on it) |
| 19 | Redaction runs before sanitization, so content `type` strings reach the customer redactor | `eventQueue.ts:130-146` | skip discriminators |
| 20 | Inbound `Authorization`/`Cookie` headers are published in `parameters.extra` | `extra-projection.ts:38-48` | decide; recommend a default denylist |
| 21 | `destroy()` permanently disables the queue; after `beforeExit` every later event is dropped | `eventQueue.ts:263-265,327-331` | design explicit shutdown |
| 22 | Vacuous tests: `expect.fail` inside a swallowing `try` | `src/tests/error-capture.test.ts:467,532,594` | write real assertions |
| 23 | `sdk-generator/versions.env` says 1.0.2; local `node_modules` has 1.0.1; package pins 1.0.3 | section 2.8 | pin the Rust client to the schema at a known server commit |

---

## 9. Top porting risks

1. Silent server-side loss. The API returns `202` for nearly everything and drops or dead-letters bad events later (section 2.5). The Rust SDK must validate required fields, `session_id` (`null`, never `""`), column lengths, `ip_address`, and tag rules before sending, and wire tests must assert the exact body.
2. Generated client mismatch. `public-schema.json` is YAML, lacks `agentcat:custom`, and marks `session_id` nullable. A generated Rust enum or double-option type can make custom events unserializable or turn `null` into an omitted key (section 2.7). A small hand-written `serde` request type is lower risk than the generated model.
3. Shutdown and flush. JS relies on `beforeExit`, signal re-raise, and `waitUntil`. Rust has no async `Drop`; a tokio signal listener disables default termination for good; an rmcp stdio server's runtime drops background tasks when `main` returns (section 3.5). Ship an explicit `shutdown(timeout)` API, a bounded-blocking guard, and docs for stdio, HTTP, Lambda, and workers-rs.
4. Error capture and grouping. Rust has no `Error.stack` and no thrown values. Errors arrive as `Err` values (no backtrace unless `anyhow`/`eyre` captured one), `isError` results, or panics (needs `catch_unwind` plus a chained panic hook). Server grouping uses `type` plus `module|function` for non-JS platforms, and its default redaction mangles `::` and `.rs` (section 5.7). Send short types, `module` on every frame, stable symbol names, and strict frame types.
5. Text semantics. JS limits count UTF-16 code units, JSON key order is insertion order, and `JSON.stringify` drops `undefined`. Rust needs `encode_utf16().count()` with char-boundary cuts, `serde_json` `preserve_order`, and `skip_serializing_if` (except `session_id`). Byte-exact token vectors are in section 5.6.
6. rmcp context mapping. `parameters.extra`, client info, protocol version, and the HTTP header capture that the fleet health harness checks (`x-agentcat-health-run`) depend on what the pinned rmcp version exposes in `RequestContext` (`extensions`, `meta`, `peer_info()`). Verify against the exact rmcp version before freezing the event shape.
7. Parity versus fixes. Sections 8 and 5.8 list TS behaviors that are bugs (duplicate exports, no timeouts, retry on 4xx, Sentry timing, 259-char caps, unprotected `identifyActorData`, unredacted custom events). Decide per item, record the decision, and pin it with a test, so the TS and Rust SDKs do not drift silently.
8. Concurrency model. TS state (`processing`, `activeRequests`, first-wins singletons) relies on a single-threaded event loop. Rust needs a `OnceLock` singleton, a semaphore of 5, and a mutex-guarded drop-oldest deque, plus hooks run with `tokio::time::timeout(30 s)` and `catch_unwind`, so a slow or panicking customer hook cannot block the tool call or the queue.
